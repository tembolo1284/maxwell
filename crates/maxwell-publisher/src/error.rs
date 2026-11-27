//! Publisher-specific error types.

use std::error::Error;
use std::fmt;

use crate::subscription::SubscriberId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublisherError {
    InvalidConfig(&'static str),

    SubscriberLimitReached { limit: usize },

    SubscriberAlreadyExists { subscriber_id: SubscriberId },

    SubscriberNotFound { subscriber_id: SubscriberId },

    InstrumentLimitReached { limit: usize },

    Backpressure { subscriber_id: SubscriberId },

    ResyncRequired { subscriber_id: SubscriberId },

    InvalidSubscriberState { subscriber_id: SubscriberId },

    InvalidWatermark { expected: u64, received: u64 },

    SequenceExhausted,

    AllocationFailed,
}

impl fmt::Display for PublisherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => {
                write!(f, "invalid publisher configuration: {message}")
            }

            Self::SubscriberLimitReached { limit } => {
                write!(f, "subscriber limit reached: {limit}")
            }

            Self::SubscriberAlreadyExists { subscriber_id } => {
                write!(f, "subscriber {subscriber_id} already exists")
            }

            Self::SubscriberNotFound { subscriber_id } => {
                write!(f, "subscriber {subscriber_id} not found")
            }

            Self::InstrumentLimitReached { limit } => {
                write!(f, "subscription instrument limit reached: {limit}")
            }

            Self::Backpressure { subscriber_id } => {
                write!(f, "subscriber {subscriber_id} queue is full")
            }

            Self::ResyncRequired { subscriber_id } => {
                write!(f, "subscriber {subscriber_id} requires resynchronization")
            }

            Self::InvalidSubscriberState { subscriber_id } => {
                write!(f, "subscriber {subscriber_id} is in an invalid state")
            }

            Self::InvalidWatermark { expected, received } => {
                write!(
                    f,
                    "invalid watermark: expected {expected}, received {received}"
                )
            }

            Self::SequenceExhausted => {
                write!(f, "publication sequence exhausted")
            }

            Self::AllocationFailed => {
                write!(f, "publisher resource allocation failed")
            }
        }
    }
}

impl Error for PublisherError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_backpressure_error() {
        let error = PublisherError::Backpressure {
            subscriber_id: SubscriberId::new(42),
        };

        assert_eq!(error.to_string(), "subscriber 42 queue is full");
    }

    #[test]
    fn formats_watermark_error() {
        let error = PublisherError::InvalidWatermark {
            expected: 100,
            received: 99,
        };

        assert_eq!(
            error.to_string(),
            "invalid watermark: expected 100, received 99"
        );
    }
}
