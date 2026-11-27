//! Deterministic recovery fault injection.

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, InstrumentId, MarketEvent, MonotonicTimestamp,
    NanoDuration, SequenceNumber, SessionId, StreamId, TradingStatus, TradingStatusChanged,
};

use maxwell_recovery::{
    RecoveryConfig, RecoveryError, RecoveryManager, RecoveryRequest, RecoveryState,
};

const fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(1))
}

const fn time(value: u64) -> MonotonicTimestamp {
    MonotonicTimestamp::new(value)
}

const fn event(sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(), SequenceNumber::new(sequence)),
            InstrumentId::new(1),
            None,
        ),
        MarketEvent::TradingStatusChanged(TradingStatusChanged {
            status: TradingStatus::Open,
        }),
    )
}

fn config() -> RecoveryConfig {
    RecoveryConfig {
        retransmission_timeout: NanoDuration::from_nanos(100),
        snapshot_timeout: NanoDuration::from_nanos(500),
        max_retransmission_attempts: 3,
        max_snapshot_attempts: 2,
        ..RecoveryConfig::default()
    }
}

fn manager() -> RecoveryManager {
    let mut manager = RecoveryManager::new(feed(), config()).unwrap();

    manager.initialize_live(SequenceNumber::ONE).unwrap();

    manager
}

#[test]
fn retransmission_timeout_escalates_to_snapshot() {
    let mut manager = manager();

    manager.ingest(event(10), time(0), &mut |_| Ok(())).unwrap();

    for attempt in 1..=3_u32 {
        match manager.pending_request().unwrap() {
            RecoveryRequest::Retransmission(request) => {
                assert_eq!(request.attempt, attempt);
            }

            RecoveryRequest::Snapshot(_) => panic!("expected retransmission"),
        }

        manager.poll(time(u64::from(attempt) * 100)).unwrap();
    }

    assert_eq!(manager.state(), RecoveryState::AwaitingSnapshot,);

    assert_eq!(manager.stats().retransmission_requests, 3);
    assert_eq!(manager.stats().retransmission_timeouts, 3);
}

#[test]
fn snapshot_timeout_exhaustion_fails_closed() {
    let mut manager = manager();

    manager.start_snapshot(time(0)).unwrap();

    manager.poll(time(500)).unwrap();

    assert_eq!(manager.state(), RecoveryState::AwaitingSnapshot,);

    manager.poll(time(1000)).unwrap();

    assert_eq!(manager.state(), RecoveryState::Failed);
    assert!(!manager.is_publishable());
    assert!(manager.pending_request().is_none());
}

#[test]
fn stale_retransmission_response_is_rejected() {
    let mut manager = manager();

    manager.ingest(event(5), time(0), &mut |_| Ok(())).unwrap();

    let old_id = manager.pending_request().unwrap().id();

    manager.poll(time(100)).unwrap();

    let new_id = manager.pending_request().unwrap().id();

    assert_ne!(old_id, new_id);

    let result = manager.ingest_retransmission(old_id, event(1), time(101), &mut |_| Ok(()));

    assert_eq!(result, Err(RecoveryError::StaleResponse));
    assert_eq!(manager.next_expected().raw(), 1);
}

#[test]
fn conflicting_buffered_events_fail_closed() {
    let mut manager = manager();

    manager.ingest(event(5), time(1), &mut |_| Ok(())).unwrap();

    let mut conflicting = event(5);

    conflicting.event = MarketEvent::TradingStatusChanged(TradingStatusChanged {
        status: TradingStatus::Halted,
    });

    let result = manager.ingest(conflicting, time(2), &mut |_| Ok(()));

    assert!(matches!(
        result,
        Err(RecoveryError::ConflictingEvent { .. }),
    ));

    assert_eq!(manager.state(), RecoveryState::Failed);
}

#[test]
fn backward_clock_rejected() {
    let mut manager = manager();

    manager.poll(time(100)).unwrap();

    assert_eq!(
        manager.poll(time(99)),
        Err(RecoveryError::ClockWentBackward),
    );
}

#[test]
fn retry_requests_only_uncommitted_suffix() {
    let mut manager = manager();

    manager.ingest(event(10), time(0), &mut |_| Ok(())).unwrap();

    let id = manager.pending_request().unwrap().id();

    for sequence in 1..=3 {
        manager
            .ingest_retransmission(id, event(sequence), time(sequence), &mut |_| Ok(()))
            .unwrap();
    }

    manager.poll(time(100)).unwrap();

    match manager.pending_request().unwrap() {
        RecoveryRequest::Retransmission(request) => {
            assert_eq!(request.range.start().raw(), 4);
            assert_eq!(request.range.end().raw(), 9);
            assert_eq!(request.attempt, 2);
        }

        RecoveryRequest::Snapshot(_) => panic!("expected retransmission"),
    }
}

// ============================================================
// Deterministic pseudo-random generator
// ============================================================

struct XorShift64(u64);

impl XorShift64 {
    const fn next(&mut self) -> u64 {
        let mut value = self.0;

        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;

        self.0 = value;

        value
    }
}

#[test]
fn randomized_loss_duplication_and_reordering() {
    const EVENTS: u64 = 10_000;
    const WINDOW: u64 = 64;

    let mut rng = XorShift64(0x1234_5678_ABCD_EF01);

    let mut manager = RecoveryManager::new(feed(), RecoveryConfig::default()).unwrap();

    manager.initialize_live(SequenceNumber::ONE).unwrap();

    let mut applied = Vec::new();
    let mut clock = 0_u64;

    let mut apply = |event: &EventEnvelope| {
        applied.push(event.sequence().raw());
        Ok(())
    };

    let mut window_start = 1_u64;

    while window_start <= EVENTS {
        let window_end = (window_start + WINDOW - 1).min(EVENTS);

        let mut delivery = Vec::new();

        for sequence in window_start..=window_end {
            // Approximately 10% packet loss.
            if rng.next().is_multiple_of(10) {
                continue;
            }

            delivery.push(sequence);

            // Approximately 10% duplication among
            // delivered events.
            if rng.next().is_multiple_of(10) {
                delivery.push(sequence);
            }
        }

        // Fisher-Yates shuffle.
        for index in (1..delivery.len()).rev() {
            let other = usize::try_from(rng.next() % (index as u64 + 1)).unwrap();

            delivery.swap(index, other);
        }

        for sequence in delivery {
            clock += 1;

            manager
                .ingest(event(sequence), time(clock), &mut apply)
                .unwrap();
        }

        // Simulate an upstream heartbeat revealing
        // the latest published event sequence.
        clock += 1;

        manager
            .observe_watermark(feed(), SequenceNumber::new(window_end), time(clock))
            .unwrap();

        // Simulate a reliable historical event server.
        while let Some(request) = manager.pending_request() {
            match request {
                RecoveryRequest::Retransmission(request) => {
                    manager.acknowledge_request(request.id).unwrap();

                    for sequence in request.range {
                        // The request may have completed
                        // early because buffered events
                        // were drained.
                        if sequence < manager.next_expected() {
                            continue;
                        }

                        clock += 1;

                        manager
                            .ingest_retransmission(
                                request.id,
                                event(sequence.raw()),
                                time(clock),
                                &mut apply,
                            )
                            .unwrap();
                    }
                }

                RecoveryRequest::Snapshot(_) => {
                    panic!("unexpected snapshot escalation");
                }
            }
        }

        assert!(manager.is_publishable());

        assert_eq!(manager.next_expected().raw(), window_end + 1,);

        window_start = window_end + 1;
    }

    assert_eq!(applied, (1..=EVENTS).collect::<Vec<_>>(),);

    assert_eq!(manager.stats().events_applied, EVENTS);
    assert!(manager.stats().gaps_detected > 0);
    assert!(manager.stats().duplicate_events > 0);
    assert_eq!(manager.stats().recovery_failures, 0);
    assert_eq!(manager.buffered_events(), 0);
}

#[test]
fn one_million_contiguous_events() {
    const EVENTS: u64 = 1_000_000;

    let mut manager = manager();

    let mut count = 0_u64;

    let mut apply = |event: &EventEnvelope| {
        count += 1;

        assert_eq!(event.sequence().raw(), count);

        Ok(())
    };

    for sequence in 1..=EVENTS {
        manager
            .ingest(event(sequence), time(sequence), &mut apply)
            .unwrap();
    }

    assert_eq!(count, EVENTS);
    assert_eq!(manager.stats().events_applied, EVENTS);
    assert_eq!(manager.stats().gaps_detected, 0);
    assert_eq!(manager.stats().events_buffered, 0);
    assert!(manager.is_publishable());
}
