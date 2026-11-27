//! Retransmission and contiguous replay integration tests.

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, InstrumentId, MarketEvent, MonotonicTimestamp,
    SequenceNumber, SessionId, StreamId, TradingStatus, TradingStatusChanged,
};

use maxwell_recovery::{
    IngestOutcome, RecoveryConfig, RecoveryError, RecoveryManager, RecoveryRequest, RecoveryState,
    RetransmissionRequest,
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

fn manager() -> RecoveryManager {
    let mut manager = RecoveryManager::new(feed(), RecoveryConfig::default()).unwrap();

    manager.initialize_live(SequenceNumber::ONE).unwrap();

    manager
}

fn request(manager: &RecoveryManager) -> RetransmissionRequest {
    match manager.pending_request().unwrap() {
        RecoveryRequest::Retransmission(request) => request,
        RecoveryRequest::Snapshot(_) => panic!("expected retransmission request"),
    }
}

#[test]
fn contiguous_live_processing() {
    let mut manager = manager();
    let mut applied = Vec::new();

    for sequence in 1..=100 {
        manager
            .ingest(event(sequence), time(sequence), &mut |event| {
                applied.push(event.sequence().raw());
                Ok(())
            })
            .unwrap();
    }

    assert_eq!(applied, (1..=100).collect::<Vec<_>>());
    assert_eq!(manager.state(), RecoveryState::Live);
    assert_eq!(manager.buffered_events(), 0);
    assert!(manager.pending_request().is_none());
}

#[test]
fn single_missing_event() {
    let mut manager = manager();
    let mut applied = Vec::new();

    let mut apply = |event: &EventEnvelope| {
        applied.push(event.sequence().raw());
        Ok(())
    };

    manager.ingest(event(1), time(1), &mut apply).unwrap();

    manager.ingest(event(3), time(2), &mut apply).unwrap();

    assert!(!manager.is_publishable());
    assert_eq!(manager.buffered_events(), 1);

    let request = request(&manager);

    assert_eq!(request.range.start(), SequenceNumber::new(2));
    assert_eq!(request.range.end(), SequenceNumber::new(2));

    manager.acknowledge_request(request.id).unwrap();

    manager
        .ingest_retransmission(request.id, event(2), time(3), &mut apply)
        .unwrap();

    assert_eq!(applied, vec![1, 2, 3]);
    assert!(manager.is_publishable());
    assert_eq!(manager.buffered_events(), 0);
}

#[test]
fn out_of_order_retransmission() {
    let mut manager = manager();
    let mut applied = Vec::new();

    let mut apply = |event: &EventEnvelope| {
        applied.push(event.sequence().raw());
        Ok(())
    };

    manager.ingest(event(5), time(1), &mut apply).unwrap();

    let request = request(&manager);

    for (index, sequence) in [4, 2, 3, 1].into_iter().enumerate() {
        manager
            .ingest_retransmission(
                request.id,
                event(sequence),
                time(index as u64 + 2),
                &mut apply,
            )
            .unwrap();
    }

    assert_eq!(applied, vec![1, 2, 3, 4, 5]);
    assert!(manager.is_publishable());
}

#[test]
fn duplicate_live_event_not_applied_twice() {
    let mut manager = manager();
    let mut count = 0;

    let mut apply = |_: &EventEnvelope| {
        count += 1;
        Ok(())
    };

    manager.ingest(event(1), time(1), &mut apply).unwrap();

    let result = manager.ingest(event(1), time(2), &mut apply).unwrap();

    assert_eq!(result, IngestOutcome::Duplicate);
    assert_eq!(count, 1);
    assert_eq!(manager.stats().duplicate_events, 1);
}

#[test]
fn application_failure_does_not_advance_sequence() {
    let mut manager = manager();

    let result = manager.ingest(event(1), time(1), &mut |_| {
        Err(RecoveryError::EventApplicationFailed)
    });

    assert_eq!(result, Err(RecoveryError::EventApplicationFailed),);

    assert_eq!(manager.next_expected(), SequenceNumber::ONE,);

    assert_eq!(manager.state(), RecoveryState::Failed);
    assert!(!manager.is_publishable());
}

#[test]
fn retransmission_batches_are_bounded() {
    let config = RecoveryConfig {
        max_request_batch: 3,
        ..RecoveryConfig::default()
    };

    let mut manager = RecoveryManager::new(feed(), config).unwrap();

    manager.initialize_live(SequenceNumber::ONE).unwrap();

    let mut applied = Vec::new();

    let mut apply = |event: &EventEnvelope| {
        applied.push(event.sequence().raw());
        Ok(())
    };

    manager.ingest(event(10), time(1), &mut apply).unwrap();

    for batch in 0..3_u64 {
        let request = request(&manager);

        let start = batch * 3 + 1;
        let end = start + 2;

        assert_eq!(request.range.start().raw(), start);
        assert_eq!(request.range.end().raw(), end);

        for sequence in start..=end {
            manager
                .ingest_retransmission(request.id, event(sequence), time(sequence + 1), &mut apply)
                .unwrap();
        }
    }

    assert_eq!(applied, (1..=10).collect::<Vec<_>>());
    assert!(manager.is_publishable());
    assert_eq!(manager.stats().retransmission_requests, 3);
}

#[test]
fn heartbeat_detects_trailing_packet_loss() {
    let mut manager = manager();

    manager
        .observe_watermark(feed(), SequenceNumber::new(3), time(1))
        .unwrap();

    let request = request(&manager);

    assert_eq!(request.range.start().raw(), 1);
    assert_eq!(request.range.end().raw(), 3);
    assert!(!manager.is_publishable());
}

#[test]
fn live_packets_can_complete_retransmission() {
    let mut manager = manager();

    let mut apply = |_: &EventEnvelope| Ok(());

    manager.ingest(event(3), time(1), &mut apply).unwrap();

    manager.ingest(event(1), time(2), &mut apply).unwrap();
    manager.ingest(event(2), time(3), &mut apply).unwrap();

    assert!(manager.is_publishable());
    assert_eq!(manager.next_expected().raw(), 4);
    assert!(manager.pending_request().is_none());
}
