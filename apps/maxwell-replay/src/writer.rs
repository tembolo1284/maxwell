//! Buffered append-only journal writer.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

use maxwell_core::FeedId;
use maxwell_protocol::{Encoder, ProtocolMessage};

use crate::config::{DurabilityPolicy, WriterConfig};
use crate::error::{ReplayError, ReplayResult};
use crate::format::{
    Crc32c, HEADER_SIZE, JournalHeader, RECORD_OVERHEAD,
};
use crate::record::{JournalRecord, StreamValidator};
use crate::stats::RecordingStats;

pub struct JournalWriter {
    output: BufWriter<File>,
    config: WriterConfig,
    header: JournalHeader,
    validator: StreamValidator,
    payload: Vec<u8>,
    stats: RecordingStats,
    poisoned: bool,
}

impl JournalWriter {
    pub fn create(
        path: impl AsRef<Path>,
        config: WriterConfig,
        feed: FeedId,
        recording_id: u64,
        created_at_ns: u64,
    ) -> ReplayResult<Self> {
        config.validate()?;

        let header = JournalHeader {
            mode: config.journal.mode,
            created_at_ns,
            max_payload_bytes:
                config.journal.max_payload_bytes as u32,
            feed,
            recording_id,
        };

        let encoded_header = header.encode()?;

        let mut payload = Vec::new();

        payload
            .try_reserve_exact(config.journal.max_payload_bytes)
            .map_err(|_| ReplayError::AllocationFailed {
                resource: "journal encoding buffer",
                requested: config.journal.max_payload_bytes,
            })?;

        payload.resize(config.journal.max_payload_bytes, 0);

        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;

        let mut output =
            BufWriter::with_capacity(config.buffer_bytes, file);

        output.write_all(&encoded_header)?;

        let stats = RecordingStats {
            bytes_written: HEADER_SIZE as u64,
            ..RecordingStats::default()
        };

        Ok(Self {
            output,
            config,
            header,
            validator: StreamValidator::default(),
            payload,
            stats,
            poisoned: false,
        })
    }

    #[must_use]
    pub const fn header(&self) -> &JournalHeader {
        &self.header
    }

    #[must_use]
    pub const fn stats(&self) -> &RecordingStats {
        &self.stats
    }

    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    fn ensure_healthy(
        &self,
        operation: &'static str,
    ) -> ReplayResult<()> {
        if self.poisoned {
            return Err(ReplayError::InvalidState {
                operation,
                state: "poisoned",
            });
        }

        Ok(())
    }

    pub fn append(
        &mut self,
        record: JournalRecord,
    ) -> ReplayResult<()> {
        self.ensure_healthy("append")?;
        self.validator.validate(&self.header, record)?;

        let required = Encoder
            .required_size(ProtocolMessage::current(record.event))
            .map_err(|error| ReplayError::EncodingFailed {
                reason: error.to_string(),
            })?;

        if required > self.payload.len() {
            return Err(ReplayError::PayloadTooLarge {
                actual: required,
                maximum: self.payload.len(),
            });
        }

        let written = record.encode_payload(&mut self.payload)?;

        let length = u32::try_from(written + RECORD_OVERHEAD)
            .map_err(|_| ReplayError::ArithmeticOverflow {
                operation: "record length",
            })?;

        let length_bytes = length.to_le_bytes();
        let sequence_bytes = record.sequence().to_le_bytes();

        let timestamp_bytes =
            record.recording_timestamp_ns.to_le_bytes();

        let payload = &self.payload[..written];

        let mut checksum = Crc32c::new();

        checksum.update(&length_bytes);
        checksum.update(&sequence_bytes);
        checksum.update(&timestamp_bytes);
        checksum.update(payload);

        let checksum_bytes = checksum.finish().to_le_bytes();

        let result = (|| -> std::io::Result<()> {
            self.output.write_all(&length_bytes)?;
            self.output.write_all(&sequence_bytes)?;
            self.output.write_all(&timestamp_bytes)?;
            self.output.write_all(payload)?;
            self.output.write_all(&checksum_bytes)?;

            Ok(())
        })();

        if let Err(error) = result {
            self.poisoned = true;
            return Err(error.into());
        }

        self.validator.commit(record);

        self.stats.record_written(
            record,
            u64::from(length) + 4,
        );

        self.checkpoint()
    }

    fn checkpoint(&mut self) -> ReplayResult<()> {
        let count = self.stats.records_written;

        match self.config.durability {
            DurabilityPolicy::Buffered => Ok(()),

            DurabilityPolicy::FlushEveryRecord => self.flush(),

            DurabilityPolicy::SyncEveryRecord => self.sync(),

            DurabilityPolicy::FlushEveryRecords(interval) => {
                if count % interval == 0 {
                    self.flush()?;
                }

                Ok(())
            }

            DurabilityPolicy::SyncEveryRecords(interval) => {
                if count % interval == 0 {
                    self.sync()?;
                }

                Ok(())
            }
        }
    }

    pub fn flush(&mut self) -> ReplayResult<()> {
        self.ensure_healthy("flush")?;

        if let Err(error) = self.output.flush() {
            self.poisoned = true;
            return Err(error.into());
        }

        self.stats.flushes =
            self.stats.flushes.saturating_add(1);

        Ok(())
    }

    pub fn sync(&mut self) -> ReplayResult<()> {
        self.flush()?;

        if let Err(error) = self.output.get_ref().sync_data() {
            self.poisoned = true;
            return Err(error.into());
        }

        self.stats.synchronizations =
            self.stats.synchronizations.saturating_add(1);

        Ok(())
    }

    /// Finalizes the journal.
    ///
    /// If durable is true, file data is synchronized.
    /// Directory-entry durability is not guaranteed.
    pub fn finish(
        mut self,
        durable: bool,
    ) -> ReplayResult<RecordingStats> {
        if durable {
            self.sync()?;
        } else {
            self.flush()?;
        }

        Ok(self.stats)
    }
}
