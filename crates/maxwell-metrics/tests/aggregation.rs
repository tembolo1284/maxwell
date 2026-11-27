//! Integration tests for deterministic metrics aggregation.

use std::thread;

use maxwell_metrics::histogram::LATENCY_BOUNDS_NS;

use maxwell_metrics::{Counter, Histogram, MetricsError};

const WORKERS: usize = 8;
const OBSERVATIONS_PER_WORKER: u64 = 10_000;

#[derive(Debug)]
struct WorkerMetrics {
    messages: Counter,
    latency: Histogram<24>,
}

impl WorkerMetrics {
    fn new() -> Self {
        Self {
            messages: Counter::new(),
            latency: Histogram::default(),
        }
    }

    fn record(&mut self, latency_ns: u64) {
        self.messages.increment();

        assert!(self.latency.record(latency_ns));
    }

    fn merge(&mut self, other: &Self) {
        assert!(self.latency.merge(&other.latency).unwrap());

        self.messages.merge(&other.messages);
    }
}

fn observation(worker: usize, index: u64) -> u64 {
    let worker = worker as u64;

    50 + ((worker * 997 + index * 31) % 100_000)
}

fn run_worker(worker: usize) -> WorkerMetrics {
    let mut metrics = WorkerMetrics::new();

    for index in 0..OBSERVATIONS_PER_WORKER {
        metrics.record(observation(worker, index));
    }

    metrics
}

#[test]
fn aggregates_single_owner_worker_metrics() {
    let mut combined = WorkerMetrics::new();

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(WORKERS);

        for worker in 0..WORKERS {
            handles.push(scope.spawn(move || run_worker(worker)));
        }

        for handle in handles {
            let metrics = handle.join().unwrap();

            combined.merge(&metrics);
        }
    });

    let expected = WORKERS as u64 * OBSERVATIONS_PER_WORKER;

    assert_eq!(combined.messages.value(), expected);
    assert_eq!(combined.latency.count(), expected);

    let bucket_count: u64 = combined.latency.buckets().iter().copied().sum();

    assert_eq!(bucket_count + combined.latency.overflow(), expected);
}

#[test]
fn parallel_aggregation_matches_serial_recording() {
    let mut serial = WorkerMetrics::new();

    for worker in 0..WORKERS {
        for index in 0..OBSERVATIONS_PER_WORKER {
            serial.record(observation(worker, index));
        }
    }

    let mut parallel = WorkerMetrics::new();

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(WORKERS);

        for worker in 0..WORKERS {
            handles.push(scope.spawn(move || run_worker(worker)));
        }

        for handle in handles {
            parallel.merge(&handle.join().unwrap());
        }
    });

    assert_eq!(serial.messages, parallel.messages);

    assert_eq!(serial.latency.snapshot(), parallel.latency.snapshot());
}

#[test]
fn aggregation_order_does_not_change_histogram() {
    let workers: Vec<_> = (0..WORKERS).map(run_worker).collect();

    let mut forward = WorkerMetrics::new();
    let mut reverse = WorkerMetrics::new();

    for worker in &workers {
        forward.merge(worker);
    }

    for worker in workers.iter().rev() {
        reverse.merge(worker);
    }

    assert_eq!(forward.messages, reverse.messages);
    assert_eq!(forward.latency, reverse.latency);
}

#[test]
fn empty_histogram_is_merge_identity() {
    let mut histogram = Histogram::default();
    let empty = Histogram::default();

    histogram.record(100);
    histogram.record(200);

    let before = histogram;

    assert_eq!(histogram.merge(&empty), Ok(true));
    assert_eq!(histogram, before);
}

#[test]
fn merging_into_empty_histogram_preserves_observations() {
    let mut first = Histogram::default();
    let mut second = Histogram::default();

    for value in [100, 200, 300, 400, 500] {
        second.record(value);
    }

    assert_eq!(first.merge(&second), Ok(true));
    assert_eq!(first, second);
}

#[test]
fn incompatible_histograms_cannot_be_aggregated() {
    let mut first = Histogram::default();

    let mut bounds = LATENCY_BOUNDS_NS;

    bounds[0] = 25;

    let second = Histogram::new(bounds).unwrap();

    first.record(100);

    let before = first;

    assert_eq!(
        first.merge(&second),
        Err(MetricsError::IncompatibleHistogramBounds)
    );

    assert_eq!(first, before);
}

#[test]
fn merged_percentiles_match_serial_percentiles() {
    let mut first = Histogram::new([100, 200, 500, 1000]).unwrap();
    let mut second = Histogram::new([100, 200, 500, 1000]).unwrap();
    let mut serial = Histogram::new([100, 200, 500, 1000]).unwrap();

    for index in 0u64..10_000 {
        let value = index % 1000;

        serial.record(value);

        if index % 2 == 0 {
            first.record(value);
        } else {
            second.record(value);
        }
    }

    assert_eq!(first.merge(&second), Ok(true));

    let merged = first.snapshot();
    let expected = serial.snapshot();

    for percentile in [0, 5_000, 9_000, 9_500, 9_900, 9_990, 9_999, 10_000] {
        assert_eq!(
            merged.percentile(percentile),
            expected.percentile(percentile)
        );
    }
}
