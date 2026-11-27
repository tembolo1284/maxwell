//! Structured errors for journal persistence and historical replay.

use std::error::Error;
use std::fmt;
use std::io;

// -----------------------------------------------------------------------------
// Result alias
// -----------------------------------------------------------------------------

/// Result type used throughout Maxwell Replay.
pub type ReplayResult<T> = Result<T, ReplayError>;

// -----------------------------------------------------------------------------
// Replay error
// -----------------------------------------------------------------------------

/// Errors produced by journal recording, reading, validation, and replay.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReplayError {
    // -------------------------------------------------------------------------
    // Configuration
    // -------------------------------------------------------------------------

    /// A configuration field contains an unsupported value.
    InvalidConfig {
        field: &'static str,
        reason: &'static str,
    },

    // -------------------------------------------------------------------------
    // I/O
    // -------------------------------------------------------------------------

    /// An underlying I/O operation failed.
    Io(io::Error),

    /// A resource allocation could not be satisfied.
    AllocationFailed {
        resource: &'static str,
        requested: usize,
    },

    // -------------------------------------------------------------------------
    // Journal header
    // -------------------------------------------------------------------------

    /// The journal does not contain the expected magic bytes.
    InvalidMagic,

    /// The journal format version is unsupported.
    UnsupportedVersion {
        found: u16,
        supported: u16,
    },

    /// The journal header contains invalid or inconsistent metadata.
    InvalidHeader {
        reason: &'static str,
    },

    /// The journal contains an unsupported mode identifier.
    InvalidJournalMode {
        value: u8,
    },

    /// The journal header ended before all required bytes were available.
    TruncatedHeader {
        expected: usize,
        actual: usize,
    },

    // -------------------------------------------------------------------------
    // Record validation
    // -------------------------------------------------------------------------

    /// A record length is smaller than the required structural overhead.
    InvalidRecordLength {
        offset: u64,
        length: u32,
        minimum: u32,
    },

    /// An encoded event payload exceeds the permitted resource limit.
    PayloadTooLarge {
        actual: usize,
        maximum: usize,
    },

    /// An encoded event payload is empty.
    EmptyPayload,

    /// A record contains invalid or inconsistent metadata.
    InvalidRecord {
        offset: u64,
        reason: &'static str,
    },

    /// A record ended before all declared bytes were available.
    TruncatedRecord {
        offset: u64,
        expected: usize,
        actual: usize,
    },

    /// A stored checksum differs from the calculated checksum.
    ///
    /// The offset identifies the beginning of the protected journal
    /// structure, which may be a header or record.
    ChecksumMismatch {
        offset: u64,
        expected: u32,
        actual: u32,
    },

    // -------------------------------------------------------------------------
    // Sequence and timestamp validation
    // -------------------------------------------------------------------------

    /// An authoritative journal received an unexpected upstream sequence.
    SequenceMismatch {
        expected: u64,
        actual: u64,
    },

    /// An authoritative sequence cannot be incremented without overflow.
    SequenceOverflow {
        sequence: u64,
    },

    /// The recording timestamp moved backward.
    TimestampRegression {
        previous: u64,
        actual: u64,
    },

    /// Record envelope metadata disagrees with the encoded event.
    SequenceMetadataMismatch {
        envelope: u64,
        payload: u64,
    },

    // -------------------------------------------------------------------------
    // Protocol
    // -------------------------------------------------------------------------

    /// A Maxwell protocol event could not be encoded.
    EncodingFailed {
        reason: String,
    },

    /// A Maxwell protocol event could not be decoded.
    DecodingFailed {
        offset: u64,
        reason: String,
    },

    // -------------------------------------------------------------------------
    // Indexing and seeking
    // -------------------------------------------------------------------------

    /// An index entry is inconsistent with the journal.
    InvalidIndex {
        reason: &'static str,
    },

    /// The requested seek operation is unsupported.
    UnsupportedSeek {
        reason: &'static str,
    },

    /// The requested journal position does not exist.
    SeekTargetNotFound {
        target: u64,
        kind: &'static str,
    },

    // -------------------------------------------------------------------------
    // Replay lifecycle
    // -------------------------------------------------------------------------

    /// An operation is invalid in the current lifecycle state.
    InvalidState {
        operation: &'static str,
        state: &'static str,
    },

    /// Replay cannot safely claim authoritative continuity.
    NonAuthoritativeReplay {
        reason: &'static str,
    },

    /// The caller has not established the required reconstruction baseline.
    BaselineRequired {
        sequence: u64,
    },

    /// A downstream consumer permanently rejected a replayed event.
    SinkRejected {
        reason: String,
    },

    /// Playback timestamp arithmetic exceeded its supported range.
    ClockOverflow,

    /// An internal counter or file offset exceeded its supported range.
    ArithmeticOverflow {
        operation: &'static str,
    },
}

// -----------------------------------------------------------------------------
// Error formatting
// -----------------------------------------------------------------------------

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig { field, reason } => {
                write!(f, "invalid configuration for '{field}': {reason}")
            }

            Self::Io(error) => {
                write!(f, "journal I/O error: {error}")
            }

            Self::AllocationFailed {
                resource,
                requested,
            } => {
                write!(
                    f,
                    "allocation failed for {resource}: \
                     requested capacity {requested}"
                )
            }

            Self::InvalidMagic => {
                write!(f, "invalid journal magic bytes")
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
                write!(f, "invalid journal mode identifier: {value}")
            }

            Self::TruncatedHeader { expected, actual } => {
                write!(
                    f,
                    "truncated journal header: expected {expected} bytes, \
                     received {actual}"
                )
            }

            Self::InvalidRecordLength {
                offset,
                length,
                minimum,
            } => {
                write!(
                    f,
                    "invalid record length at offset {offset}: \
                     received {length}, minimum is {minimum}"
                )
            }

            Self::PayloadTooLarge { actual, maximum } => {
                write!(
                    f,
                    "event payload exceeds configured limit: \
                     {actual} bytes exceeds {maximum} bytes"
                )
            }

            Self::EmptyPayload => {
                write!(f, "journal event payload must not be empty")
            }

            Self::InvalidRecord { offset, reason } => {
                write!(f, "invalid record at offset {offset}: {reason}")
            }

            Self::TruncatedRecord {
                offset,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "truncated record at offset {offset}: \
                     expected {expected} bytes, received {actual}"
                )
            }

            Self::ChecksumMismatch {
                offset,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "checksum mismatch at offset {offset}: \
                     expected {expected:#010x}, calculated {actual:#010x}"
                )
            }

            Self::SequenceMismatch { expected, actual } => {
                write!(
                    f,
                    "upstream sequence mismatch: \
                     expected {expected}, received {actual}"
                )
            }

            Self::SequenceOverflow { sequence } => {
                write!(
                    f,
                    "upstream sequence overflow after sequence {sequence}"
                )
            }

            Self::TimestampRegression { previous, actual } => {
                write!(
                    f,
                    "recording timestamp moved backward: \
                     previous {previous}, received {actual}"
                )
            }

            Self::SequenceMetadataMismatch { envelope, payload } => {
                write!(
                    f,
                    "record sequence metadata mismatch: \
                     envelope {envelope}, payload {payload}"
                )
            }

            Self::EncodingFailed { reason } => {
                write!(f, "market event encoding failed: {reason}")
            }

            Self::DecodingFailed { offset, reason } => {
                write!(
                    f,
                    "market event decoding failed at offset {offset}: {reason}"
                )
            }

            Self::InvalidIndex { reason } => {
                write!(f, "invalid journal index: {reason}")
            }

            Self::UnsupportedSeek { reason } => {
                write!(f, "unsupported journal seek operation: {reason}")
            }

            Self::SeekTargetNotFound { target, kind } => {
                write!(f, "journal {kind} seek target not found: {target}")
            }

            Self::InvalidState { operation, state } => {
                write!(
                    f,
                    "operation '{operation}' is invalid in state '{state}'"
                )
            }

            Self::NonAuthoritativeReplay { reason } => {
                write!(
                    f,
                    "replay cannot guarantee authoritative continuity: {reason}"
                )
            }

            Self::BaselineRequired { sequence } => {
                write!(
                    f,
                    "a compatible reconstruction baseline is required \
                     before sequence {sequence}"
                )
            }

            Self::SinkRejected { reason } => {
                write!(f, "downstream replay sink rejected event: {reason}")
            }

            Self::ClockOverflow => {
                write!(f, "playback clock arithmetic overflow")
            }

            Self::ArithmeticOverflow { operation } => {
                write!(f, "arithmetic overflow during '{operation}'")
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Standard error integration
// -----------------------------------------------------------------------------

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

// -----------------------------------------------------------------------------
// Error classification
// -----------------------------------------------------------------------------

impl ReplayError {
    /// Returns whether the error represents an underlying I/O failure.
    #[must_use]
    pub const fn is_io(&self) -> bool {
        matches!(self, Self::Io(_))
    }

    /// Returns whether a journal structure ended prematurely.
    #[must_use]
    pub const fn is_truncation(&self) -> bool {
        matches!(
            self,
            Self::TruncatedHeader { .. }
                | Self::TruncatedRecord { .. }
        )
    }

    /// Returns whether the error represents an incomplete record.
    ///
    /// The reader must separately establish that the incomplete record
    /// is at the physical end of the journal before applying a tolerant
    /// tail policy.
    #[must_use]
    pub const fn is_incomplete_record(&self) -> bool {
        matches!(self, Self::TruncatedRecord { .. })
    }

    /// Returns whether the error represents an invalid journal structure
    /// or inconsistent encoded event metadata.
    ///
    /// Unsupported versions, resource limits, and incomplete tails are
    /// deliberately classified separately.
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
        )
    }

    /// Returns whether the error represents a sequence validation failure.
    #[must_use]
    pub const fn is_sequence_error(&self) -> bool {
        matches!(
            self,
            Self::SequenceMismatch { .. }
                | Self::SequenceOverflow { .. }
                | Self::SequenceMetadataMismatch { .. }
        )
    }

    /// Returns whether the error represents an unsupported format version.
    #[must_use]
    pub const fn is_unsupported_version(&self) -> bool {
        matches!(self, Self::UnsupportedVersion { .. })
    }

    /// Returns the underlying I/O error kind, when available.
    #[must_use]
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io(error) => Some(error.kind()),
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_conversion_preserves_error_kind() {
        let original = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "permission denied",
        );

        let error = ReplayError::from(original);

        assert!(error.is_io());
        assert_eq!(
            error.io_kind(),
            Some(io::ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn io_error_preserves_standard_error_source() {
        let error = ReplayError::from(io::Error::new(
            io::ErrorKind::NotFound,
            "journal not found",
        ));

        assert!(error.source().is_some());
    }

    #[test]
    fn non_io_error_has_no_standard_error_source() {
        let error = ReplayError::InvalidMagic;

        assert!(error.source().is_none());
        assert!(error.io_kind().is_none());
        assert!(!error.is_io());
    }

    #[test]
    fn invalid_configuration_display_is_descriptive() {
        let error = ReplayError::InvalidConfig {
            field: "buffer_bytes",
            reason: "must be greater than zero",
        };

        assert_eq!(
            error.to_string(),
            "invalid configuration for 'buffer_bytes': \
             must be greater than zero"
        );
    }

    #[test]
    fn unsupported_version_is_not_classified_as_corruption() {
        let error = ReplayError::UnsupportedVersion {
            found: 2,
            supported: 1,
        };

        assert!(error.is_unsupported_version());
        assert!(!error.is_corruption());
        assert!(!error.is_truncation());
    }

    #[test]
    fn checksum_mismatch_is_classified_as_corruption() {
        let error = ReplayError::ChecksumMismatch {
            offset: 128,
            expected: 0x1234_5678,
            actual: 0x8765_4321,
        };

        assert!(error.is_corruption());
        assert!(!error.is_truncation());
        assert!(!error.is_incomplete_record());

        let message = error.to_string();

        assert!(message.contains("128"));
        assert!(message.contains("0x12345678"));
        assert!(message.contains("0x87654321"));
    }

    #[test]
    fn truncated_header_is_not_an_incomplete_record() {
        let error = ReplayError::TruncatedHeader {
            expected: 64,
            actual: 16,
        };

        assert!(error.is_truncation());
        assert!(!error.is_incomplete_record());
        assert!(!error.is_corruption());
    }

    #[test]
    fn truncated_record_is_classified_correctly() {
        let error = ReplayError::TruncatedRecord {
            offset: 1_024,
            expected: 128,
            actual: 32,
        };

        assert!(error.is_truncation());
        assert!(error.is_incomplete_record());
        assert!(!error.is_corruption());
    }

    #[test]
    fn invalid_record_length_is_classified_as_corruption() {
        let error = ReplayError::InvalidRecordLength {
            offset: 256,
            length: 2,
            minimum: 20,
        };

        assert!(error.is_corruption());
        assert!(!error.is_truncation());
    }

    #[test]
    fn sequence_mismatch_is_classified_correctly() {
        let error = ReplayError::SequenceMismatch {
            expected: 101,
            actual: 103,
        };

        assert!(error.is_sequence_error());
        assert!(!error.is_io());
        assert!(!error.is_truncation());

        assert_eq!(
            error.to_string(),
            "upstream sequence mismatch: expected 101, received 103"
        );
    }

    #[test]
    fn sequence_overflow_is_classified_correctly() {
        let error = ReplayError::SequenceOverflow {
            sequence: u64::MAX,
        };

        assert!(error.is_sequence_error());
    }

    #[test]
    fn sequence_metadata_mismatch_is_corruption() {
        let error = ReplayError::SequenceMetadataMismatch {
            envelope: 100,
            payload: 101,
        };

        assert!(error.is_corruption());
        assert!(error.is_sequence_error());
    }

    #[test]
    fn payload_limit_failure_is_not_necessarily_corruption() {
        let error = ReplayError::PayloadTooLarge {
            actual: 1_024,
            maximum: 512,
        };

        assert!(!error.is_corruption());
        assert!(!error.is_truncation());
    }

    #[test]
    fn timestamp_regression_display_is_descriptive() {
        let error = ReplayError::TimestampRegression {
            previous: 1_000,
            actual: 999,
        };

        assert_eq!(
            error.to_string(),
            "recording timestamp moved backward: \
             previous 1000, received 999"
        );
    }

    #[test]
    fn invalid_state_display_contains_operation_and_state() {
        let error = ReplayError::InvalidState {
            operation: "append",
            state: "closed",
        };

        assert_eq!(
            error.to_string(),
            "operation 'append' is invalid in state 'closed'"
        );
    }

    #[test]
    fn decoding_failure_contains_file_offset() {
        let error = ReplayError::DecodingFailed {
            offset: 4_096,
            reason: String::from("invalid event type"),
        };

        assert_eq!(
            error.to_string(),
            "market event decoding failed at offset 4096: \
             invalid event type"
        );
    }

    #[test]
    fn baseline_required_display_contains_sequence() {
        let error = ReplayError::BaselineRequired {
            sequence: 10_000,
        };

        assert_eq!(
            error.to_string(),
            "a compatible reconstruction baseline is required \
             before sequence 10000"
        );
    }

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<ReplayError>();
    }
}
