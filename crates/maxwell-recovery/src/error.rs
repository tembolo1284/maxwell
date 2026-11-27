//! Recovery-specific error representation.

use core::fmt;

use maxwell_core::{FeedId, SequenceNumber};

pub type RecoveryResult<T> = Result<T, RecoveryError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryError {
    InvalidConfiguration(&'static str),

    FeedMismatch {
        expected: FeedId,
        received: FeedId,
    },

    BufferFull {
        capacity: usize,
    },

    ConflictingEvent {
        sequence: SequenceNumber,
    },

    UnexpectedSequence {
        expected: SequenceNumber,
        received: SequenceNumber,
    },

    SequenceExhausted,
    RequestIdExhausted,
    InvalidRequest,
    StaleResponse,
    InvalidState,
    InvalidEvent,
    SnapshotTooOld,
    SnapshotInstallFailed,
    EventApplicationFailed,
    ClockWentBackward,
    DeadlineOverflow,
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(f, "invalid recovery configuration: {message}")
            }

            Self::FeedMismatch { expected, received } => {
                write!(f, "feed mismatch: expected {expected}, received {received}")
            }

            Self::BufferFull { capacity } => {
                write!(f, "recovery buffer full: capacity {capacity}")
            }

            Self::ConflictingEvent { sequence } => {
                write!(f, "conflicting events at sequence {sequence}")
            }

            Self::UnexpectedSequence { expected, received } => {
                write!(
                    f,
                    "unexpected sequence: expected {expected}, received {received}"
                )
            }

            Self::SequenceExhausted => f.write_str("sequence number space exhausted"),

            Self::RequestIdExhausted => f.write_str("recovery request ID space exhausted"),

            Self::InvalidRequest => f.write_str("invalid recovery request"),

            Self::StaleResponse => f.write_str("stale recovery response"),

            Self::InvalidState => f.write_str("operation invalid in current recovery state"),

            Self::InvalidEvent => f.write_str("invalid market data event"),

            Self::SnapshotTooOld => f.write_str("snapshot watermark is too old"),

            Self::SnapshotInstallFailed => f.write_str("snapshot installation failed"),

            Self::EventApplicationFailed => f.write_str("market data event application failed"),

            Self::ClockWentBackward => f.write_str("recovery clock moved backward"),

            Self::DeadlineOverflow => f.write_str("recovery deadline overflow"),
        }
    }
}

impl std::error::Error for RecoveryError {}
