//! Bounded historical market data playback.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::clock::ReplayClock;
use crate::config::{JournalMode, ReplayConfig};
use crate::error::{ReplayError, ReplayResult};
use crate::filter::ReplayFilter;
use crate::index::JournalIndex;
use crate::reader::JournalReader;
use crate::record::JournalRecord;
use crate::stats::ReplayStats;

// ============================================================
// Downstream sink
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkStatus {
    Accepted,
    Backpressured,
}

/// A replay consumer.
///
/// Returning Backpressured guarantees that the record has not
/// been applied. Returning Accepted commits the delivery.
///
/// Errors are terminal for the current replay engine.
pub trait ReplaySink {
    fn deliver(
        &mut self,
        record: &JournalRecord,
    ) -> ReplayResult<SinkStatus>;
}

impl<F> ReplaySink for F
where
    F: FnMut(&JournalRecord) -> ReplayResult<SinkStatus>,
{
    fn deliver(
        &mut self,
        record: &JournalRecord,
    ) -> ReplayResult<SinkStatus> {
        self(record)
    }
}

// ============================================================
// Replay status
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayStatus {
    Progress,
    Waiting(Duration),
    Backpressured,
    BackpressureBudgetExhausted,
    Finished,
}

// ============================================================
// Replay engine
// ============================================================

pub struct ReplayEngine {
    config: ReplayConfig,
    reader: JournalReader,
    index: Option<JournalIndex>,
    clock: ReplayClock,
    filter: ReplayFilter,
    pending: Option<JournalRecord>,
    stats: ReplayStats,
    consecutive_backpressure: u64,
    finished: bool,
    failed: bool,
    seeked: bool,
}

impl ReplayEngine {
    pub fn open(
        path: impl AsRef<Path>,
        config: ReplayConfig,
        filter: ReplayFilter,
    ) -> ReplayResult<Self> {
        config.validate()?;
        filter.validate()?;

        let mut reader =
            JournalReader::open(path, config.reader)?;

        let index = if config.index.enabled {
            Some(JournalIndex::build(
                &mut reader,
                config.index,
            )?)
        } else {
            None
        };

        Ok(Self {
            config,
            reader,
            index,
            clock: ReplayClock::new(config.playback_mode)?,
            filter,
            pending: None,
            stats: ReplayStats::default(),
            consecutive_backpressure: 0,
            finished: false,
            failed: false,
            seeked: false,
        })
    }

    #[must_use]
    pub const fn stats(&self) -> &ReplayStats {
        &self.stats
    }

    #[must_use]
    pub const fn pending(&self) -> Option<&JournalRecord> {
        self.pending.as_ref()
    }

    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.finished
    }

    #[must_use]
    pub const fn is_failed(&self) -> bool {
        self.failed
    }

    #[must_use]
    pub const fn incomplete_tail(&self) -> bool {
        self.reader.incomplete_tail()
    }

    /// Reports source continuity, not reconstruction readiness.
    ///
    /// A compatible order book baseline is still required.
    #[must_use]
    pub fn preserves_authoritative_stream(&self) -> bool {
        self.reader.header().mode == JournalMode::Authoritative
            && self.filter.is_unrestricted()
            && !self.seeked
    }

    pub fn require_authoritative_stream(
        &self,
    ) -> ReplayResult<()> {
        if !self.preserves_authoritative_stream() {
            return Err(ReplayError::NonAuthoritativeReplay {
                reason: "raw, filtered, or seeked playback",
            });
        }

        Ok(())
    }

    pub fn reset_backpressure_budget(&mut self) {
        self.consecutive_backpressure = 0;
    }

    pub fn rewind(&mut self) -> ReplayResult<()> {
        self.reader.rewind()?;

        self.reset_playback_state();
        self.seeked = false;

        Ok(())
    }

    fn reset_playback_state(&mut self) {
        self.clock.reset();
        self.pending = None;
        self.stats = ReplayStats::default();
        self.consecutive_backpressure = 0;
        self.finished = false;
        self.failed = false;
    }

    fn ensure_seekable(&self) -> ReplayResult<()> {
        if self.pending.is_some() {
            return Err(ReplayError::InvalidState {
                operation: "seek",
                state: "pending record",
            });
        }

        Ok(())
    }

    pub fn seek_sequence(
        &mut self,
        sequence: u64,
    ) -> ReplayResult<()> {
        self.ensure_seekable()?;

        let index = self.index.as_ref().ok_or(
            ReplayError::UnsupportedSeek {
                reason: "indexing is disabled",
            },
        )?;

        let result = index.seek_sequence(
            &mut self.reader,
            sequence,
        );

        self.install_seek_result(result)
    }

    pub fn seek_timestamp(
        &mut self,
        timestamp_ns: u64,
    ) -> ReplayResult<()> {
        self.ensure_seekable()?;

        let index = self.index.as_ref().ok_or(
            ReplayError::UnsupportedSeek {
                reason: "indexing is disabled",
            },
        )?;

        let result = index.seek_timestamp(
            &mut self.reader,
            timestamp_ns,
        );

        self.install_seek_result(result)
    }

    fn install_seek_result(
        &mut self,
        result: ReplayResult<JournalRecord>,
    ) -> ReplayResult<()> {
        match result {
            Ok(record) => {
                self.reset_playback_state();

                self.pending = Some(record);
                self.stats.records_read = 1;
                self.seeked = true;

                Ok(())
            }

            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    pub fn step(
        &mut self,
        sink: &mut impl ReplaySink,
    ) -> ReplayResult<ReplayStatus> {
        if self.failed {
            return Err(ReplayError::InvalidState {
                operation: "replay step",
                state: "failed",
            });
        }

        if self.finished {
            return Ok(ReplayStatus::Finished);
        }

        let result = self.process_step(sink);

        if result.is_err() {
            self.failed = true;
        }

        result
    }

    fn process_step(
        &mut self,
        sink: &mut impl ReplaySink,
    ) -> ReplayResult<ReplayStatus> {
        for _ in 0..self.config.max_records_per_step {
            if self.pending.is_none() {
                let Some(record) = self.reader.next_record()? else {
                    self.finished = true;
                    return Ok(ReplayStatus::Finished);
                };

                self.stats.records_read =
                    self.stats.records_read.saturating_add(1);

                self.pending = Some(record);
            }

            let record = self.pending.expect(
                "pending record established before processing",
            );

            if !self.filter.matches(record) {
                self.stats.records_filtered =
                    self.stats.records_filtered.saturating_add(1);

                self.pending = None;

                continue;
            }

            let delay = self.clock.remaining_delay(
                record.recording_timestamp_ns,
                Instant::now(),
            )?;

            if !delay.is_zero() {
                return Ok(ReplayStatus::Waiting(delay));
            }

            if self.consecutive_backpressure
                >= self.config.max_backpressure_attempts
            {
                return Ok(
                    ReplayStatus::BackpressureBudgetExhausted,
                );
            }

            match sink.deliver(&record)? {
                SinkStatus::Accepted => {
                    self.clock.commit(
                        record.recording_timestamp_ns,
                    )?;

                    self.stats.delivered(record);
                    self.pending = None;
                    self.consecutive_backpressure = 0;
                }

                SinkStatus::Backpressured => {
                    self.consecutive_backpressure =
                        self.consecutive_backpressure
                            .saturating_add(1);

                    self.stats.backpressure_attempts =
                        self.stats.backpressure_attempts
                            .saturating_add(1);

                    if self.consecutive_backpressure
                        >= self.config.max_backpressure_attempts
                    {
                        return Ok(
                            ReplayStatus::BackpressureBudgetExhausted,
                        );
                    }

                    return Ok(ReplayStatus::Backpressured);
                }
            }
        }

        Ok(ReplayStatus::Progress)
    }
}
