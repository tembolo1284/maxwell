
//! Deterministic high-volume sequencing stress tests.
//!
//! These tests verify correctness under load.
//! They are not substitutes for dedicated benchmarks.

use maxwell_core::*;

use maxwell_sequencer::{
    SequencerConfig,
    SequencerState,
    StreamSequencer,
};

const EVENT_COUNT: u64 = 1_000_000;

fn seq(value: u64) -> SequenceNumber {
    SequenceNumber::new(value)
}

fn feed() -> FeedId {
    FeedId::new(
        StreamId::new(1),
        SessionId::new(1),
    )
}

fn event(sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(), seq(sequence)),
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

fn stream(capacity: usize) -> StreamSequencer {
    StreamSequencer::new(
        feed(),
        seq(1),
        SequencerConfig::new(capacity, 1),
    )
    .unwrap()
}

// ============================================================
// Deterministic pseudorandom generator
// ============================================================

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    fn next(&mut self) -> u64 {
        let mut value = self.state;

        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;

        self.state = value;

        value
    }

    fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            let selected =
                (self.next() % (index as u64 + 1)) as usize;

            values.swap(index, selected);
        }
    }
}

// ============================================================
// Contiguous event stress
// ============================================================

#[test]
fn one_million_contiguous_events() {
    let mut sequencer = stream(4_096);
    let mut expected = 1_u64;

    for sequence in 1..=EVENT_COUNT {
        sequencer
            .ingest(event(sequence), |value| {
                assert_eq!(
                    value.header.sequence().raw(),
                    expected,
                );

                expected += 1;
            })
            .unwrap();
    }

    assert_eq!(expected, EVENT_COUNT + 1);
    assert_eq!(sequencer.expected(), Some(seq(EVENT_COUNT + 1)));
    assert_eq!(sequencer.metrics().delivered, EVENT_COUNT);
    assert_eq!(sequencer.buffered(), 0);
    assert_eq!(sequencer.state(), SequencerState::Live);
}

// ============================================================
// Reverse-order batch stress
// ============================================================

#[test]
fn reversed_batches() {
    const BATCH_SIZE: u64 = 256;
    const BATCH_COUNT: u64 = 1_000;

    let mut sequencer = stream(512);
    let mut expected = 1_u64;

    for batch in 0..BATCH_COUNT {
        let start = batch * BATCH_SIZE + 1;
        let end = start + BATCH_SIZE;

        for sequence in (start..end).rev() {
            sequencer
                .ingest(event(sequence), |value| {
                    assert_eq!(
                        value.header.sequence().raw(),
                        expected,
                    );

                    expected += 1;
                })
                .unwrap();
        }

        assert_eq!(sequencer.buffered(), 0);
        assert_eq!(sequencer.state(), SequencerState::Live);
    }

    assert_eq!(
        sequencer.metrics().delivered,
        BATCH_COUNT * BATCH_SIZE,
    );
}

// ============================================================
// Randomized reorder stress
// ============================================================

#[test]
fn randomized_batches_with_duplicates() {
    const BATCH_SIZE: usize = 128;
    const BATCH_COUNT: u64 = 2_000;

    let mut sequencer = stream(256);
    let mut rng = XorShift64::new(0x1234_5678_ABCD_EF01);

    let mut expected = 1_u64;
    let mut duplicate_count = 0_u64;

    let mut batch = [0_u64; BATCH_SIZE];

    for batch_index in 0..BATCH_COUNT {
        let start = batch_index * BATCH_SIZE as u64 + 1;

        for (offset, sequence) in batch.iter_mut().enumerate() {
            *sequence = start + offset as u64;
        }

        rng.shuffle(&mut batch);

        for &sequence in &batch {
            sequencer
                .ingest(event(sequence), |value| {
                    assert_eq!(
                        value.header.sequence().raw(),
                        expected,
                    );

                    expected += 1;
                })
                .unwrap();

            // Introduce duplicates at deterministic
            // pseudorandom intervals.
            if rng.next() % 5 == 0 {
                sequencer
                    .ingest(event(sequence), |_| {
                        panic!("duplicate event delivered");
                    })
                    .unwrap();

                duplicate_count += 1;
            }
        }

        assert_eq!(sequencer.buffered(), 0);
        assert_eq!(sequencer.state(), SequencerState::Live);
    }

    let total = BATCH_COUNT * BATCH_SIZE as u64;
    let metrics = sequencer.metrics();

    assert_eq!(metrics.delivered, total);
    assert_eq!(metrics.duplicates, duplicate_count);
    assert_eq!(metrics.received, total + duplicate_count);
    assert_eq!(expected, total + 1);
    assert_eq!(metrics.recovery_requests, 0);
}

// ============================================================
// Circular buffer reuse
// ============================================================

#[test]
fn repeated_circular_slot_reuse() {
    let mut sequencer = stream(8);
    let mut expected = 1_u64;

    for start in (1..100_001).step_by(4) {
        for sequence in [
            start + 3,
            start + 1,
            start + 2,
            start,
        ] {
            sequencer
                .ingest(event(sequence), |value| {
                    assert_eq!(
                        value.header.sequence().raw(),
                        expected,
                    );

                    expected += 1;
                })
                .unwrap();
        }

        assert_eq!(sequencer.buffered(), 0);
    }

    assert_eq!(sequencer.metrics().delivered, 100_000);
    assert_eq!(sequencer.metrics().buffer_high_watermark, 3);
}

// ============================================================
// Deterministic replay
// ============================================================

#[test]
fn identical_input_produces_identical_output() {
    fn run() -> (Vec<u64>, maxwell_sequencer::SequencerMetrics) {
        let mut sequencer = stream(16);
        let mut delivered = Vec::new();

        for sequence in [
            3, 2, 1, 2, 6, 5, 4, 6, 8, 7, 10, 9,
        ] {
            sequencer
                .ingest(event(sequence), |value| {
                    delivered.push(
                        value.header.sequence().raw(),
                    );
                })
                .unwrap();
        }

        (delivered, sequencer.metrics())
    }

    assert_eq!(run(), run());
    assert_eq!(run().0, (1..=10).collect::<Vec<_>>());
}

