//! Strongly typed protocol errors.

use core::fmt;
use maxwell_core::EventError;

pub type ProtocolResult<T> = Result<T, ProtocolError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    UnexpectedEnd { needed: usize, remaining: usize },
    BufferTooSmall { needed: usize, remaining: usize },
    InvalidMagic,
    UnsupportedVersion { major: u8, minor: u8 },
    UnknownMessageKind(u8),
    InvalidFlags(u8),
    InvalidPayloadLength { expected: usize, actual: usize },
    PayloadTooLarge(usize),
    InvalidSide(u8),
    InvalidTradingStatus(u8),
    InvalidClearScope(u8),
    InvalidPresenceFlag(u8),
    NonCanonicalEncoding,
    TrailingBytes(usize),
    InvalidEvent(EventError),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd { needed, remaining } => {
                write!(
                    f,
                    "unexpected end: need {needed} bytes, \
                     have {remaining}"
                )
            }
            Self::BufferTooSmall { needed, remaining } => {
                write!(
                    f,
                    "buffer too small: need {needed} bytes, \
                     have {remaining}"
                )
            }
            Self::InvalidMagic => f.write_str("invalid protocol magic"),
            Self::UnsupportedVersion { major, minor } => {
                write!(f, "unsupported protocol version {major}.{minor}")
            }
            Self::UnknownMessageKind(value) => {
                write!(f, "unknown message kind: {value}")
            }
            Self::InvalidFlags(value) => {
                write!(f, "invalid message flags: {value:#04x}")
            }
            Self::InvalidPayloadLength { expected, actual } => {
                write!(
                    f,
                    "invalid payload length: expected {expected}, \
                     received {actual}"
                )
            }
            Self::PayloadTooLarge(value) => {
                write!(f, "payload too large: {value} bytes")
            }
            Self::InvalidSide(value) => {
                write!(f, "invalid side: {value}")
            }
            Self::InvalidTradingStatus(value) => {
                write!(f, "invalid trading status: {value}")
            }
            Self::InvalidClearScope(value) => {
                write!(f, "invalid book clear scope: {value}")
            }
            Self::InvalidPresenceFlag(value) => {
                write!(f, "invalid presence flag: {value}")
            }
            Self::NonCanonicalEncoding => f.write_str("noncanonical wire encoding"),
            Self::TrailingBytes(value) => {
                write!(f, "{value} unexpected trailing bytes")
            }
            Self::InvalidEvent(error) => {
                write!(f, "invalid market event: {error}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidEvent(error) => Some(error),
            _ => None,
        }
    }
}

impl From<EventError> for ProtocolError {
    #[inline]
    fn from(error: EventError) -> Self {
        Self::InvalidEvent(error)
    }
}

const _: () = {
    assert!(!core::mem::needs_drop::<ProtocolError>());
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_error_conversion() {
        let error: ProtocolError = EventError::ZeroOrderQuantity.into();

        assert_eq!(
            error,
            ProtocolError::InvalidEvent(EventError::ZeroOrderQuantity)
        );
    }

    #[test]
    fn unsupported_version_display() {
        let error = ProtocolError::UnsupportedVersion { major: 2, minor: 1 };

        assert_eq!(error.to_string(), "unsupported protocol version 2.1");
    }

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<ProtocolError>();
    }
}
