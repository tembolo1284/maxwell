//! Bounded sparse journal indexing.

use crate::config::{IndexConfig, JournalMode};
use crate::error::{ReplayError, ReplayResult};
use crate::format::JournalHeader;
use crate::reader::JournalReader;
use crate::record::JournalRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub ordinal: u64,
    pub offset: u64,
    pub sequence: u64,
    pub timestamp_ns: u64,
}

#[derive(Debug)]
pub struct JournalIndex {
    header: JournalHeader,
    entries: Vec<IndexEntry>,
    total_records: u64,
    complete: bool,
}

impl JournalIndex {
    pub fn build(reader: &mut JournalReader, config: IndexConfig) -> ReplayResult<Self> {
        config.validate()?;
        reader.rewind()?;

        let mut entries = Vec::new();

        if config.enabled {
            entries.try_reserve_exact(config.max_entries).map_err(|_| {
                ReplayError::AllocationFailed {
                    resource: "journal index",
                    requested: config.max_entries,
                }
            })?;
        }

        let header = *reader.header();
        let mut ordinal = 0_u64;

        loop {
            let offset = reader.offset();

            let Some(record) = reader.next_record()? else {
                break;
            };

            if config.enabled
                && ordinal.is_multiple_of(config.record_interval)
                && entries.len() < config.max_entries
            {
                entries.push(IndexEntry {
                    ordinal,
                    offset,
                    sequence: record.sequence(),
                    timestamp_ns: record.recording_timestamp_ns,
                });
            }

            ordinal = ordinal
                .checked_add(1)
                .ok_or(ReplayError::ArithmeticOverflow {
                    operation: "index ordinal",
                })?;
        }

        let complete = !reader.incomplete_tail();

        reader.rewind()?;

        Ok(Self {
            header,
            entries,
            total_records: ordinal,
            complete,
        })
    }

    #[must_use]
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }

    #[must_use]
    pub const fn total_records(&self) -> u64 {
        self.total_records
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    fn validate_reader(&self, reader: &JournalReader) -> ReplayResult<()> {
        if self.header != *reader.header() {
            return Err(ReplayError::InvalidIndex {
                reason: "index belongs to another journal",
            });
        }

        Ok(())
    }

    fn position_reader(
        &self,
        reader: &mut JournalReader,
        entry: Option<&IndexEntry>,
    ) -> ReplayResult<()> {
        self.validate_reader(reader)?;

        match entry {
            Some(entry) => reader.seek_indexed(entry.offset, entry.ordinal),
            None => reader.rewind(),
        }
    }

    /// Returns the first matching sequence in journal order.
    ///
    /// The returned record has been consumed from the reader.
    pub fn seek_sequence(
        &self,
        reader: &mut JournalReader,
        target: u64,
    ) -> ReplayResult<JournalRecord> {
        let entry = if self.header.mode == JournalMode::Authoritative {
            let position = self
                .entries
                .partition_point(|entry| entry.sequence <= target);

            position
                .checked_sub(1)
                .and_then(|index| self.entries.get(index))
        } else {
            None
        };

        self.position_reader(reader, entry)?;

        while let Some(record) = reader.next_record()? {
            if record.sequence() == target {
                return Ok(record);
            }

            if self.header.mode == JournalMode::Authoritative && record.sequence() > target {
                break;
            }
        }

        Err(ReplayError::SeekTargetNotFound {
            target,
            kind: "sequence",
        })
    }

    /// Returns the first record whose timestamp is at least target.
    ///
    /// The returned record has been consumed from the reader.
    pub fn seek_timestamp(
        &self,
        reader: &mut JournalReader,
        target: u64,
    ) -> ReplayResult<JournalRecord> {
        // Strict comparison is required because multiple
        // records may share the same recording timestamp.
        let position = self
            .entries
            .partition_point(|entry| entry.timestamp_ns < target);

        let entry = position
            .checked_sub(1)
            .and_then(|index| self.entries.get(index));

        self.position_reader(reader, entry)?;

        while let Some(record) = reader.next_record()? {
            if record.recording_timestamp_ns >= target {
                return Ok(record);
            }
        }

        Err(ReplayError::SeekTargetNotFound {
            target,
            kind: "timestamp",
        })
    }
}
