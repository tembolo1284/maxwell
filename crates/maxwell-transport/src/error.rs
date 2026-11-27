//! Transport-level error representation.

use core::fmt;
use std::io;

pub type TransportResult<T> = Result<T, TransportError>;

#[derive(Debug)]
pub enum TransportError {
    Io(io::Error),

    InvalidConfiguration(&'static str),

    AllocationFailed,

    InvalidMulticastAddress,

    AddressFamilyMismatch,

    BufferTooSmall { required: usize, available: usize },

    DatagramTooLarge { actual: usize, maximum: usize },

    FrameTooLarge { actual: usize, maximum: usize },

    EmptyFrame,

    TruncatedFrame,

    DecoderPoisoned,

    EncoderBusy,

    InvalidWriteProgress,

    ConnectionClosed,

    PartialDatagramWrite { expected: usize, actual: usize },
}

impl TransportError {
    #[must_use]
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io(error) => Some(error.kind()),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_would_block(&self) -> bool {
        self.io_kind() == Some(io::ErrorKind::WouldBlock)
    }

    #[must_use]
    pub fn is_connection_closed(&self) -> bool {
        matches!(
            self,
            Self::ConnectionClosed
                | Self::Io(_)
                    if matches!(
                        self.io_kind(),
                        Some(
                            io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::BrokenPipe
                            | io::ErrorKind::NotConnected
                        )
                    )
        ) || matches!(self, Self::ConnectionClosed)
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => {
                write!(f, "transport I/O error: {error}")
            }

            Self::InvalidConfiguration(message) => {
                write!(f, "invalid transport configuration: {message}")
            }

            Self::AllocationFailed => f.write_str("transport buffer allocation failed"),

            Self::InvalidMulticastAddress => f.write_str("invalid multicast group address"),

            Self::AddressFamilyMismatch => f.write_str("socket address family mismatch"),

            Self::BufferTooSmall {
                required,
                available,
            } => {
                write!(
                    f,
                    "buffer too small: required {required}, \
                     available {available}"
                )
            }

            Self::DatagramTooLarge { actual, maximum } => {
                write!(
                    f,
                    "datagram too large: {actual} bytes, \
                     maximum {maximum}"
                )
            }

            Self::FrameTooLarge { actual, maximum } => {
                write!(
                    f,
                    "frame too large: {actual} bytes, \
                     maximum {maximum}"
                )
            }

            Self::EmptyFrame => f.write_str("empty TCP frame is not permitted"),

            Self::TruncatedFrame => f.write_str("connection ended during a TCP frame"),

            Self::DecoderPoisoned => f.write_str("frame decoder requires reset"),

            Self::EncoderBusy => f.write_str("previous TCP frame is still pending"),

            Self::InvalidWriteProgress => f.write_str("invalid TCP write progress"),

            Self::ConnectionClosed => f.write_str("transport connection closed"),

            Self::PartialDatagramWrite { expected, actual } => {
                write!(
                    f,
                    "partial datagram write: expected {expected}, \
                     wrote {actual}"
                )
            }
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for TransportError {
    #[inline]
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_conversion() {
        let error = TransportError::from(io::Error::from(io::ErrorKind::WouldBlock));

        assert!(error.is_would_block());
    }

    #[test]
    fn connection_reset_classification() {
        let error = TransportError::from(io::Error::from(io::ErrorKind::ConnectionReset));

        assert!(error.is_connection_closed());
    }

    #[test]
    fn explicit_connection_closed() {
        assert!(TransportError::ConnectionClosed.is_connection_closed());
    }
}
