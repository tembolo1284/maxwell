//! Journal and replay configuration.

use maxwell_protocol::constants::MAX_MESSAGE_SIZE;

use crate::error::{ReplayError, ReplayResult};

// ============================================================
// Resource limits
// ============================================================

pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = MAX_MESSAGE_SIZE;
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

pub const DEFAULT_IO_BUFFER_BYTES: usize = 256 * 1024;
pub const MAX_IO_BUFFER_BYTES: usize = 64 * 1024 * 1024;

pub const DEFAULT_INDEX_INTERVAL: u64 = 1_024;
pub const DEFAULT_MAX_INDEX_ENTRIES: usize = 65_536;
pub const MAX_INDEX_ENTRIES: usize = 1_048_576;

pub const DEFAULT_MAX_RECORDS_PER_STEP: usize = 4_096;
pub const MAX_RECORDS_PER_STEP: usize = 1_048_576;

pub const DEFAULT_MAX_BACKPRESSURE_ATTEMPTS: u64 = 10_000;

// ============================================================
// Journal mode
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalMode {
    #[default]
    Authoritative,
    RawCapture,
}

impl JournalMode {
    #[must_use]
    pub const fn requires_contiguous_sequences(self) -> bool {
        matches!(self, Self::Authoritative)
    }
}

// ============================================================
// Durability
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DurabilityPolicy {
    #[default]
    Buffered,
    FlushEveryRecord,
    FlushEveryRecords(u64),
    SyncEveryRecord,
    SyncEveryRecords(u64),
}

impl DurabilityPolicy {
    pub fn validate(self) -> ReplayResult<()> {
        match self {
            Self::FlushEveryRecords(0) | Self::SyncEveryRecords(0) => {
                Err(ReplayError::InvalidConfig {
                    field: "durability",
                    reason: "record interval must be nonzero",
                })
            }
            _ => Ok(()),
        }
    }

    #[must_use]
    pub const fn requires_sync(self) -> bool {
        matches!(self, Self::SyncEveryRecord | Self::SyncEveryRecords(_))
    }
}

// ============================================================
// Incomplete-tail policy
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TailPolicy {
    #[default]
    Strict,
    AllowIncompleteTail,
}

// ============================================================
// Playback mode
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackMode {
    #[default]
    Unbounded,
    Virtual,
    RealTime,
    Scaled {
        numerator: u32,
        denominator: u32,
    },
}

impl PlaybackMode {
    pub fn validate(self) -> ReplayResult<()> {
        match self {
            Self::Scaled { numerator: 0, .. } | Self::Scaled { denominator: 0, .. } => {
                Err(ReplayError::InvalidConfig {
                    field: "playback_mode",
                    reason: "speed components must be nonzero",
                })
            }
            _ => Ok(()),
        }
    }

    #[must_use]
    pub const fn uses_wall_clock(self) -> bool {
        matches!(self, Self::RealTime | Self::Scaled { .. })
    }

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

// ============================================================
// Journal configuration
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalConfig {
    pub mode: JournalMode,
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
    pub fn validate(&self) -> ReplayResult<()> {
        validate_payload_limit(self.max_payload_bytes)
    }
}

// ============================================================
// Writer configuration
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterConfig {
    pub journal: JournalConfig,
    pub buffer_bytes: usize,
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
    pub fn validate(&self) -> ReplayResult<()> {
        self.journal.validate()?;
        validate_buffer_size(self.buffer_bytes)?;
        self.durability.validate()
    }
}

// ============================================================
// Reader configuration
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderConfig {
    pub buffer_bytes: usize,
    pub max_payload_bytes: usize,
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
    pub fn validate(&self) -> ReplayResult<()> {
        validate_buffer_size(self.buffer_bytes)?;
        validate_payload_limit(self.max_payload_bytes)
    }
}

// ============================================================
// Sparse index configuration
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexConfig {
    pub enabled: bool,
    pub record_interval: u64,
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
    pub fn validate(&self) -> ReplayResult<()> {
        if self.record_interval == 0 {
            return Err(ReplayError::InvalidConfig {
                field: "index.record_interval",
                reason: "record interval must be nonzero",
            });
        }

        if self.max_entries == 0 || self.max_entries > MAX_INDEX_ENTRIES {
            return Err(ReplayError::InvalidConfig {
                field: "index.max_entries",
                reason: "index capacity outside supported range",
            });
        }

        Ok(())
    }
}

// ============================================================
// Replay configuration
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayConfig {
    pub reader: ReaderConfig,
    pub index: IndexConfig,
    pub playback_mode: PlaybackMode,
    pub max_records_per_step: usize,
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
    pub fn validate(&self) -> ReplayResult<()> {
        self.reader.validate()?;
        self.index.validate()?;
        self.playback_mode.validate()?;

        if self.max_records_per_step == 0 || self.max_records_per_step > MAX_RECORDS_PER_STEP {
            return Err(ReplayError::InvalidConfig {
                field: "replay.max_records_per_step",
                reason: "processing budget outside supported range",
            });
        }

        if self.max_backpressure_attempts == 0 {
            return Err(ReplayError::InvalidConfig {
                field: "replay.max_backpressure_attempts",
                reason: "backpressure budget must be nonzero",
            });
        }

        Ok(())
    }
}

// ============================================================
// Shared validation
// ============================================================

fn validate_payload_limit(bytes: usize) -> ReplayResult<()> {
    if bytes == 0 || bytes > MAX_PAYLOAD_BYTES {
        return Err(ReplayError::InvalidConfig {
            field: "max_payload_bytes",
            reason: "payload limit outside supported range",
        });
    }

    Ok(())
}

fn validate_buffer_size(bytes: usize) -> ReplayResult<()> {
    if bytes == 0 || bytes > MAX_IO_BUFFER_BYTES {
        return Err(ReplayError::InvalidConfig {
            field: "buffer_bytes",
            reason: "buffer size outside supported range",
        });
    }

    Ok(())
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(JournalConfig::default().validate().is_ok());
        assert!(WriterConfig::default().validate().is_ok());
        assert!(ReaderConfig::default().validate().is_ok());
        assert!(IndexConfig::default().validate().is_ok());
        assert!(ReplayConfig::default().validate().is_ok());
    }

    #[test]
    fn default_payload_supports_protocol_maximum() {
        assert_eq!(JournalConfig::default().max_payload_bytes, MAX_MESSAGE_SIZE);
    }

    #[test]
    fn invalid_payload_limits() {
        for bytes in [0, MAX_PAYLOAD_BYTES + 1] {
            let config = JournalConfig {
                max_payload_bytes: bytes,
                ..JournalConfig::default()
            };

            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn invalid_buffer_limits() {
        for bytes in [0, MAX_IO_BUFFER_BYTES + 1] {
            let config = ReaderConfig {
                buffer_bytes: bytes,
                ..ReaderConfig::default()
            };

            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn invalid_durability_intervals() {
        assert!(DurabilityPolicy::FlushEveryRecords(0).validate().is_err());

        assert!(DurabilityPolicy::SyncEveryRecords(0).validate().is_err());
    }

    #[test]
    fn invalid_playback_ratio() {
        let mode = PlaybackMode::Scaled {
            numerator: 0,
            denominator: 1,
        };

        assert!(mode.validate().is_err());
    }

    #[test]
    fn invalid_index_capacity() {
        let config = IndexConfig {
            max_entries: 0,
            ..IndexConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_replay_budget() {
        let config = ReplayConfig {
            max_records_per_step: 0,
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_backpressure_budget() {
        let config = ReplayConfig {
            max_backpressure_attempts: 0,
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn nested_configuration_is_validated() {
        let config = ReplayConfig {
            reader: ReaderConfig {
                buffer_bytes: 0,
                ..ReaderConfig::default()
            },
            ..ReplayConfig::default()
        };

        assert!(config.validate().is_err());
    }
}
