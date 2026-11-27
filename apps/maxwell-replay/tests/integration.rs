//! End-to-end journal recording and deterministic replay tests.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use maxwell_core::{
    EventEnvelope,
    EventHeader,
    EventId,
    FeedId,
    InstrumentId,
    MarketEvent,
    OrderAdded,
    OrderId,
    Price,
    Quantity,
    SequenceNumber,
    SessionId,
    Side,
    StreamId,
};

use maxwell_replay::{
    DurabilityPolicy,
    JournalRecord,
    PlaybackMode,
    Recorder,
    ReplayConfig,
    ReplayEngine,
    ReplayFilter,
    ReplayStatus,
    SinkStatus,
    WriterConfig,
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct JournalFile(PathBuf);

impl JournalFile {
    fn new() -> Self {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);

        Self(std::env::temp_dir().join(format!(
            "maxwell-integration-{}-{id}.mxj",
            std::process::id(),
        )))
    }
}

impl Drop for JournalFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn feed() -> FeedId {
    FeedId::new(
        StreamId::new(1),
        SessionId::new(100),
    )
}

fn event(sequence: u64) -> EventEnvelope {
    let side = if sequence % 2 == 0 {
        Side::Buy
    } else {
        Side::Sell
    };

    EventEnvelope::new(
        EventHeader::new(
            EventId::new(
                feed(),
                SequenceNumber::new(sequence),
            ),
            InstrumentId::new(
                (sequence % 4) as u32 + 1,
            ),
            None,
        ),
        MarketEvent::OrderAdded(OrderAdded {
            order_id: OrderId::new(sequence),
            price: Price::new(
                10_000 + (sequence % 100) as i64,
            ),
            quantity: Quantity::new(
                1 + sequence % 50,
            ),
            side,
        }),
    )
}

fn record_workload(
    file: &JournalFile,
    count: u64,
) {
    let config = WriterConfig {
        durability: DurabilityPolicy::FlushEveryRecords(1_024),
        ..WriterConfig::default()
    };

    let mut recorder = Recorder::create(
        &file.0,
        config,
        feed(),
        42,
    )
    .unwrap();

    for sequence in 1..=count {
        recorder.record_at(
            event(sequence),
            sequence * 100,
        )
        .unwrap();
    }

    let stats = recorder.finish(false).unwrap();

    assert_eq!(stats.records_written, count);
    assert_eq!(stats.first_sequence, Some(1));
    assert_eq!(stats.last_sequence, Some(count));
}

#[test]
fn large_workload_survives_close_and_reopen() {
    const COUNT: u64 = 100_000;

    let file = JournalFile::new();

    record_workload(&file, COUNT);

    let config = ReplayConfig {
        playback_mode: PlaybackMode::Virtual,
        max_records_per_step: 256,
        ..ReplayConfig::default()
    };

    let mut engine = ReplayEngine::open(
        &file.0,
        config,
        ReplayFilter::default(),
    )
    .unwrap();

    let mut expected = 1_u64;

    let mut sink = |record: &JournalRecord| {
        assert_eq!(record.sequence(), expected);
        assert_eq!(record.event, event(expected));

        assert_eq!(
            record.recording_timestamp_ns,
            expected * 100
        );

        expected += 1;

        Ok(SinkStatus::Accepted)
    };

    loop {
        match engine.step(&mut sink).unwrap() {
            ReplayStatus::Progress => {}
            ReplayStatus::Finished => break,
            status => panic!("unexpected status: {status:?}"),
        }
    }

    assert_eq!(expected, COUNT + 1);
    assert_eq!(engine.stats().records_delivered, COUNT);
    assert_eq!(engine.stats().records_filtered, 0);
    assert_eq!(engine.stats().last_delivered_sequence, Some(COUNT));
}

#[test]
fn sustained_backpressure_does_not_duplicate_delivery() {
    const COUNT: u64 = 10_000;

    let file = JournalFile::new();

    record_workload(&file, COUNT);

    let mut engine = ReplayEngine::open(
        &file.0,
        ReplayConfig::default(),
        ReplayFilter::default(),
    )
    .unwrap();

    let mut expected = 1_u64;
    let mut previously_blocked = false;

    let mut sink = |record: &JournalRecord| {
        assert_eq!(record.sequence(), expected);

        if !previously_blocked {
            previously_blocked = true;

            return Ok(SinkStatus::Backpressured);
        }

        previously_blocked = false;

        assert_eq!(record.event, event(expected));

        expected += 1;

        Ok(SinkStatus::Accepted)
    };

    loop {
        match engine.step(&mut sink).unwrap() {
            ReplayStatus::Progress
            | ReplayStatus::Backpressured => {}

            ReplayStatus::Finished => break,

            status => panic!("unexpected status: {status:?}"),
        }
    }

    assert_eq!(expected, COUNT + 1);
    assert_eq!(engine.stats().records_read, COUNT);
    assert_eq!(engine.stats().records_delivered, COUNT);
    assert_eq!(engine.stats().backpressure_attempts, COUNT);
}

#[test]
fn repeated_playback_is_deterministic() {
    const COUNT: u64 = 1_000;

    let file = JournalFile::new();

    record_workload(&file, COUNT);

    let mut engine = ReplayEngine::open(
        &file.0,
        ReplayConfig::default(),
        ReplayFilter::default(),
    )
    .unwrap();

    for _ in 0..3 {
        let mut expected = 1_u64;

        let mut sink = |record: &JournalRecord| {
            assert_eq!(record.event, event(expected));

            expected += 1;

            Ok(SinkStatus::Accepted)
        };

        loop {
            match engine.step(&mut sink).unwrap() {
                ReplayStatus::Progress => {}
                ReplayStatus::Finished => break,
                status => panic!("unexpected status: {status:?}"),
            }
        }

        assert_eq!(expected, COUNT + 1);
        assert_eq!(engine.stats().records_delivered, COUNT);

        engine.rewind().unwrap();
    }
}
