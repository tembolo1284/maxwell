//! End-to-end sequencing and stream isolation tests.

use maxwell_core::*;

use maxwell_sequencer::{
    IngestOutcome, Sequencer, SequencerConfig, SequencerError, SequencerState, StreamSequencer,
};

const fn seq(value: u64) -> SequenceNumber {
    SequenceNumber::new(value)
}

const fn feed(stream: u32, session: u64) -> FeedId {
    FeedId::new(StreamId::new(stream), SessionId::new(session))
}

const fn event(stream: u32, session: u64, sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(stream, session), seq(sequence)),
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
    StreamSequencer::new(feed(1, 1), seq(expected), SequencerConfig::new(16, 4)).unwrap()
}

// ============================================================
// Contiguous delivery
// ============================================================

#[test]
fn contiguous_events_delivered_immediately() {
    let mut sequencer = stream(100);
    let mut delivered = Vec::new();

    for sequence in 100..110 {
        let outcome = sequencer
            .ingest(event(1, 1, sequence), |value| {
                delivered.push(value.header.sequence().raw());
            })
            .unwrap();

        assert_eq!(outcome, IngestOutcome::Delivered { count: 1 },);
    }

    assert_eq!(delivered, (100..110).collect::<Vec<_>>(),);

    assert_eq!(sequencer.expected(), Some(seq(110)));
    assert_eq!(sequencer.state(), SequencerState::Live);
    assert_eq!(sequencer.buffered(), 0);
}

// ============================================================
// Out-of-order delivery
// ============================================================

#[test]
fn out_of_order_events_are_reassembled() {
    let mut sequencer = stream(100);
    let mut delivered = Vec::new();

    for sequence in [100, 101, 104, 103, 102, 105] {
        sequencer
            .ingest(event(1, 1, sequence), |value| {
                delivered.push(value.header.sequence().raw());
            })
            .unwrap();
    }

    assert_eq!(delivered, vec![100, 101, 102, 103, 104, 105]);
    assert_eq!(sequencer.expected(), Some(seq(106)));
    assert_eq!(sequencer.buffered(), 0);
    assert_eq!(sequencer.state(), SequencerState::Live);

    let metrics = sequencer.metrics();

    assert_eq!(metrics.received, 6);
    assert_eq!(metrics.delivered, 6);
    assert_eq!(metrics.out_of_order, 2);
    assert_eq!(metrics.gaps_detected, 1);
    assert_eq!(metrics.gaps_resolved, 1);
}

#[test]
fn missing_event_blocks_future_delivery() {
    let mut sequencer = stream(100);

    let outcome = sequencer
        .ingest(event(1, 1, 103), |_| {
            panic!("future event must not be delivered");
        })
        .unwrap();

    assert_eq!(outcome, IngestOutcome::Buffered);
    assert_eq!(sequencer.state(), SequencerState::GapDetected);
    assert_eq!(sequencer.expected(), Some(seq(100)));

    let gap = sequencer.current_gap().unwrap();

    assert_eq!(gap.start(), seq(100));
    assert_eq!(gap.end(), seq(102));
}

#[test]
fn filling_gap_drains_contiguous_buffer() {
    let mut sequencer = stream(100);

    for sequence in [101, 102, 103] {
        sequencer.ingest(event(1, 1, sequence), |_| {}).unwrap();
    }

    let mut delivered = Vec::new();

    let outcome = sequencer
        .ingest(event(1, 1, 100), |value| {
            delivered.push(value.header.sequence().raw());
        })
        .unwrap();

    assert_eq!(outcome, IngestOutcome::Delivered { count: 4 },);

    assert_eq!(delivered, vec![100, 101, 102, 103]);
}

#[test]
fn multiple_disjoint_gaps() {
    let mut sequencer = stream(100);

    sequencer.ingest(event(1, 1, 102), |_| {}).unwrap();
    sequencer.ingest(event(1, 1, 105), |_| {}).unwrap();

    let gap = sequencer.current_gap().unwrap();

    assert_eq!(gap.start(), seq(100));
    assert_eq!(gap.end(), seq(101));

    sequencer.ingest(event(1, 1, 100), |_| {}).unwrap();
    sequencer.ingest(event(1, 1, 101), |_| {}).unwrap();

    let gap = sequencer.current_gap().unwrap();

    assert_eq!(gap.start(), seq(103));
    assert_eq!(gap.end(), seq(104));
}

// ============================================================
// Duplicate suppression
// ============================================================

#[test]
fn delivered_duplicate_suppressed() {
    let mut sequencer = stream(100);

    sequencer.ingest(event(1, 1, 100), |_| {}).unwrap();

    let outcome = sequencer
        .ingest(event(1, 1, 100), |_| {
            panic!("duplicate delivered");
        })
        .unwrap();

    assert_eq!(outcome, IngestOutcome::Duplicate);
    assert_eq!(sequencer.metrics().duplicates, 1);
}

#[test]
fn buffered_duplicate_suppressed() {
    let mut sequencer = stream(100);

    sequencer.ingest(event(1, 1, 102), |_| {}).unwrap();

    let outcome = sequencer
        .ingest(event(1, 1, 102), |_| {
            panic!("duplicate delivered");
        })
        .unwrap();

    assert_eq!(outcome, IngestOutcome::Duplicate);
    assert_eq!(sequencer.buffered(), 1);
}

// ============================================================
// Stream isolation
// ============================================================

#[test]
fn streams_have_independent_watermarks() {
    let mut sequencer = Sequencer::new(SequencerConfig::new(16, 4)).unwrap();

    sequencer
        .register(StreamId::new(1), feed(1, 1), seq(100))
        .unwrap();

    sequencer
        .register(StreamId::new(2), feed(2, 1), seq(500))
        .unwrap();

    sequencer.ingest(event(1, 1, 100), |_| {}).unwrap();
    sequencer.ingest(event(2, 1, 500), |_| {}).unwrap();
    sequencer.ingest(event(1, 1, 103), |_| {}).unwrap();

    let first = sequencer.stream(StreamId::new(1)).unwrap();
    let second = sequencer.stream(StreamId::new(2)).unwrap();

    assert_eq!(first.expected(), Some(seq(101)));
    assert_eq!(second.expected(), Some(seq(501)));

    assert_eq!(first.state(), SequencerState::GapDetected);
    assert_eq!(second.state(), SequencerState::Live);
}

#[test]
fn unknown_stream_rejected() {
    let mut sequencer = Sequencer::new(SequencerConfig::default()).unwrap();

    assert_eq!(
        sequencer.ingest(event(1, 1, 100), |_| {}),
        Err(SequencerError::UnknownStream(StreamId::new(1))),
    );
}

#[test]
fn duplicate_registration_rejected() {
    let mut sequencer = Sequencer::new(SequencerConfig::default()).unwrap();

    sequencer
        .register(StreamId::new(1), feed(1, 1), seq(100))
        .unwrap();

    assert_eq!(
        sequencer.register(StreamId::new(1), feed(1, 1), seq(100),),
        Err(SequencerError::StreamAlreadyRegistered(StreamId::new(1),),),
    );
}

#[test]
fn stream_capacity_enforced() {
    let mut sequencer = Sequencer::new(SequencerConfig::new(8, 1)).unwrap();

    sequencer
        .register(StreamId::new(1), feed(1, 1), seq(100))
        .unwrap();

    assert_eq!(
        sequencer.register(StreamId::new(2), feed(2, 1), seq(100),),
        Err(SequencerError::StreamCapacityExceeded),
    );
}
