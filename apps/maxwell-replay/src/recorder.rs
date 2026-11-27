//! Market data recording orchestration.

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use maxwell_core::{EventEnvelope, FeedId};

use crate::config::WriterConfig;
use crate::error::{ReplayError, ReplayResult};
use crate::format::JournalHeader;
use crate::record::JournalRecord;
use crate::stats::RecordingStats;
use crate::writer::JournalWriter;

pub struct Recorder {
    writer: JournalWriter,
    origin: Instant,
}

impl Recorder {
    pub fn create(
        path: impl AsRef<Path>,
        config: WriterConfig,
        feed: FeedId,
        recording_id: u64,
    ) -> ReplayResult<Self> {
        let origin = Instant::now();

        let created_at_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ReplayError::ClockOverflow)?
            .as_nanos();

        let created_at_ns = u64::try_from(created_at_ns).map_err(|_| ReplayError::ClockOverflow)?;

        let writer = JournalWriter::create(path, config, feed, recording_id, created_at_ns)?;

        Ok(Self { writer, origin })
    }

    #[must_use]
    pub fn from_writer(writer: JournalWriter) -> Self {
        Self {
            writer,
            origin: Instant::now(),
        }
    }

    #[must_use]
    pub const fn header(&self) -> &JournalHeader {
        self.writer.header()
    }

    #[must_use]
    pub const fn stats(&self) -> &RecordingStats {
        self.writer.stats()
    }

    /// Records an event using the recorder's local monotonic clock.
    pub fn record(&mut self, event: EventEnvelope) -> ReplayResult<()> {
        let elapsed = self.origin.elapsed().as_nanos();

        let timestamp_ns = u64::try_from(elapsed).map_err(|_| ReplayError::ClockOverflow)?;

        self.record_at(event, timestamp_ns)
    }

    /// Records an event with an explicitly supplied timestamp.
    ///
    /// This method supports deterministic simulator recordings.
    pub fn record_at(&mut self, event: EventEnvelope, timestamp_ns: u64) -> ReplayResult<()> {
        self.writer.append(JournalRecord::new(timestamp_ns, event))
    }

    pub fn flush(&mut self) -> ReplayResult<()> {
        self.writer.flush()
    }

    pub fn sync(&mut self) -> ReplayResult<()> {
        self.writer.sync()
    }

    pub fn finish(self, durable: bool) -> ReplayResult<RecordingStats> {
        self.writer.finish(durable)
    }
}
