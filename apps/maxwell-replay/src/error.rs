//! Structured journal and replay errors.

use std::error::Error;
use std::fmt;
use std::io;

pub type ReplayResult<T> = Result<T, ReplayError>;

#[derive(Debug)]
#[non_exhaustive]
pub enum ReplayError {
    InvalidConfig {
        field: &'static str,
        reason: &'static str,
    },

    Io(io::Error),

    AllocationFailed {
        resource: &'static str,
        requested: usize,
    },

    InvalidMagic,

    UnsupportedVersion {
        found: u16,
        supported: u16,
    },

    InvalidHeader {
        reason: &'static str,
    },

    InvalidJournalMode {
        value: u8,
    },

    TruncatedHeader {
        expected: usize,
        actual: usize,
    },

    InvalidRecordLength {
        offset: u64,
        length: u32,
        minimum: u32,
    },

    PayloadTooLarge {
        actual: usize,
        maximum: usize,
    },

    EmptyPayload,

    InvalidRecord {
        offset: u64,
        reason: &'static str,
    },

    TruncatedRecord {
        offset: u64,
        expected: usize,
        actual: usize,
    },

    ChecksumMismatch {
        offset: u64,
        expected: u32,
        actual: u32,
    },

    SequenceMismatch {
        expected: u64,
        actual: u64,
    },

    SequenceOverflow {
        sequence: u64,
    },

    TimestampRegression {
        previous: u64,
        actual: u64,
    },

    SequenceMetadataMismatch {
        envelope: u64,
        payload: u64,
    },

    FeedMismatch,

    EncodingFailed {
        reason: String,
    },

    DecodingFailed {
        offset: u64,
        reason: String,
    },

    InvalidIndex {
        reason: &'static str,
    },

    UnsupportedSeek {
        reason: &'static str,
    },

    SeekTargetNotFound {
        target: u64,
        kind: &'static str,
    },

    InvalidState {
        operation: &'static str,
        state: &'static str,
    },

    NonAuthoritativeReplay {
        reason: &'static str,
    },

    BaselineRequired {
        sequence: u64,
    },

    SinkRejected {
        reason: String,
    },

    ClockOverflow,

    ArithmeticOverflow {
        operation: &'static str,
    },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig { field, reason } => {
                write!(f, "invalid configuration '{field}': {reason}")
            }

            Self::Io(error) => {
                write!(f, "journal I/O error: {error}")
            }

            Self::AllocationFailed {
                resource,
                requested,
            } => {
                write!(f, "allocation failed for {resource}: {requested}")
            }

            Self::InvalidMagic => {
                write!(f, "invalid journal magic")
            }

            Self::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "unsupported journal version {found}; \
                     supported version is {supported}"
                )
            }

            Self::InvalidHeader { reason } => {
                write!(f, "invalid journal header: {reason}")
            }

            Self::InvalidJournalMode { value } => {
                write!(f, "invalid journal mode: {value}")
            }

            Self::TruncatedHeader { expected, actual } => {
                write!(f, "truncated header: expected {expected}, got {actual}")
            }

            Self::InvalidRecordLength {
                offset,
                length,
                minimum,
            } => {
                write!(
                    f,
                    "invalid record length {length} at {offset}; \
                     minimum is {minimum}"
                )
            }

            Self::PayloadTooLarge { actual, maximum } => {
                write!(f, "payload size {actual} exceeds limit {maximum}")
            }

            Self::EmptyPayload => {
                write!(f, "empty journal payload")
            }

            Self::InvalidRecord { offset, reason } => {
                write!(f, "invalid record at {offset}: {reason}")
            }

            Self::TruncatedRecord {
                offset,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "truncated record at {offset}: \
                     expected {expected}, got {actual}"
                )
            }

            Self::ChecksumMismatch {
                offset,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "checksum mismatch at {offset}: \
                     expected {expected:#010x}, got {actual:#010x}"
                )
            }

            Self::SequenceMismatch { expected, actual } => {
                write!(f, "sequence mismatch: expected {expected}, got {actual}")
            }

            Self::SequenceOverflow { sequence } => {
                write!(f, "sequence overflow after {sequence}")
            }

            Self::TimestampRegression { previous, actual } => {
                write!(
                    f,
                    "recording timestamp regression: \
                     previous {previous}, got {actual}"
                )
            }

            Self::SequenceMetadataMismatch { envelope, payload } => {
                write!(
                    f,
                    "sequence metadata mismatch: \
                     envelope {envelope}, payload {payload}"
                )
            }

            Self::FeedMismatch => {
                write!(f, "event belongs to a different upstream feed")
            }

            Self::EncodingFailed { reason } => {
                write!(f, "event encoding failed: {reason}")
            }

            Self::DecodingFailed { offset, reason } => {
                write!(f, "event decoding failed at {offset}: {reason}")
            }

            Self::InvalidIndex { reason } => {
                write!(f, "invalid journal index: {reason}")
            }

            Self::UnsupportedSeek { reason } => {
                write!(f, "unsupported seek: {reason}")
            }

            Self::SeekTargetNotFound { target, kind } => {
                write!(f, "{kind} seek target not found: {target}")
            }

            Self::InvalidState { operation, state } => {
                write!(f, "operation '{operation}' invalid in state '{state}'")
            }

            Self::NonAuthoritativeReplay { reason } => {
                write!(f, "non-authoritative replay: {reason}")
            }

            Self::BaselineRequired { sequence } => {
                write!(f, "reconstruction baseline required before {sequence}")
            }

            Self::SinkRejected { reason } => {
                write!(f, "replay sink rejected event: {reason}")
            }

            Self::ClockOverflow => {
                write!(f, "playback clock overflow")
            }

            Self::ArithmeticOverflow { operation } => {
                write!(f, "arithmetic overflow during {operation}")
            }
        }
    }
}

impl Error for ReplayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ReplayError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl ReplayError {
    #[must_use]
    pub const fn is_io(&self) -> bool {
        matches!(self, Self::Io(_))
    }

    #[must_use]
    pub const fn is_truncation(&self) -> bool {
        matches!(
            self,
            Self::TruncatedHeader { .. } | Self::TruncatedRecord { .. }
        )
    }

    #[must_use]
    pub const fn is_incomplete_record(&self) -> bool {
        matches!(self, Self::TruncatedRecord { .. })
    }

    #[must_use]
    pub const fn is_corruption(&self) -> bool {
        matches!(
            self,
            Self::InvalidMagic
                | Self::InvalidHeader { .. }
                | Self::InvalidJournalMode { .. }
                | Self::InvalidRecordLength { .. }
                | Self::InvalidRecord { .. }
                | Self::ChecksumMismatch { .. }
                | Self::SequenceMetadataMismatch { .. }
                | Self::DecodingFailed { .. }
        )
    }

    #[must_use]
    pub const fn is_sequence_error(&self) -> bool {
        matches!(
            self,
            Self::SequenceMismatch { .. }
                | Self::SequenceOverflow { .. }
                | Self::SequenceMetadataMismatch { .. }
                | Self::FeedMismatch
        )
    }

    #[must_use]
    pub const fn is_unsupported_version(&self) -> bool {
        matches!(self, Self::UnsupportedVersion { .. })
    }

    #[must_use]
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io(error) => Some(error.kind()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_preserves_kind() {
        let error = ReplayError::from(io::Error::new(io::ErrorKind::NotFound, "missing journal"));

        assert!(error.is_io());
        assert_eq!(error.io_kind(), Some(io::ErrorKind::NotFound));
        assert!(error.source().is_some());
    }

    #[test]
    fn checksum_failure_is_corruption() {
        let error = ReplayError::ChecksumMismatch {
            offset: 64,
            expected: 1,
            actual: 2,
        };

        assert!(error.is_corruption());
        assert!(!error.is_truncation());
    }

    #[test]
    fn feed_mismatch_is_sequence_error() {
        assert!(ReplayError::FeedMismatch.is_sequence_error());
    }

    #[test]
    fn incomplete_header_is_not_incomplete_record() {
        let error = ReplayError::TruncatedHeader {
            expected: 64,
            actual: 12,
        };

        assert!(error.is_truncation());
        assert!(!error.is_incomplete_record());
    }
}
