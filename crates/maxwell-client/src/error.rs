//! Client-specific error definitions.

use std::error::Error;
use std::fmt;

use crate::state::ClientState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    InvalidConfig(&'static str),

    InvalidState {
        state: ClientState,
        operation: &'static str,
    },

    ClockRegression {
        previous_ns: u64,
        received_ns: u64,
    },

    InvalidTransportSequence {
        received: u64,
    },

    TransportSequenceExhausted,

    SnapshotCapacityExceeded {
        limit: usize,
        received: usize,
    },

    SnapshotCountMismatch {
        expected: usize,
        received: usize,
    },

    SnapshotWatermarkMismatch {
        expected: u64,
        received: u64,
    },

    SnapshotWatermarkRegression {
        minimum: u64,
        received: u64,
    },

    AllocationFailed,

    HandlerFailure,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => {
                write!(f, "invalid client configuration: {message}")
            }

            Self::InvalidState { state, operation } => {
                write!(f, "operation {operation} is invalid in state {state:?}")
            }

            Self::ClockRegression {
                previous_ns,
                received_ns,
            } => {
                write!(
                    f,
                    "clock regression: previous={previous_ns}, \
                     received={received_ns}"
                )
            }

            Self::InvalidTransportSequence { received } => {
                write!(f, "invalid transport sequence: {received}")
            }

            Self::TransportSequenceExhausted => {
                write!(f, "transport sequence exhausted")
            }

            Self::SnapshotCapacityExceeded { limit, received } => {
                write!(
                    f,
                    "snapshot capacity exceeded: \
                     limit={limit}, received={received}"
                )
            }

            Self::SnapshotCountMismatch { expected, received } => {
                write!(
                    f,
                    "snapshot count mismatch: \
                     expected={expected}, received={received}"
                )
            }

            Self::SnapshotWatermarkMismatch { expected, received } => {
                write!(
                    f,
                    "snapshot watermark mismatch: \
                     expected={expected}, received={received}"
                )
            }

            Self::SnapshotWatermarkRegression { minimum, received } => {
                write!(
                    f,
                    "snapshot watermark regression: \
                     minimum={minimum}, received={received}"
                )
            }

            Self::AllocationFailed => {
                write!(f, "client resource allocation failed")
            }

            Self::HandlerFailure => {
                write!(f, "market data handler failed")
            }
        }
    }
}

impl Error for ClientError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_invalid_sequence() {
        let error = ClientError::InvalidTransportSequence { received: 0 };

        assert_eq!(error.to_string(), "invalid transport sequence: 0");
    }

    #[test]
    fn formats_snapshot_count_mismatch() {
        let error = ClientError::SnapshotCountMismatch {
            expected: 10,
            received: 8,
        };

        assert_eq!(
            error.to_string(),
            "snapshot count mismatch: expected=10, received=8"
        );
    }
}
