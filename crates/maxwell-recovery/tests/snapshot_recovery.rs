//! Snapshot synchronization and session transition tests.

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, InstrumentId, MarketEvent, MonotonicTimestamp,
    SequenceNumber, SessionId, StreamId, TradingStatus, TradingStatusChanged,
};

use maxwell_recovery::{
    RecoveryConfig, RecoveryError, RecoveryManager, RecoveryRequest, RecoveryState,
    SnapshotRequest, SnapshotWatermark,
};

const fn feed(session: u64) -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(session))
}

const fn time(value: u64) -> MonotonicTimestamp {
    MonotonicTimestamp::new(value)
}

const fn event(session: u64, sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(session), SequenceNumber::new(sequence)),
            InstrumentId::new(1),
            None,
        ),
        MarketEvent::TradingStatusChanged(TradingStatusChanged {
            status: TradingStatus::Open,
        }),
    )
}

fn snapshot_request(manager: &RecoveryManager) -> SnapshotRequest {
    match manager.pending_request().unwrap() {
        RecoveryRequest::Snapshot(request) => request,
        RecoveryRequest::Retransmission(_) => panic!("expected snapshot request"),
    }
}

#[test]
fn startup_snapshot_synchronization() {
    let mut manager = RecoveryManager::new(feed(1), RecoveryConfig::default()).unwrap();

    manager.start_snapshot(time(1)).unwrap();

    let request = snapshot_request(&manager);

    manager
        .install_snapshot(
            SnapshotWatermark::new(request.id, feed(1), SequenceNumber::new(100)),
            time(2),
            || Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();

    assert_eq!(manager.next_expected().raw(), 101);
    assert!(manager.is_publishable());
}

#[test]
fn snapshot_discards_covered_events_and_replays_suffix() {
    let mut manager = RecoveryManager::new(feed(1), RecoveryConfig::default()).unwrap();

    manager.start_snapshot(time(1)).unwrap();

    let request = snapshot_request(&manager);

    let mut applied = Vec::new();

    let mut apply = |event: &EventEnvelope| {
        applied.push(event.sequence().raw());
        Ok(())
    };

    for (index, sequence) in [105, 103, 104, 102].into_iter().enumerate() {
        manager
            .ingest(event(1, sequence), time(index as u64 + 2), &mut apply)
            .unwrap();
    }

    manager
        .install_snapshot(
            SnapshotWatermark::new(request.id, feed(1), SequenceNumber::new(103)),
            time(6),
            || Ok(()),
            &mut apply,
        )
        .unwrap();

    assert_eq!(applied, vec![104, 105]);
    assert_eq!(manager.next_expected().raw(), 106);
    assert_eq!(manager.buffered_events(), 0);
    assert!(manager.is_publishable());
}

#[test]
fn snapshot_with_remaining_gap_requires_retransmission() {
    let mut manager = RecoveryManager::new(feed(1), RecoveryConfig::default()).unwrap();

    manager.start_snapshot(time(1)).unwrap();

    let request = snapshot_request(&manager);

    manager
        .ingest(event(1, 105), time(2), &mut |_| Ok(()))
        .unwrap();

    manager
        .install_snapshot(
            SnapshotWatermark::new(request.id, feed(1), SequenceNumber::new(102)),
            time(3),
            || Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();

    match manager.pending_request().unwrap() {
        RecoveryRequest::Retransmission(request) => {
            assert_eq!(request.range.start().raw(), 103);
            assert_eq!(request.range.end().raw(), 104);
        }

        RecoveryRequest::Snapshot(_) => panic!("expected retransmission"),
    }

    assert!(!manager.is_publishable());
}

#[test]
fn buffer_overflow_requires_covering_snapshot() {
    let config = RecoveryConfig {
        buffer_capacity: 2,
        ..RecoveryConfig::default()
    };

    let mut manager = RecoveryManager::new(feed(1), config).unwrap();

    manager.initialize_live(SequenceNumber::ONE).unwrap();

    for sequence in 10..=12 {
        manager
            .ingest(event(1, sequence), time(sequence), &mut |_| Ok(()))
            .unwrap();
    }

    let request = snapshot_request(&manager);

    assert_eq!(request.minimum_watermark.raw(), 12);
    assert_eq!(manager.buffered_events(), 0);

    let result = manager.install_snapshot(
        SnapshotWatermark::new(request.id, feed(1), SequenceNumber::new(11)),
        time(13),
        || panic!("invalid snapshot must not be installed"),
        &mut |_| Ok(()),
    );

    assert_eq!(result, Err(RecoveryError::SnapshotTooOld));

    manager
        .install_snapshot(
            SnapshotWatermark::new(request.id, feed(1), SequenceNumber::new(12)),
            time(14),
            || Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();

    assert!(manager.is_publishable());
    assert_eq!(manager.next_expected().raw(), 13);
}

#[test]
fn session_transition_rejects_old_feed_events() {
    let mut manager = RecoveryManager::new(feed(1), RecoveryConfig::default()).unwrap();

    manager.initialize_live(SequenceNumber::ONE).unwrap();

    manager.transition_session(feed(2), time(1)).unwrap();

    assert_eq!(manager.feed(), feed(2));
    assert!(!manager.is_publishable());

    let result = manager.ingest(event(1, 100), time(2), &mut |_| Ok(()));

    assert!(matches!(result, Err(RecoveryError::FeedMismatch { .. }),));

    let request = snapshot_request(&manager);

    manager
        .install_snapshot(
            SnapshotWatermark::new(request.id, feed(2), SequenceNumber::new(5)),
            time(3),
            || Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();

    assert_eq!(manager.next_expected().raw(), 6);
    assert!(manager.is_publishable());
}

#[test]
fn snapshot_installation_failure_marks_feed_failed() {
    let mut manager = RecoveryManager::new(feed(1), RecoveryConfig::default()).unwrap();

    manager.start_snapshot(time(1)).unwrap();

    let request = snapshot_request(&manager);

    let result = manager.install_snapshot(
        SnapshotWatermark::new(request.id, feed(1), SequenceNumber::new(100)),
        time(2),
        || Err(RecoveryError::SnapshotInstallFailed),
        &mut |_| Ok(()),
    );

    assert_eq!(result, Err(RecoveryError::SnapshotInstallFailed),);

    assert_eq!(manager.state(), RecoveryState::Failed);
}
