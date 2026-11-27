
//! Recovery, session transition, and failure tests.

use maxwell_core::*;

use maxwell_sequencer::{
    IngestOutcome,
    SequencerConfig,
    SequencerError,
    SequencerState,
    StreamSequencer,
};

fn seq(value: u64) -> SequenceNumber {
    SequenceNumber::new(value)
}

fn feed(session: u64) -> FeedId {
    FeedId::new(
        StreamId::new(1),
        SessionId::new(session),
    )
}

fn event(session: u64, sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(session), seq(sequence)),
            InstrumentId::new(42),
            None,
        ),
        MarketEvent::OrderAdded(OrderAdded {
            order_id: OrderId::new(sequence),
            price: Price::new(10_000),
            quantity: Quantity::new(100),
            side: Side::Buy,
        }),
    )
}

fn stream(expected: u64) -> StreamSequencer {
    StreamSequencer::new(
        feed(1),
        seq(expected),
        SequencerConfig::new(8, 1),
    )
    .unwrap()
}

// ============================================================
// Reorder window exhaustion
// ============================================================

#[test]
fn excessive_gap_requires_recovery() {
    let mut sequencer = stream(100);

    let result = sequencer.ingest(event(1, 108), |_| {});

    assert!(matches!(
        result,
        Err(SequencerError::SequenceOutsideWindow { .. }),
    ));

    assert_eq!(
        sequencer.state(),
        SequencerState::RecoveryRequired,
    );

    assert_eq!(sequencer.expected(), Some(seq(100)));
    assert_eq!(sequencer.metrics().recovery_requests, 1);
}

#[test]
fn recovery_state_is_sticky() {
    let mut sequencer = stream(100);

    sequencer.require_recovery();

    assert_eq!(
        sequencer.ingest(event(1, 100), |_| {}),
        Err(SequencerError::RecoveryRequired),
    );

    assert_eq!(sequencer.expected(), Some(seq(100)));
}

#[test]
fn repeated_recovery_requests_not_double_counted() {
    let mut sequencer = stream(100);

    sequencer.require_recovery();
    sequencer.require_recovery();

    assert_eq!(sequencer.metrics().recovery_requests, 1);
}

// ============================================================
// Snapshot checkpoint installation
// ============================================================

#[test]
fn checkpoint_restores_live_state() {
    let mut sequencer = stream(100);

    sequencer.ingest(event(1, 103), |_| {}).unwrap();
    sequencer.require_recovery();

    // Represents an externally installed snapshot
    // containing all events through sequence 199.
    sequencer.install_checkpoint(seq(200)).unwrap();

    assert_eq!(sequencer.state(), SequencerState::Live);
    assert_eq!(sequencer.expected(), Some(seq(200)));
    assert_eq!(sequencer.buffered(), 0);

    assert_eq!(
        sequencer.ingest(event(1, 200), |_| {}),
        Ok(IngestOutcome::Delivered { count: 1 }),
    );
}

#[test]
fn checkpoint_cannot_regress_watermark() {
    let mut sequencer = stream(100);

    sequencer.ingest(event(1, 100), |_| {}).unwrap();

    assert_eq!(
        sequencer.install_checkpoint(seq(99)),
        Err(SequencerError::InvalidRecoverySequence),
    );

    assert_eq!(sequencer.expected(), Some(seq(101)));
}

// ============================================================
// Session transitions
// ============================================================

#[test]
fn unexpected_session_rejected() {
    let mut sequencer = stream(100);

    assert!(matches!(
        sequencer.ingest(event(2, 100), |_| {}),
        Err(SequencerError::FeedMismatch { .. }),
    ));

    assert_eq!(sequencer.expected(), Some(seq(100)));
}

#[test]
fn explicit_session_transition() {
    let mut sequencer = stream(100);

    sequencer.ingest(event(1, 103), |_| {}).unwrap();

    sequencer.start_session(feed(2), seq(1)).unwrap();

    assert_eq!(sequencer.feed(), feed(2));
    assert_eq!(sequencer.expected(), Some(seq(1)));
    assert_eq!(sequencer.buffered(), 0);
    assert_eq!(sequencer.state(), SequencerState::Live);

    assert_eq!(
        sequencer.ingest(event(2, 1), |_| {}),
        Ok(IngestOutcome::Delivered { count: 1 }),
    );
}

#[test]
fn old_session_packets_rejected() {
    let mut sequencer = stream(100);

    sequencer.start_session(feed(2), seq(1)).unwrap();

    assert!(matches!(
        sequencer.ingest(event(1, 100), |_| {}),
        Err(SequencerError::FeedMismatch { .. }),
    ));
}

#[test]
fn same_session_cannot_be_restarted() {
    let mut sequencer = stream(100);

    assert_eq!(
        sequencer.start_session(feed(1), seq(1)),
        Err(SequencerError::InvalidRecoverySequence),
    );
}

// ============================================================
// Sequence exhaustion
// ============================================================

#[test]
fn maximum_sequence_delivered_without_wrapping() {
    let mut sequencer = stream(u64::MAX);

    assert_eq!(
        sequencer.ingest(event(1, u64::MAX), |_| {}),
        Ok(IngestOutcome::Delivered { count: 1 }),
    );

    assert_eq!(sequencer.expected(), None);
    assert_eq!(sequencer.state(), SequencerState::Exhausted);

    assert_eq!(
        sequencer.ingest(event(1, 0), |_| {}),
        Err(SequencerError::SequenceExhausted),
    );
}

#[test]
fn new_session_after_exhaustion() {
    let mut sequencer = stream(u64::MAX);

    sequencer.ingest(event(1, u64::MAX), |_| {}).unwrap();

    sequencer.start_session(feed(2), seq(1)).unwrap();

    assert_eq!(sequencer.state(), SequencerState::Live);
    assert_eq!(sequencer.expected(), Some(seq(1)));
}

// ============================================================
// Conflicting duplicates
// ============================================================

#[test]
fn conflicting_buffered_duplicate_requires_recovery() {
    let mut sequencer = stream(100);

    sequencer.ingest(event(1, 102), |_| {}).unwrap();

    let mut conflicting = event(1, 102);

    if let MarketEvent::OrderAdded(ref mut value) =
        conflicting.event
    {
        value.quantity = Quantity::new(999);
    }

    assert_eq!(
        sequencer.ingest(conflicting, |_| {}),
        Err(SequencerError::ConflictingDuplicate(seq(102))),
    );

    assert_eq!(
        sequencer.state(),
        SequencerState::RecoveryRequired,
    );
}

// ============================================================
// Downstream failure
// ============================================================

#[test]
fn downstream_panic_leaves_stream_fail_closed() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let mut sequencer = stream(100);

    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = sequencer.ingest(event(1, 100), |_| {
            panic!("simulated downstream failure");
        });
    }));

    assert!(result.is_err());

    assert_eq!(
        sequencer.state(),
        SequencerState::RecoveryRequired,
    );

    assert_eq!(sequencer.expected(), Some(seq(100)));
}

