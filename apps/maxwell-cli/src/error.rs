//! Unified CLI error handling.

use std::error::Error;
use std::fmt;
use std::io;

use maxwell_replay::ReplayError;
use maxwell_simulator::SimulatorError;

pub type CliResult<T> = Result<T, CliError>;

#[derive(Debug)]
#[non_exhaustive]
pub enum CliError {
    Usage(String),

    Io(io::Error),

    Simulator(SimulatorError),

    Replay(ReplayError),

    InvalidState {
        operation: &'static str,
        reason: String,
    },
}

impl CliError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,

            Self::Io(_) | Self::Simulator(_) | Self::Replay(_) | Self::InvalidState { .. } => 1,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    pub fn invalid_state(operation: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidState {
            operation,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => {
                write!(formatter, "{message}")
            }

            Self::Io(error) => {
                write!(formatter, "I/O error: {error}")
            }

            Self::Simulator(error) => {
                write!(formatter, "simulation error: {error}")
            }

            Self::Replay(error) => {
                write!(formatter, "journal/replay error: {error}")
            }

            Self::InvalidState { operation, reason } => {
                write!(formatter, "{operation}: {reason}")
            }
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Simulator(error) => Some(error),
            Self::Replay(error) => Some(error),

            Self::Usage(_) | Self::InvalidState { .. } => None,
        }
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<SimulatorError> for CliError {
    fn from(error: SimulatorError) -> Self {
        Self::Simulator(error)
    }
}

impl From<ReplayError> for CliError {
    fn from(error: ReplayError) -> Self {
        Self::Replay(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_errors_return_exit_code_two() {
        let error = CliError::usage("missing argument");

        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn operational_errors_return_exit_code_one() {
        let error = CliError::invalid_state("simulation", "unexpected backpressure");

        assert_eq!(error.exit_code(), 1);
    }

    #[test]
    fn io_errors_preserve_source() {
        let error = CliError::from(io::Error::other("failure"));

        assert!(error.source().is_some());
        assert_eq!(error.exit_code(), 1);
    }
}
