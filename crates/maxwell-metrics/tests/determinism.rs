//! Deterministic instrumentation and exposition tests.

use maxwell_metrics::histogram::LATENCY_BOUNDS_NS;

use maxwell_metrics::{MetricSnapshot, MetricsRegistry, write_prometheus};

const EVENTS: u64 = 100_000;

#[derive(Debug, PartialEq, Eq)]
struct ReplayResult {
    snapshots: Vec<MetricSnapshot<24>>,
    exposition: Vec<u8>,
}

fn next_random(state: &mut u64) -> u64 {
    let mut value = *state;

    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;

    *state = value;

    value
}

fn replay(seed: u64) -> ReplayResult {
    let mut registry = MetricsRegistry::<24>::new(8).unwrap();

    let received = registry.register_counter("maxwell_received_total").unwrap();

    let processed = registry
        .register_counter("maxwell_processed_total")
        .unwrap();

    let rejected = registry.register_counter("maxwell_rejected_total").unwrap();

    let gaps = registry
        .register_counter("maxwell_sequence_gaps_total")
        .unwrap();

    let queue_depth = registry.register_gauge("maxwell_queue_depth").unwrap();

    let latency = registry
        .register_histogram("maxwell_latency_nanoseconds", LATENCY_BOUNDS_NS)
        .unwrap();

    let mut random_state = seed;

    for _ in 0..EVENTS {
        let random = next_random(&mut random_state);

        registry.increment(received).unwrap();

        if random.is_multiple_of(100) {
            registry.increment(rejected).unwrap();
        } else {
            registry.increment(processed).unwrap();

            let observation = 50 + random % 100_000;

            assert!(registry.record(latency, observation).unwrap());
        }

        if random.is_multiple_of(1000) {
            registry.increment(gaps).unwrap();
        }

        registry
            .set_gauge(queue_depth, (random % 256) as i64)
            .unwrap();
    }

    let snapshots = registry.snapshots().collect();

    let mut exposition = Vec::new();

    write_prometheus(&mut exposition, registry.snapshots()).unwrap();

    ReplayResult {
        snapshots,
        exposition,
    }
}

#[test]
fn identical_inputs_produce_identical_snapshots() {
    let first = replay(0x1234_5678_9ABC_DEF0);
    let second = replay(0x1234_5678_9ABC_DEF0);

    assert_eq!(first.snapshots, second.snapshots);
}

#[test]
fn identical_inputs_produce_identical_exposition() {
    let first = replay(12345);
    let second = replay(12345);

    assert_eq!(first.exposition, second.exposition);
}

#[test]
fn different_inputs_produce_different_metrics() {
    let first = replay(12345);
    let second = replay(67890);

    assert_ne!(first.snapshots, second.snapshots);
}

#[test]
fn complete_replay_is_deterministic() {
    let first = replay(98765);
    let second = replay(98765);

    assert_eq!(first, second);
}

#[test]
fn replay_counters_reconcile() {
    let result = replay(12345);

    let counter_value = |name: &str| -> u64 {
        result
            .snapshots
            .iter()
            .find_map(|snapshot| match snapshot {
                MetricSnapshot::Counter {
                    name: metric_name,
                    value,
                } if *metric_name == name => Some(*value),

                _ => None,
            })
            .unwrap()
    };

    let received = counter_value("maxwell_received_total");
    let processed = counter_value("maxwell_processed_total");
    let rejected = counter_value("maxwell_rejected_total");

    assert_eq!(received, EVENTS);
    assert_eq!(processed + rejected, received);

    let histogram = result
        .snapshots
        .iter()
        .find_map(|snapshot| match snapshot {
            MetricSnapshot::Histogram { value, .. } => Some(value),

            _ => None,
        })
        .unwrap();

    assert_eq!(histogram.count, processed);

    let finite_count: u64 = histogram.buckets.iter().copied().sum();

    assert_eq!(finite_count + histogram.overflow, histogram.count);
}

#[test]
fn replay_percentiles_are_monotonic() {
    let result = replay(12345);

    let histogram = result
        .snapshots
        .iter()
        .find_map(|snapshot| match snapshot {
            MetricSnapshot::Histogram { value, .. } => Some(value),

            _ => None,
        })
        .unwrap();

    let mut previous = 0u64;

    for percentile in 0..=10_000u16 {
        let value = histogram.percentile(percentile).unwrap().unwrap();

        assert!(value >= previous);

        previous = value;
    }
}

#[test]
fn repeated_reset_and_recording_preserves_configuration() {
    let mut registry = MetricsRegistry::<24>::new(2).unwrap();

    let counter = registry.register_counter("messages_total").unwrap();

    let histogram = registry
        .register_histogram("latency", LATENCY_BOUNDS_NS)
        .unwrap();

    for _ in 0..1000 {
        for value in 0..1000u64 {
            registry.increment(counter).unwrap();
            registry.record(histogram, value).unwrap();
        }

        assert_eq!(registry.counter(counter).unwrap().value(), 1000);

        assert_eq!(registry.histogram(histogram).unwrap().count(), 1000);

        registry.reset_all();

        assert_eq!(registry.len(), 2);
        assert_eq!(registry.remaining_capacity(), 0);

        assert_eq!(
            registry.histogram(histogram).unwrap().bounds(),
            &LATENCY_BOUNDS_NS
        );
    }
}
