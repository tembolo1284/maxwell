//! Integration tests for application instrumentation.

use maxwell_metrics::histogram::LATENCY_BOUNDS_NS;

use maxwell_metrics::{MetricId, MetricSnapshot, MetricsError, MetricsRegistry, write_prometheus};

type Registry = MetricsRegistry<24>;

#[derive(Debug, Clone, Copy)]
struct ServerMetricIds {
    messages_received: MetricId,
    messages_processed: MetricId,
    messages_rejected: MetricId,
    sequence_gaps: MetricId,
    recovery_events: MetricId,
    active_sessions: MetricId,
    queue_depth: MetricId,
    processing_latency: MetricId,
}

fn setup() -> (Registry, ServerMetricIds) {
    let mut registry = Registry::new(16).unwrap();

    let ids = ServerMetricIds {
        messages_received: registry
            .register_counter("maxwell_messages_received_total")
            .unwrap(),

        messages_processed: registry
            .register_counter("maxwell_messages_processed_total")
            .unwrap(),

        messages_rejected: registry
            .register_counter("maxwell_messages_rejected_total")
            .unwrap(),

        sequence_gaps: registry
            .register_counter("maxwell_sequence_gaps_total")
            .unwrap(),

        recovery_events: registry
            .register_counter("maxwell_recovery_events_total")
            .unwrap(),

        active_sessions: registry.register_gauge("maxwell_active_sessions").unwrap(),

        queue_depth: registry.register_gauge("maxwell_queue_depth").unwrap(),

        processing_latency: registry
            .register_histogram("maxwell_processing_latency_nanoseconds", LATENCY_BOUNDS_NS)
            .unwrap(),
    };

    (registry, ids)
}

#[test]
fn instruments_market_data_processing() {
    let (mut registry, ids) = setup();

    registry.set_gauge(ids.active_sessions, 4).unwrap();

    for sequence in 1u64..=10_000 {
        registry.increment(ids.messages_received).unwrap();

        if sequence % 100 == 0 {
            registry.increment(ids.messages_rejected).unwrap();
        } else {
            registry.increment(ids.messages_processed).unwrap();

            let latency = 100 + sequence % 900;

            assert!(registry.record(ids.processing_latency, latency).unwrap());
        }

        registry
            .set_gauge(ids.queue_depth, (sequence % 64) as i64)
            .unwrap();
    }

    let received = registry.counter(ids.messages_received).unwrap().value();

    let processed = registry.counter(ids.messages_processed).unwrap().value();

    let rejected = registry.counter(ids.messages_rejected).unwrap().value();

    assert_eq!(received, 10_000);
    assert_eq!(processed, 9_900);
    assert_eq!(rejected, 100);
    assert_eq!(received, processed + rejected);

    assert_eq!(registry.gauge(ids.active_sessions).unwrap().value(), 4);

    let latency = registry
        .histogram(ids.processing_latency)
        .unwrap()
        .snapshot();

    assert_eq!(latency.count, processed);
    assert!(latency.min.unwrap() >= 100);
    assert!(latency.max.unwrap() < 1000);
    assert!(latency.p99().unwrap() <= 1000);
}

#[test]
fn instruments_recovery_events() {
    let (mut registry, ids) = setup();

    for _ in 0..10 {
        registry.increment(ids.sequence_gaps).unwrap();
        registry.increment(ids.recovery_events).unwrap();
    }

    assert_eq!(registry.counter(ids.sequence_gaps).unwrap().value(), 10);

    assert_eq!(registry.counter(ids.recovery_events).unwrap().value(), 10);
}

#[test]
fn registry_snapshots_are_independent_of_future_updates() {
    let (mut registry, ids) = setup();

    registry.add(ids.messages_received, 100).unwrap();

    let snapshot = registry.snapshot(ids.messages_received).unwrap();

    registry.add(ids.messages_received, 200).unwrap();

    assert_eq!(
        snapshot,
        MetricSnapshot::Counter {
            name: "maxwell_messages_received_total",
            value: 100,
        }
    );

    assert_eq!(
        registry.counter(ids.messages_received).unwrap().value(),
        300
    );
}

#[test]
fn rejects_incorrect_metric_operations() {
    let (mut registry, ids) = setup();

    assert_eq!(
        registry.increment(ids.queue_depth),
        Err(MetricsError::MetricTypeMismatch)
    );

    assert_eq!(
        registry.set_gauge(ids.messages_received, 100),
        Err(MetricsError::MetricTypeMismatch)
    );

    assert_eq!(
        registry.record(ids.active_sessions, 100),
        Err(MetricsError::MetricTypeMismatch)
    );
}

#[test]
fn exports_complete_server_metrics() {
    let (mut registry, ids) = setup();

    registry.add(ids.messages_received, 1000).unwrap();
    registry.add(ids.messages_processed, 990).unwrap();
    registry.add(ids.messages_rejected, 10).unwrap();
    registry.set_gauge(ids.active_sessions, 4).unwrap();
    registry.set_gauge(ids.queue_depth, 12).unwrap();

    for value in [100, 200, 300, 400, 500] {
        registry.record(ids.processing_latency, value).unwrap();
    }

    let mut output = Vec::new();

    write_prometheus(&mut output, registry.snapshots()).unwrap();

    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("maxwell_messages_received_total 1000\n"));

    assert!(text.contains("maxwell_messages_processed_total 990\n"));

    assert!(text.contains("maxwell_messages_rejected_total 10\n"));

    assert!(text.contains("maxwell_active_sessions 4\n"));

    assert!(text.contains("maxwell_queue_depth 12\n"));

    assert!(text.contains("maxwell_processing_latency_nanoseconds_count 5\n"));

    assert!(text.contains("maxwell_processing_latency_nanoseconds_sum 1500\n"));
}

#[test]
fn reset_preserves_registered_metrics() {
    let (mut registry, ids) = setup();

    registry.add(ids.messages_received, 100).unwrap();
    registry.set_gauge(ids.queue_depth, 10).unwrap();
    registry.record(ids.processing_latency, 500).unwrap();

    registry.reset_all();

    assert_eq!(registry.len(), 8);
    assert_eq!(registry.capacity(), 16);

    assert_eq!(registry.counter(ids.messages_received).unwrap().value(), 0);

    assert_eq!(registry.gauge(ids.queue_depth).unwrap().value(), 0);

    assert_eq!(
        registry.histogram(ids.processing_latency).unwrap().count(),
        0
    );

    registry.increment(ids.messages_received).unwrap();

    assert_eq!(registry.counter(ids.messages_received).unwrap().value(), 1);
}
