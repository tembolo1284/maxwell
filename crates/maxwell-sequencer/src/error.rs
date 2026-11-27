
//! Strongly typed sequencing errors.

use core::fmt;

use maxwell_core::{
    EventError,
    FeedId,
    SequenceNumber,
    StreamId,
};

pub type SequencerResult<T> = Result<T, SequencerError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequencerError {
    InvalidReorderCapacity(usize),
    InvalidStreamCapacity(usize),
    AllocationFailed,
    StreamCapacityExceeded,
    UnknownStream(StreamId),
    StreamAlreadyRegistered(StreamId),

    FeedMismatch {
        expected: FeedId,
        actual: FeedId,
    },

    SequenceOutsideWindow {
        expected: SequenceNumber,
        received: SequenceNumber,
        capacity: usize,
    },

    SequenceMismatch {
        expected: SequenceNumber,
        received: SequenceNumber,
    },

    BufferCollision {
        existing: SequenceNumber,
        incoming: SequenceNumber,
    },

    ConflictingDuplicate(SequenceNumber),
    InvalidGap,
    SequenceExhausted,
    RecoveryRequired,
    InvalidRecoverySequence,
    InvalidEvent(EventError),
}

impl fmt::Display for SequencerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReorderCapacity(value) => {
                write!(f, "invalid reorder capacity: {value}")
            }

            Self::InvalidStreamCapacity(value) => {
                write!(f, "invalid stream capacity: {value}")
            }

            Self::AllocationFailed => {
                f.write_str("sequencer allocation failed")
            }

            Self::StreamCapacityExceeded => {
                f.write_str("stream capacity exceeded")
            }

            Self::UnknownStream(stream) => {
                write!(f, "unknown stream: {stream}")
            }

            Self::StreamAlreadyRegistered(stream) => {
                write!(f, "stream already registered: {stream}")
            }

            Self::FeedMismatch { expected, actual } => {
                write!(
                    f,
                    "feed mismatch: expected {expected:?}, \
                     received {actual:?}"
                )
            }

            Self::SequenceOutsideWindow {
                expected,
                received,
                capacity,
            } => {
                write!(
                    f,
                    "sequence {received:?} outside window: \
                     expected {expected:?}, capacity {capacity}"
                )
            }

            Self::SequenceMismatch {
                expected,
                received,
            } => {
                write!(
                    f,
                    "sequence mismatch: expected {expected:?}, \
                     received {received:?}"
                )
            }

            Self::BufferCollision {
                existing,
                incoming,
            } => {
                write!(
                    f,
                    "buffer collision: existing {existing:?}, \
                     incoming {incoming:?}"
                )
            }

            Self::ConflictingDuplicate(sequence) => {
                write!(
                    f,
                    "conflicting duplicate: {sequence:?}"
                )
            }

            Self::InvalidGap => {
                f.write_str("invalid sequence gap")
            }

            Self::SequenceExhausted => {
                f.write_str("sequence number space exhausted")
            }

            Self::RecoveryRequired => {
                f.write_str("feed recovery required")
            }

            Self::InvalidRecoverySequence => {
                f.write_str("recovery would regress sequence")
            }

            Self::InvalidEvent(error) => {
                write!(f, "invalid event: {error}")
            }
        }
    }
}

impl std::error::Error for SequencerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidEvent(error) => Some(error),
            _ => None,
        }
    }
}

impl From<EventError> for SequencerError {
    #[inline]
    fn from(error: EventError) -> Self {
        Self::InvalidEvent(error)
    }
}

const _: () = {
    assert!(!core::mem::needs_drop::<SequencerError>());
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_conversion() {
        let error: SequencerError =
            EventError::ZeroOrderQuantity.into();

        assert_eq!(
            error,
            SequencerError::InvalidEvent(
                EventError::ZeroOrderQuantity,
            ),
        );
    }

    #[test]
    fn recovery_error_display() {
        assert_eq!(
            SequencerError::RecoveryRequired.to_string(),
            "feed recovery required",
        );
    }

    #[test]
    fn errors_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<SequencerError>();
    }
}

