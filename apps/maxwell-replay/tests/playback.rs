//! Replay scheduling, indexing, filtering and backpressure tests.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, InstrumentId, MarketEvent, OrderAdded, OrderId,
    Price, Quantity, SequenceNumber, SessionId, Side, StreamId,
};

use maxwell_replay::{
    IndexConfig, JournalIndex, JournalReader, JournalRecord, JournalWriter, PlaybackMode,
    ReaderConfig, ReplayConfig, ReplayEngine, ReplayFilter, ReplayStatus, SinkStatus, WriterConfig,
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(count: u64) -> Self {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);

        let path =
            std::env::temp_dir().join(format!("maxwell-playback-{}-{id}.mxj", std::process::id(),));

        let feed = FeedId::new(StreamId::new(1), SessionId::new(1));

        let mut writer = JournalWriter::create(&path, WriterConfig::default(), feed, 1, 0).unwrap();

        for sequence in 1..=count {
            let event = EventEnvelope::new(
                EventHeader::new(
                    EventId::new(feed, SequenceNumber::new(sequence)),
                    InstrumentId::new(42),
                    None,
                ),
                MarketEvent::OrderAdded(OrderAdded {
                    order_id: OrderId::new(sequence),
                    price: Price::new(10_000),
                    quantity: Quantity::new(50),
                    side: Side::Buy,
                }),
            );

            writer
                .append(JournalRecord::new(sequence * 100, event))
                .unwrap();
        }

        writer.finish(false).unwrap();

        Self { path }
    }

    fn engine(&self, config: ReplayConfig, filter: ReplayFilter) -> ReplayEngine {
        ReplayEngine::open(&self.path, config, filter).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[test]
fn complete_virtual_playback() {
    let fixture = Fixture::new(100);

    let config = ReplayConfig {
        playback_mode: PlaybackMode::Virtual,
        max_records_per_step: 7,
        ..ReplayConfig::default()
    };

    let mut engine = fixture.engine(config, ReplayFilter::default());

    let mut delivered = Vec::new();

    let mut sink = |record: &JournalRecord| {
        delivered.push(record.sequence());
        Ok(SinkStatus::Accepted)
    };

    loop {
        match engine.step(&mut sink).unwrap() {
            ReplayStatus::Progress => {}
            ReplayStatus::Finished => break,
            status => panic!("unexpected status: {status:?}"),
        }
    }

    assert_eq!(delivered, (1..=100).collect::<Vec<_>>());
    assert_eq!(engine.stats().records_delivered, 100);
}

#[test]
fn backpressure_preserves_pending_record() {
    let fixture = Fixture::new(3);

    let mut engine = fixture.engine(ReplayConfig::default(), ReplayFilter::default());

    let mut blocked = |_record: &JournalRecord| Ok(SinkStatus::Backpressured);

    for _ in 0..5 {
        assert_eq!(
            engine.step(&mut blocked).unwrap(),
            ReplayStatus::Backpressured
        );

        assert_eq!(engine.pending().unwrap().sequence(), 1);
        assert_eq!(engine.stats().records_delivered, 0);
        assert_eq!(engine.stats().records_read, 1);
    }

    let mut delivered = Vec::new();

    let mut sink = |record: &JournalRecord| {
        delivered.push(record.sequence());
        Ok(SinkStatus::Accepted)
    };

    assert_eq!(engine.step(&mut sink).unwrap(), ReplayStatus::Finished);

    assert_eq!(delivered, vec![1, 2, 3]);
    assert_eq!(engine.stats().backpressure_attempts, 5);
}

#[test]
fn backpressure_budget_can_be_reset() {
    let fixture = Fixture::new(1);

    let config = ReplayConfig {
        max_backpressure_attempts: 2,
        ..ReplayConfig::default()
    };

    let mut engine = fixture.engine(config, ReplayFilter::default());

    let mut blocked = |_record: &JournalRecord| Ok(SinkStatus::Backpressured);

    assert_eq!(
        engine.step(&mut blocked).unwrap(),
        ReplayStatus::Backpressured
    );

    assert_eq!(
        engine.step(&mut blocked).unwrap(),
        ReplayStatus::BackpressureBudgetExhausted
    );

    assert_eq!(engine.pending().unwrap().sequence(), 1);

    engine.reset_backpressure_budget();

    let mut accepted = |_record: &JournalRecord| Ok(SinkStatus::Accepted);

    assert_eq!(engine.step(&mut accepted).unwrap(), ReplayStatus::Finished);

    assert_eq!(engine.stats().records_delivered, 1);
}

#[test]
fn filtering_preserves_original_sequences() {
    let fixture = Fixture::new(10);

    let filter = ReplayFilter {
        first_sequence: Some(4),
        last_sequence: Some(6),
        ..ReplayFilter::default()
    };

    let mut engine = fixture.engine(ReplayConfig::default(), filter);

    assert!(!engine.preserves_authoritative_stream());

    let mut delivered = Vec::new();

    let mut sink = |record: &JournalRecord| {
        delivered.push(record.sequence());
        Ok(SinkStatus::Accepted)
    };

    assert_eq!(engine.step(&mut sink).unwrap(), ReplayStatus::Finished);

    assert_eq!(delivered, vec![4, 5, 6]);
    assert_eq!(engine.stats().records_filtered, 7);
}

#[test]
fn sparse_index_sequence_seek() {
    let fixture = Fixture::new(100);

    let mut reader = JournalReader::open(&fixture.path, ReaderConfig::default()).unwrap();

    let config = IndexConfig {
        record_interval: 10,
        max_entries: 3,
        ..IndexConfig::default()
    };

    let index = JournalIndex::build(&mut reader, config).unwrap();

    assert_eq!(index.entries().len(), 3);
    assert_eq!(index.total_records(), 100);

    let record = index.seek_sequence(&mut reader, 95).unwrap();

    assert_eq!(record.sequence(), 95);
}

#[test]
fn sparse_index_timestamp_seek() {
    let fixture = Fixture::new(100);

    let mut reader = JournalReader::open(&fixture.path, ReaderConfig::default()).unwrap();

    let index = JournalIndex::build(&mut reader, IndexConfig::default()).unwrap();

    let record = index.seek_timestamp(&mut reader, 5_050).unwrap();

    assert_eq!(record.sequence(), 51);
}

#[test]
fn seeked_record_is_delivered() {
    let fixture = Fixture::new(10);

    let mut engine = fixture.engine(ReplayConfig::default(), ReplayFilter::default());

    engine.seek_sequence(8).unwrap();

    assert!(!engine.preserves_authoritative_stream());

    let mut delivered = Vec::new();

    let mut sink = |record: &JournalRecord| {
        delivered.push(record.sequence());
        Ok(SinkStatus::Accepted)
    };

    assert_eq!(engine.step(&mut sink).unwrap(), ReplayStatus::Finished);

    assert_eq!(delivered, vec![8, 9, 10]);
}

#[test]
fn rewind_restores_original_stream() {
    let fixture = Fixture::new(10);

    let mut engine = fixture.engine(ReplayConfig::default(), ReplayFilter::default());

    engine.seek_sequence(8).unwrap();
    engine.rewind().unwrap();

    assert!(engine.preserves_authoritative_stream());

    let mut delivered = Vec::new();

    let mut sink = |record: &JournalRecord| {
        delivered.push(record.sequence());
        Ok(SinkStatus::Accepted)
    };

    assert_eq!(engine.step(&mut sink).unwrap(), ReplayStatus::Finished);

    assert_eq!(delivered, (1..=10).collect::<Vec<_>>());
}
