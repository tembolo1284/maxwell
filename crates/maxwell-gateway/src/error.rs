//! Gateway-specific error definitions.

use std::error::Error;
use std::fmt;

use maxwell_publisher::PublisherError;

use crate::session::SessionId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayError {
    InvalidConfig(&'static str),

    SessionLimitReached { limit: usize },

    SessionAlreadyExists { session_id: SessionId },

    SessionNotFound { session_id: SessionId },

    InvalidSessionState { session_id: SessionId },

    SessionExpired { session_id: SessionId },

    ClockRegression { previous_ns: u64, received_ns: u64 },

    AllocationFailed,

    Publisher(PublisherError),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => {
                write!(f, "invalid gateway configuration: {message}")
            }

            Self::SessionLimitReached { limit } => {
                write!(f, "session limit reached: {limit}")
            }

            Self::SessionAlreadyExists { session_id } => {
                write!(f, "session {session_id} already exists")
            }

            Self::SessionNotFound { session_id } => {
                write!(f, "session {session_id} not found")
            }

            Self::InvalidSessionState { session_id } => {
                write!(f, "session {session_id} is in an invalid state")
            }

            Self::SessionExpired { session_id } => {
                write!(f, "session {session_id} has expired")
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

            Self::AllocationFailed => {
                write!(f, "gateway resource allocation failed")
            }

            Self::Publisher(error) => {
                write!(f, "publisher error: {error}")
            }
        }
    }
}

impl Error for GatewayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Publisher(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PublisherError> for GatewayError {
    fn from(error: PublisherError) -> Self {
        Self::Publisher(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_session_error() {
        let error = GatewayError::SessionNotFound {
            session_id: SessionId::new(42),
        };

        assert_eq!(error.to_string(), "session 42 not found");
    }

    #[test]
    fn converts_publisher_error() {
        let source = PublisherError::SequenceExhausted;
        let error = GatewayError::from(source.clone());

        assert_eq!(error, GatewayError::Publisher(source));
        assert!(error.source().is_some());
    }
}
