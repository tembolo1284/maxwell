//! End-to-end simulator fault injection tests.

use maxwell_core::{
    EventEnvelope, FeedId, MonotonicTimestamp, SequenceNumber, SessionId, StreamId,
};

use maxwell_simulator::config::{BASIS_POINTS, FaultConfig, SimulatorConfig};
use maxwell_simulator::scenario::Scenario;
use maxwell_simulator::simulator::{
    DeliveryOutcome, EventSink, RuntimeLimits, Simulator, SimulatorState,
};
use maxwell_simulator::{SimulatorError, SimulatorResult};

// ============================================================
// Fixtures
// ============================================================

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(1))
}

fn config(event_count: u64) -> SimulatorConfig {
    let mut config = Scenario::Smoke.config().unwrap();

    config.run.event_count = event_count;

    config
}

fn simulator(config: SimulatorConfig) -> Simulator {
    Simulator::new(config, feed(), RuntimeLimits::default()).unwrap()
}

#[derive(Debug, Default)]
struct RecordingSink {
    events: Vec<EventEnvelope>,
    timestamps: Vec<u64>,
    blocked: bool,
}

impl EventSink for RecordingSink {
    fn try_send(
        &mut self,
        event: &EventEnvelope,
        received_at: MonotonicTimestamp,
    ) -> SimulatorResult<DeliveryOutcome> {
        if self.blocked {
            return Ok(DeliveryOutcome::Blocked);
        }

        self.events.push(*event);
        self.timestamps.push(received_at.raw());

        Ok(DeliveryOutcome::Accepted)
    }
}

fn sequences(sink: &RecordingSink) -> Vec<u64> {
    sink.events
        .iter()
        .map(|event| event.sequence().raw())
        .collect()
}

// ============================================================
// Clean delivery
// ============================================================

#[test]
fn clean_delivery_preserves_authoritative_order() {
    let mut simulator = simulator(config(1_000));
    let mut sink = RecordingSink::default();

    let report = simulator.run_until_blocked(&mut sink).unwrap();

    assert!(report.is_completed());

    assert_eq!(sequences(&sink), (1..=1_000).collect::<Vec<_>>(),);

    assert_eq!(simulator.stats().faults.dropped, 0);
    assert_eq!(simulator.stats().faults.duplicated, 0);
    assert_eq!(simulator.stats().faults.reordered, 0);

    assert!(simulator.stats().is_fully_accounted());
}

// ============================================================
// Packet loss
// ============================================================

#[test]
fn complete_packet_loss_preserves_authoritative_history() {
    let mut config = config(1_000);

    config.faults = FaultConfig {
        drop_bps: BASIS_POINTS,
        ..FaultConfig::disabled()
    };

    let mut simulator = simulator(config);
    let mut sink = RecordingSink::default();

    simulator.run_until_blocked(&mut sink).unwrap();

    assert!(sink.events.is_empty());
    assert_eq!(simulator.generated(), 1_000);
    assert_eq!(simulator.history().len(), 1_000);
    assert_eq!(simulator.stats().faults.dropped, 1_000);

    let recovered: Vec<_> = simulator
        .retransmission(SequenceNumber::ONE, SequenceNumber::new(1_000))
        .unwrap()
        .map(|event| event.sequence().raw())
        .collect();

    assert_eq!(recovered, (1..=1_000).collect::<Vec<_>>(),);

    assert!(simulator.stats().is_fully_accounted());
}

#[test]
fn partial_packet_loss_creates_recoverable_sequence_gaps() {
    let mut config = config(10_000);

    config.faults = FaultConfig {
        drop_bps: 1_000,
        ..FaultConfig::disabled()
    };

    let mut simulator = simulator(config);
    let mut sink = RecordingSink::default();

    simulator.run_until_blocked(&mut sink).unwrap();

    let stats = simulator.stats();

    assert!(stats.faults.dropped > 0);
    assert!(stats.delivery.accepted > 0);

    assert_eq!(
        stats.generated(),
        stats.delivery.accepted + stats.faults.dropped,
    );

    assert!(
        sink.events
            .windows(2)
            .any(|pair| { pair[1].sequence().raw() > pair[0].sequence().raw() + 1 }),
    );

    assert_eq!(simulator.history().len(), 10_000);
}

// ============================================================
// Duplicate delivery
// ============================================================

#[test]
fn complete_duplication_preserves_original_payloads() {
    let mut config = config(1_000);

    config.faults = FaultConfig {
        duplicate_bps: BASIS_POINTS,
        ..FaultConfig::disabled()
    };

    let mut simulator = simulator(config);
    let mut sink = RecordingSink::default();

    simulator.run_until_blocked(&mut sink).unwrap();

    assert_eq!(sink.events.len(), 2_000);

    for (index, pair) in sink.events.chunks_exact(2).enumerate() {
        assert_eq!(pair[0], pair[1]);

        assert_eq!(pair[0].sequence().raw(), index as u64 + 1,);
    }

    assert_eq!(simulator.stats().faults.duplicated, 1_000);
    assert!(simulator.stats().is_fully_accounted());
}

// ============================================================
// Reordering
// ============================================================

#[test]
fn complete_reordering_preserves_every_event() {
    let mut config = config(1_003);

    config.faults = FaultConfig {
        reorder_bps: BASIS_POINTS,
        reorder_window: 4,
        ..FaultConfig::disabled()
    };

    let mut simulator = simulator(config);
    let mut sink = RecordingSink::default();

    simulator.run_until_blocked(&mut sink).unwrap();

    let mut delivered = sequences(&sink);

    assert_eq!(&delivered[..5], &[5, 1, 2, 3, 4]);

    delivered.sort_unstable();

    assert_eq!(delivered, (1..=1_003).collect::<Vec<_>>(),);

    assert_eq!(simulator.pending_delayed(), 0);
    assert_eq!(simulator.pending_delivery(), 0);

    assert!(simulator.stats().is_fully_accounted());
}

#[test]
fn reordered_delivery_preserves_monotonic_reception_time() {
    let config = Scenario::OutOfOrder.config().unwrap();

    let mut simulator = simulator(config);
    let mut sink = RecordingSink::default();

    simulator.run_until_blocked(&mut sink).unwrap();

    assert!(sink.timestamps.windows(2).all(|pair| { pair[0] < pair[1] }),);

    assert!(
        sink.events
            .windows(2)
            .any(|pair| { pair[0].sequence() > pair[1].sequence() }),
    );
}

// ============================================================
// Mixed faults and reproducibility
// ============================================================

#[test]
fn mixed_fault_delivery_is_reproducible() {
    let mut config = Scenario::MixedFaults.config().unwrap();

    config.run.event_count = 10_000;

    let mut first = simulator(config);
    let mut second = simulator(config);

    let mut first_sink = RecordingSink::default();
    let mut second_sink = RecordingSink::default();

    first.run_until_blocked(&mut first_sink).unwrap();
    second.run_until_blocked(&mut second_sink).unwrap();

    assert_eq!(first_sink.events, second_sink.events);
    assert_eq!(first_sink.timestamps, second_sink.timestamps);

    assert_eq!(first.stats().faults, second.stats().faults,);

    assert_eq!(first.stats().events, second.stats().events,);

    assert!(first.stats().is_fully_accounted());
    assert!(second.stats().is_fully_accounted());
}

#[test]
fn delivery_faults_do_not_change_authoritative_market() {
    let clean_config = config(20_000);

    let mut faulty_config = clean_config;

    faulty_config.faults = FaultConfig {
        drop_bps: 1_000,
        duplicate_bps: 2_000,
        reorder_bps: 3_000,
        reorder_window: 16,
    };

    let mut clean = simulator(clean_config);
    let mut faulty = simulator(faulty_config);

    let mut clean_sink = RecordingSink::default();
    let mut faulty_sink = RecordingSink::default();

    clean.run_until_blocked(&mut clean_sink).unwrap();
    faulty.run_until_blocked(&mut faulty_sink).unwrap();

    assert_ne!(clean_sink.events, faulty_sink.events);

    assert!(clean.history().iter().eq(faulty.history().iter()),);

    let watermark = SequenceNumber::new(20_000);

    assert_eq!(
        clean.snapshot(watermark).unwrap(),
        faulty.snapshot(watermark).unwrap(),
    );

    assert_eq!(clean.stats().events, faulty.stats().events,);
}

// ============================================================
// Backpressure
// ============================================================

#[test]
fn blocked_delivery_never_loses_or_regenerates_events() {
    let mut config = config(1_000);

    config.resources.delivery_capacity = 8;

    let limits = RuntimeLimits {
        delivery_capacity: 8,
        generation_budget: 32,
        delivery_budget: 32,
        ..RuntimeLimits::default()
    };

    let mut simulator = Simulator::new(config, feed(), limits).unwrap();

    let mut sink = RecordingSink {
        blocked: true,
        ..RecordingSink::default()
    };

    let report = simulator.run_until_blocked(&mut sink).unwrap();

    assert!(report.is_blocked());
    assert_eq!(simulator.generated(), 8);
    assert_eq!(simulator.pending_delivery(), 8);

    for _ in 0..10 {
        let report = simulator.service(&mut sink).unwrap();

        assert!(!report.made_progress());
        assert_eq!(simulator.generated(), 8);
        assert_eq!(simulator.history().len(), 8);
        assert_eq!(simulator.pending_delivery(), 8);
    }

    sink.blocked = false;

    simulator.run_until_blocked(&mut sink).unwrap();

    assert_eq!(sequences(&sink), (1..=1_000).collect::<Vec<_>>(),);

    assert!(simulator.stats().is_fully_accounted());

    simulator.validate_consistency().unwrap();
}

#[test]
fn duplicate_admission_is_atomic_under_backpressure() {
    let mut config = config(100);

    config.faults = FaultConfig {
        duplicate_bps: BASIS_POINTS,
        ..FaultConfig::disabled()
    };

    config.resources.delivery_capacity = 3;

    let limits = RuntimeLimits {
        delivery_capacity: 3,
        generation_budget: 16,
        delivery_budget: 16,
        ..RuntimeLimits::default()
    };

    let mut simulator = Simulator::new(config, feed(), limits).unwrap();

    let mut sink = RecordingSink {
        blocked: true,
        ..RecordingSink::default()
    };

    simulator.run_until_blocked(&mut sink).unwrap();

    // Two copies fit. A second pair does not.
    assert_eq!(simulator.generated(), 1);
    assert_eq!(simulator.pending_delivery(), 2);

    sink.blocked = false;

    simulator.run_until_blocked(&mut sink).unwrap();

    assert_eq!(sink.events.len(), 200);

    for pair in sink.events.chunks_exact(2) {
        assert_eq!(pair[0], pair[1]);
    }

    assert!(simulator.stats().is_fully_accounted());
}

#[test]
fn graceful_stop_flushes_withheld_events() {
    let mut config = config(1_000);

    config.faults = FaultConfig {
        reorder_bps: BASIS_POINTS,
        reorder_window: 8,
        ..FaultConfig::disabled()
    };

    let limits = RuntimeLimits {
        generation_budget: 3,
        ..RuntimeLimits::default()
    };

    let mut simulator = Simulator::new(config, feed(), limits).unwrap();

    let mut sink = RecordingSink::default();

    simulator.start().unwrap();

    simulator.service(&mut sink).unwrap();

    assert_eq!(simulator.generated(), 3);
    assert_eq!(simulator.pending_delayed(), 3);
    assert!(sink.events.is_empty());

    simulator.request_stop().unwrap();

    simulator.run_until_blocked(&mut sink).unwrap();

    assert_eq!(simulator.state(), SimulatorState::Completed);
    assert_eq!(sequences(&sink), vec![1, 2, 3]);

    assert!(simulator.stats().is_fully_accounted());
}

// ============================================================
// Recovery history
// ============================================================

#[test]
fn history_eviction_requires_explicit_recovery_failure() {
    let mut config = config(100);

    config.resources.history_capacity = 16;

    let limits = RuntimeLimits {
        history_capacity: 16,
        ..RuntimeLimits::default()
    };

    let mut simulator = Simulator::new(config, feed(), limits).unwrap();

    let mut sink = RecordingSink::default();

    simulator.run_until_blocked(&mut sink).unwrap();

    assert_eq!(
        simulator.history().oldest_sequence(),
        Some(SequenceNumber::new(85)),
    );

    assert!(matches!(
        simulator.retransmission(SequenceNumber::new(80), SequenceNumber::new(90),),
        Err(SimulatorError::HistoryUnavailable { .. }),
    ));

    let retained = simulator
        .retransmission(SequenceNumber::new(85), SequenceNumber::new(100))
        .unwrap();

    assert_eq!(retained.len(), 16);
    assert_eq!(simulator.stats().history.evicted, 84);
}

#[test]
fn authoritative_snapshot_survives_history_eviction() {
    let mut config = config(1_000);

    config.resources.history_capacity = 8;

    let limits = RuntimeLimits {
        history_capacity: 8,
        ..RuntimeLimits::default()
    };

    let mut simulator = Simulator::new(config, feed(), limits).unwrap();

    let mut sink = RecordingSink::default();

    simulator.run_until_blocked(&mut sink).unwrap();

    assert_eq!(simulator.history().len(), 8);

    let snapshot = simulator.snapshot(SequenceNumber::new(1_000)).unwrap();

    assert_eq!(snapshot.watermark().raw(), 1_000);

    assert_eq!(snapshot.order_count(), simulator.market().total_orders(),);

    snapshot.validate_consistency().unwrap();
}

// ============================================================
// Accounting
// ============================================================

#[test]
fn every_fault_scenario_reconciles_final_delivery_counts() {
    for scenario in [
        Scenario::PacketLoss,
        Scenario::Duplicates,
        Scenario::OutOfOrder,
        Scenario::MixedFaults,
        Scenario::RecoveryStress,
    ] {
        let mut config = scenario.config().unwrap();

        config.run.event_count = 5_000;

        let mut simulator = simulator(config);
        let mut sink = RecordingSink::default();

        simulator.run_until_blocked(&mut sink).unwrap();

        let stats = simulator.stats();

        assert!(
            stats.is_fully_accounted(),
            "accounting failed for {scenario}",
        );

        assert_eq!(
            sink.events.len() as u128,
            stats.expected_delivery_messages(),
        );

        assert_eq!(simulator.pending_delivery(), 0);
        assert_eq!(simulator.pending_delayed(), 0);

        simulator.validate_consistency().unwrap();
    }
}
