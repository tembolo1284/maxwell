//! Journal persistence and corruption tests.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, InstrumentId, MarketEvent, OrderAdded, OrderId,
    Price, Quantity, SequenceNumber, SessionId, Side, StreamId,
};

use maxwell_replay::{
    JournalConfig, JournalMode, JournalReader, JournalRecord, JournalWriter, ReaderConfig,
    ReplayError, TailPolicy, WriterConfig,
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct TestFile(PathBuf);

impl TestFile {
    fn new() -> Self {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);

        Self(std::env::temp_dir().join(format!("maxwell-journal-{}-{id}.mxj", std::process::id(),)))
    }
}

impl Drop for TestFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(1))
}

fn event(sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(), SequenceNumber::new(sequence)),
            InstrumentId::new(42),
            None,
        ),
        MarketEvent::OrderAdded(OrderAdded {
            order_id: OrderId::new(sequence),
            price: Price::new(10_000),
            quantity: Quantity::new(50),
            side: Side::Buy,
        }),
    )
}

fn writer(file: &TestFile) -> JournalWriter {
    JournalWriter::create(&file.0, WriterConfig::default(), feed(), 1, 0).unwrap()
}

fn write_records(file: &TestFile, count: u64) {
    let mut writer = writer(file);

    for sequence in 1..=count {
        writer
            .append(JournalRecord::new(sequence * 100, event(sequence)))
            .unwrap();
    }

    writer.finish(false).unwrap();
}

#[test]
fn empty_journal() {
    let file = TestFile::new();

    writer(&file).finish(false).unwrap();

    let mut reader = JournalReader::open(&file.0, ReaderConfig::default()).unwrap();

    assert!(reader.next_record().unwrap().is_none());
    assert_eq!(reader.records_read(), 0);
}

#[test]
fn journal_round_trip() {
    let file = TestFile::new();

    write_records(&file, 100);

    let mut reader = JournalReader::open(&file.0, ReaderConfig::default()).unwrap();

    for sequence in 1..=100 {
        let record = reader.next_record().unwrap().unwrap();

        assert_eq!(record.event, event(sequence));
        assert_eq!(record.recording_timestamp_ns, sequence * 100);
    }

    assert!(reader.next_record().unwrap().is_none());
    assert_eq!(reader.records_read(), 100);
}

#[test]
fn authoritative_gap_is_rejected() {
    let file = TestFile::new();
    let mut writer = writer(&file);

    writer.append(JournalRecord::new(100, event(1))).unwrap();

    assert!(matches!(
        writer.append(JournalRecord::new(200, event(3))),
        Err(ReplayError::SequenceMismatch { .. })
    ));

    writer.append(JournalRecord::new(200, event(2))).unwrap();

    assert_eq!(writer.stats().records_written, 2);

    writer.finish(false).unwrap();
}

#[test]
fn timestamp_regression_is_rejected() {
    let file = TestFile::new();
    let mut writer = writer(&file);

    writer.append(JournalRecord::new(100, event(1))).unwrap();

    assert!(matches!(
        writer.append(JournalRecord::new(99, event(2))),
        Err(ReplayError::TimestampRegression { .. })
    ));
}

#[test]
fn raw_capture_preserves_arrival_order() {
    let file = TestFile::new();

    let config = WriterConfig {
        journal: JournalConfig {
            mode: JournalMode::RawCapture,
            ..JournalConfig::default()
        },
        ..WriterConfig::default()
    };

    let mut writer = JournalWriter::create(&file.0, config, feed(), 1, 0).unwrap();

    let sequences = [1, 3, 2, 2, 7];

    for (index, sequence) in sequences.into_iter().enumerate() {
        writer
            .append(JournalRecord::new(index as u64, event(sequence)))
            .unwrap();
    }

    writer.finish(false).unwrap();

    let mut reader = JournalReader::open(&file.0, ReaderConfig::default()).unwrap();

    for sequence in sequences {
        assert_eq!(reader.next_record().unwrap().unwrap().sequence(), sequence);
    }

    assert!(reader.next_record().unwrap().is_none());
}

#[test]
fn corrupted_record_is_rejected() {
    let file = TestFile::new();

    write_records(&file, 1);

    let mut output = OpenOptions::new().write(true).open(&file.0).unwrap();

    output.seek(SeekFrom::End(-1)).unwrap();
    output.write_all(&[0xFF]).unwrap();
    output.flush().unwrap();

    let mut reader = JournalReader::open(&file.0, ReaderConfig::default()).unwrap();

    assert!(matches!(
        reader.next_record(),
        Err(ReplayError::ChecksumMismatch { .. })
    ));
}

#[test]
fn strict_reader_rejects_incomplete_tail() {
    let file = TestFile::new();

    write_records(&file, 2);

    let output = OpenOptions::new().write(true).open(&file.0).unwrap();

    let length = output.metadata().unwrap().len();

    output.set_len(length - 1).unwrap();

    let mut reader = JournalReader::open(&file.0, ReaderConfig::default()).unwrap();

    assert!(reader.next_record().unwrap().is_some());

    assert!(matches!(
        reader.next_record(),
        Err(ReplayError::TruncatedRecord { .. })
    ));
}

#[test]
fn tolerant_reader_stops_at_incomplete_tail() {
    let file = TestFile::new();

    write_records(&file, 2);

    let output = OpenOptions::new().write(true).open(&file.0).unwrap();

    let length = output.metadata().unwrap().len();

    output.set_len(length - 1).unwrap();

    let config = ReaderConfig {
        tail_policy: TailPolicy::AllowIncompleteTail,
        ..ReaderConfig::default()
    };

    let mut reader = JournalReader::open(&file.0, config).unwrap();

    assert!(reader.next_record().unwrap().is_some());
    assert!(reader.next_record().unwrap().is_none());
    assert!(reader.incomplete_tail());
    assert_eq!(reader.records_read(), 1);
}

#[test]
fn existing_journal_is_not_overwritten() {
    let file = TestFile::new();

    write_records(&file, 1);

    assert!(JournalWriter::create(&file.0, WriterConfig::default(), feed(), 2, 0,).is_err());
}

#[test]
fn reader_can_rewind() {
    let file = TestFile::new();

    write_records(&file, 2);

    let mut reader = JournalReader::open(&file.0, ReaderConfig::default()).unwrap();

    assert_eq!(reader.next_record().unwrap().unwrap().sequence(), 1);

    reader.rewind().unwrap();

    assert_eq!(reader.next_record().unwrap().unwrap().sequence(), 1);
}
