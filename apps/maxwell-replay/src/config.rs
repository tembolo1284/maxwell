//! Configuration and operational policies for market data journaling.
//!
//! Configuration is validated before journal readers, writers, indexes,
//! and replay engines allocate resources or begin processing events.

use crate::error::{ReplayError, ReplayResult};

// -----------------------------------------------------------------------------
// Resource limits
// -----------------------------------------------------------------------------

/// Default maximum encoded event payload size.
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 64 * 1024;

/// Absolute maximum encoded event payload size supported by version one.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Default buffered I/O capacity.
pub const DEFAULT_IO_BUFFER_BYTES: usize = 256 * 1024;

/// Absolute maximum buffered I/O capacity.
pub const MAX_IO_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// Default number of records between sparse index entries.
pub const DEFAULT_INDEX_INTERVAL: u64 = 1_024;

/// Default maximum number of retained sparse index entries.
pub const DEFAULT_MAX_INDEX_ENTRIES: usize = 65_536;

/// Absolute maximum number of retained sparse index entries.
pub const MAX_INDEX_ENTRIES: usize = 1_048_576;

/// Default maximum number of records processed per replay step.
pub const DEFAULT_MAX_RECORDS_PER_STEP: usize = 4_096;

/// Absolute maximum number of records processed per replay step.
pub const MAX_RECORDS_PER_STEP: usize = 1_048_576;

/// Default maximum consecutive backpressure attempts.
pub const DEFAULT_MAX_BACKPRESSURE_ATTEMPTS: u64 = 10_000;

// -----------------------------------------------------------------------------
// Journal mode
// -----------------------------------------------------------------------------

/// Determines the sequence validation semantics of a journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalMode {
    /// Events belong to a single contiguous upstream sequence domain.
    ///
    /// The first sequence may be arbitrary. Subsequent sequences must
    /// advance by exactly one.
    #[default]
    Authoritative,

    /// Events are retained in their original arrival order.
    ///
    /// Duplicate sequences, gaps, and reordered sequences are permitted.
    RawCapture,
}

impl JournalMode {
    /// Returns whether contiguous upstream sequences are required.
    #[must_use]
    pub const fn requires_contiguous_sequences(self) -> bool {
        matches!(self, Self::Authoritative)
    }
}

// -----------------------------------------------------------------------------
// Durability policy
// -----------------------------------------------------------------------------

/// Determines when buffered journal writes are flushed or synchronized.
///
/// These policies govern automatic checkpoints after successful record
/// writes. Explicit flush and synchronization operations remain available
/// independently of the configured policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DurabilityPolicy {
    /// Writes are buffered until an explicit flush or buffer exhaustion.
    ///
    /// No durable persistence guarantee is implied.
    #[default]
    Buffered,

    /// Flush the userspace buffer after every successful record write.
    ///
    /// Flushing does not guarantee durable persistence.
    FlushEveryRecord,

    /// Flush after the configured number of successful record writes.
    FlushEveryRecords(u64),

    /// Flush and synchronize file data after every successful record write.
    SyncEveryRecord,

    /// Flush and synchronize file data after the configured number of
    /// successful record writes.
    SyncEveryRecords(u64),
}

impl DurabilityPolicy {
    /// Validates the durability policy.
    pub fn validate(self) -> ReplayResult<()> {
        match self {
            Self::FlushEveryRecords(0) | Self::SyncEveryRecords(0) => {
                Err(ReplayError::InvalidConfig {
                    field: "durability",
                    reason: "record interval must be greater than zero",
                })
            }
            _ => Ok(()),
        }
    }

    /// Returns whether this policy requires synchronization.
    #[must_use]
    pub const fn requires_sync(self) -> bool {
        matches!(
            self,
            Self::SyncEveryRecord | Self::SyncEveryRecords(_)
        )
    }
}

// -----------------------------------------------------------------------------
// Incomplete-tail policy
// -----------------------------------------------------------------------------

/// Determines how readers handle incomplete final records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TailPolicy {
    /// Reject an incomplete final record.
    #[default]
    Strict,

    /// Stop at the last complete, validated record when the final record
    /// is incomplete.
    ///
    /// This policy does not permit invalid checksums, malformed complete
    /// records, invalid headers, or unsupported format versions.
    AllowIncompleteTail,
}

// -----------------------------------------------------------------------------
// Playback mode
// -----------------------------------------------------------------------------

/// Determines how recorded timestamps affect playback scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackMode {
    /// Deliver events as quickly as the downstream consumer permits.
    #[default]
    Unbounded,

    /// Advance a deterministic virtual clock without wall-clock sleeping.
    Virtual,

    /// Preserve the original recording-time intervals.
    RealTime,

    /// Scale recording-time intervals by a rational playback speed.
    ///
    /// A numerator of 10 and denominator of 1 represents 10x playback.
    /// A numerator of 1 and denominator of 2 represents 0.5x playback.
    ///
    /// Both components must be greater than zero.
    Scaled {
        numerator: u32,
        denominator: u32,
    },
}

impl PlaybackMode {
    /// Validates the playback mode.
    pub fn validate(self) -> ReplayResult<()> {
        match self {
            Self::Scaled {
                numerator: 0, ..
            }
            | Self::Scaled {
                denominator: 0, ..
            } => Err(ReplayError::InvalidConfig {
                field: "playback_mode",
                reason: "speed numerator and denominator must be nonzero",
            }),
            _ => Ok(()),
        }
    }

    /// Returns whether playback requires wall-clock scheduling.
    #[must_use]
    pub const fn uses_wall_clock(self) -> bool {
        matches!(self, Self::RealTime | Self::Scaled { .. })
    }

    /// Returns the rational playback speed, when applicable.
    ///
    /// Unbounded and virtual playback do not have a wall-clock speed.
    #[must_use]
    pub const fn speed_ratio(self) -> Option<(u32, u32)> {
        match self {
            Self::Unbounded | Self::Virtual => None,
            Self::RealTime => Some((1, 1)),
            Self::Scaled {
                numerator,
                denominator,
            } => Some((numerator, denominator)),
        }
    }
}

// -----------------------------------------------------------------------------
// Journal configuration
// -----------------------------------------------------------------------------

/// Configuration governing newly created journal files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalConfig {
    /// Sequence validation semantics of the journal.
    pub mode: JournalMode,

    /// Maximum permitted encoded event payload size.
    pub max_payload_bytes: usize,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            mode: JournalMode::Authoritative,
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
        }
    }
}

impl JournalConfig {
    /// Validates the journal configuration.
    pub fn validate(&self) -> ReplayResult<()> {
        validate_payload_limit(self.max_payload_bytes)
    }
}

// -----------------------------------------------------------------------------
// Writer configuration
// -----------------------------------------------------------------------------

/// Configuration governing journal persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterConfig {
    /// Configuration of the journal being created.
    pub journal: JournalConfig,

    /// Capacity of the buffered writer.
    pub buffer_bytes: usize,

    /// Automatic persistence checkpoint policy.
    pub durability: DurabilityPolicy,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            journal: JournalConfig::default(),
            buffer_bytes: DEFAULT_IO_BUFFER_BYTES,
            durability: DurabilityPolicy::Buffered,
        }
    }
}

impl WriterConfig {
    /// Validates the writer configuration.
    pub fn validate(&self) -> ReplayResult<()> {
        self.journal.validate()?;
        validate_buffer_size(self.buffer_bytes)?;
        self.durability.validate()?;

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Reader configuration
// -----------------------------------------------------------------------------

/// Configuration governing journal reading and validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderConfig {
    /// Capacity of the buffered reader.
    pub buffer_bytes: usize,

    /// Maximum payload size accepted by this reader.
    ///
    /// This limit is independent of the maximum payload size declared
    /// by the journal. The reader must enforce both limits.
    pub max_payload_bytes: usize,

    /// Policy governing incomplete final records.
    pub tail_policy: TailPolicy,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            buffer_bytes: DEFAULT_IO_BUFFER_BYTES,
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            tail_policy: TailPolicy::Strict,
        }
    }
}

impl ReaderConfig {
    /// Validates the reader configuration.
    pub fn validate(&self) -> ReplayResult<()> {
        validate_buffer_size(self.buffer_bytes)?;
        validate_payload_limit(self.max_payload_bytes)?;

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Index configuration
// -----------------------------------------------------------------------------

/// Configuration governing sparse journal indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexConfig {
    /// Whether sparse indexing is enabled.
    pub enabled: bool,

    /// Number of records between candidate index entries.
    pub record_interval: u64,

    /// Maximum number of retained index entries.
    ///
    /// The index implementation must not exceed this limit.
    pub max_entries: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            record_interval: DEFAULT_INDEX_INTERVAL,
            max_entries: DEFAULT_MAX_INDEX_ENTRIES,
        }
    }
}

impl IndexConfig {
    /// Validates the sparse index configuration.
    pub fn validate(&self) -> ReplayResult<()> {
        if self.record_interval == 0 {
            return Err(ReplayError::InvalidConfig {
                field: "index.record_interval",
                reason: "record interval must be greater than zero",
            });
        }

        if self.max_entries == 0 || self.max_entries > MAX_INDEX_ENTRIES {
            return Err(ReplayError::InvalidConfig {
                field: "index.max_entries",
                reason: "index entry limit is outside the supported range",
            });
        }

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Replay configuration
// -----------------------------------------------------------------------------

/// Configuration governing historical event playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayConfig {
    /// Journal reader configuration.
    pub reader: ReaderConfig,

    /// Sparse index configuration.
    pub index: IndexConfig,

    /// Playback scheduling mode.
    pub playback_mode: PlaybackMode,

    /// Maximum number of journal records examined during one replay step.
    ///
    /// Filtered records count toward this budget, ensuring that heavily
    /// filtered journals cannot monopolize the replay processing loop.
    pub max_records_per_step: usize,

    /// Maximum consecutive downstream backpressure attempts.
    ///
    /// Reaching this limit must return control to the caller without
    /// discarding the pending record.
    pub max_backpressure_attempts: u64,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            reader: ReaderConfig::default(),
            index: IndexConfig::default(),
            playback_mode: PlaybackMode::Unbounded,
            max_records_per_step: DEFAULT_MAX_RECORDS_PER_STEP,
            max_backpressure_attempts: DEFAULT_MAX_BACKPRESSURE_ATTEMPTS,
        }
    }
}

impl ReplayConfig {
    /// Validates the replay configuration.
    pub fn validate(&self) -> ReplayResult<()> {
        self.reader.validate()?;
        self.index.validate()?;
        self.playback_mode.validate()?;

        if self.max_records_per_step == 0
            || self.max_records_per_step > MAX_RECORDS_PER_STEP
        {
            return Err(ReplayError::InvalidConfig {
                field: "replay.max_records_per_step",
                reason: "record processing budget is outside the supported range",
            });
        }

        if self.max_backpressure_attempts == 0 {
            return Err(ReplayError::InvalidConfig {
                field: "replay.max_backpressure_attempts",
                reason: "backpressure attempt limit must be greater than zero",
            });
        }

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Shared validation
// -----------------------------------------------------------------------------

fn validate_payload_limit(bytes: usize) -> ReplayResult<()> {
    if bytes == 0 || bytes > MAX_PAYLOAD_BYTES {
        return Err(ReplayError::InvalidConfig {
            field: "max_payload_bytes",
            reason: "payload size limit is outside the supported range",
        });
    }

    Ok(())
}

fn validate_buffer_size(bytes: usize) -> ReplayResult<()> {
    if bytes == 0 || bytes > MAX_IO_BUFFER_BYTES {
        return Err(ReplayError::InvalidConfig {
            field: "buffer_bytes",
            reason: "I/O buffer size is outside the supported range",
        });
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configurations_are_valid() {
        assert!(JournalConfig::default().validate().is_ok());
        assert!(WriterConfig::default().validate().is_ok());
        assert!(ReaderConfig::default().validate().is_ok());
        assert!(IndexConfig::default().validate().is_ok());
        assert!(ReplayConfig::default().validate().is_ok());
    }

    #[test]
    fn authoritative_mode_requires_contiguous_sequences() {
        assert!(
            JournalMode::Authoritative.requires_contiguous_sequences()
        );

        assert!(
            !JournalMode::RawCapture.requires_contiguous_sequences()
        );
    }

    #[test]
    fn zero_payload_limit_is_rejected() {
        let config = JournalConfig {
            max_payload_bytes: 0,
            ..JournalConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn oversized_payload_limit_is_rejected() {
        let config = JournalConfig {
            max_payload_bytes: MAX_PAYLOAD_BYTES + 1,
            ..JournalConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn maximum_payload_limit_is_valid() {
        let config = JournalConfig {
            max_payload_bytes: MAX_PAYLOAD_BYTES,
            ..JournalConfig::default()
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn zero_writer_buffer_is_rejected() {
        let config = WriterConfig {
            buffer_bytes: 0,
            ..WriterConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn oversized_reader_buffer_is_rejected() {
        let config = ReaderConfig {
            buffer_bytes: MAX_IO_BUFFER_BYTES + 1,
            ..ReaderConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_reader_payload_limit_is_rejected() {
        let config = ReaderConfig {
            max_payload_bytes: 0,
            ..ReaderConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_durability_interval_is_rejected() {
        assert!(
            DurabilityPolicy::FlushEveryRecords(0)
                .validate()
                .is_err()
        );

        assert!(
            DurabilityPolicy::SyncEveryRecords(0)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn valid_durability_policies_are_accepted() {
        let policies = [
            DurabilityPolicy::Buffered,
            DurabilityPolicy::FlushEveryRecord,
            DurabilityPolicy::FlushEveryRecords(100),
            DurabilityPolicy::SyncEveryRecord,
            DurabilityPolicy::SyncEveryRecords(100),
        ];

        for policy in policies {
            assert!(policy.validate().is_ok());
        }
    }

    #[test]
    fn synchronization_policies_are_identified() {
        assert!(DurabilityPolicy::SyncEveryRecord.requires_sync());
        assert!(DurabilityPolicy::SyncEveryRecords(100).requires_sync());

        assert!(!DurabilityPolicy::Buffered.requires_sync());
        assert!(!DurabilityPolicy::FlushEveryRecord.requires_sync());
        assert!(!DurabilityPolicy::FlushEveryRecords(100).requires_sync());
    }

    #[test]
    fn invalid_playback_speed_is_rejected() {
        let modes = [
            PlaybackMode::Scaled {
                numerator: 0,
                denominator: 1,
            },
            PlaybackMode::Scaled {
                numerator: 1,
                denominator: 0,
            },
        ];

        for mode in modes {
            assert!(mode.validate().is_err());
        }
    }

    #[test]
    fn valid_playback_modes_are_accepted() {
        let modes = [
            PlaybackMode::Unbounded,
            PlaybackMode::Virtual,
            PlaybackMode::RealTime,
            PlaybackMode::Scaled {
                numerator: 10,
                denominator: 1,
            },
            PlaybackMode::Scaled {
                numerator: 1,
                denominator: 2,
            },
        ];

        for mode in modes {
            assert!(mode.validate().is_ok());
        }
    }

    #[test]
    fn playback_wall_clock_requirements_are_correct() {
        assert!(!PlaybackMode::Unbounded.uses_wall_clock());
        assert!(!PlaybackMode::Virtual.uses_wall_clock());
        assert!(PlaybackMode::RealTime.uses_wall_clock());

        assert!(
            PlaybackMode::Scaled {
                numerator: 10,
                denominator: 1,
            }
            .uses_wall_clock()
        );
    }

    #[test]
    fn playback_speed_ratios_are_correct() {
        assert_eq!(PlaybackMode::Unbounded.speed_ratio(), None);
        assert_eq!(PlaybackMode::Virtual.speed_ratio(), None);
        assert_eq!(PlaybackMode::RealTime.speed_ratio(), Some((1, 1)));

        assert_eq!(
            PlaybackMode::Scaled {
                numerator: 5,
                denominator: 2,
            }
            .speed_ratio(),
            Some((5, 2))
        );
    }

    #[test]
    fn zero_index_interval_is_rejected() {
        let config = IndexConfig {
            record_interval: 0,
            ..IndexConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_index_capacity_is_rejected() {
        let config = IndexConfig {
            max_entries: 0,
            ..IndexConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn excessive_index_capacity_is_rejected() {
        let config = IndexConfig {
            max_entries: MAX_INDEX_ENTRIES + 1,
            ..IndexConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn disabled_index_configuration_is_valid() {
        let config = IndexConfig {
            enabled: false,
            ..IndexConfig::default()
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn zero_replay_budget_is_rejected() {
        let config = ReplayConfig {
            max_records_per_step: 0,
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn excessive_replay_budget_is_rejected() {
        let config = ReplayConfig {
            max_records_per_step: MAX_RECORDS_PER_STEP + 1,
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_backpressure_limit_is_rejected() {
        let config = ReplayConfig {
            max_backpressure_attempts: 0,
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn replay_validates_nested_reader_configuration() {
        let config = ReplayConfig {
            reader: ReaderConfig {
                buffer_bytes: 0,
                ..ReaderConfig::default()
            },
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn replay_validates_nested_index_configuration() {
        let config = ReplayConfig {
            index: IndexConfig {
                max_entries: 0,
                ..IndexConfig::default()
            },
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn replay_validates_playback_configuration() {
        let config = ReplayConfig {
            playback_mode: PlaybackMode::Scaled {
                numerator: 0,
                denominator: 1,
            },
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn writer_validates_nested_journal_configuration() {
        let config = WriterConfig {
            journal: JournalConfig {
                max_payload_bytes: 0,
                ..JournalConfig::default()
            },
            ..WriterConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn writer_validates_durability_configuration() {
        let config = WriterConfig {
            durability: DurabilityPolicy::SyncEveryRecords(0),
            ..WriterConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn strict_tail_policy_is_default() {
        assert_eq!(
            ReaderConfig::default().tail_policy,
            TailPolicy::Strict
        );
    }

    #[test]
    fn raw_capture_writer_configuration_is_valid() {
        let config = WriterConfig {
            journal: JournalConfig {
                mode: JournalMode::RawCapture,
                ..JournalConfig::default()
            },
            durability: DurabilityPolicy::SyncEveryRecords(1_024),
            ..WriterConfig::default()
        };

        assert!(config.validate().is_ok());
    }
}
