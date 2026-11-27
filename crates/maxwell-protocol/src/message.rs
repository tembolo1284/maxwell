//! Protocol message identifiers and canonical messages.

use maxwell_core::{EventEnvelope, EventKind};

use crate::constants::*;
use crate::error::{ProtocolError, ProtocolResult};
use crate::header::MessageHeader;
use crate::version::ProtocolVersion;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageKind {
    OrderAdded = 1,
    OrderModified = 2,
    OrderCancelled = 3,
    OrderDeleted = 4,
    OrderExecuted = 5,
    Trade = 6,
    PriceLevelUpdated = 7,
    BookCleared = 8,
    TradingStatusChanged = 9,
}

impl MessageKind {
    #[must_use]
    #[inline]
    pub const fn code(self) -> u8 {
        self as u8
    }

    #[must_use]
    #[inline]
    pub const fn payload_size(self) -> usize {
        match self {
            Self::OrderAdded => ORDER_ADDED_SIZE,
            Self::OrderModified => ORDER_MODIFIED_SIZE,
            Self::OrderCancelled => ORDER_CANCELLED_SIZE,
            Self::OrderDeleted => ORDER_DELETED_SIZE,
            Self::OrderExecuted => ORDER_EXECUTED_SIZE,
            Self::Trade => TRADE_SIZE,
            Self::PriceLevelUpdated => PRICE_LEVEL_UPDATED_SIZE,
            Self::BookCleared => BOOK_CLEARED_SIZE,
            Self::TradingStatusChanged => TRADING_STATUS_CHANGED_SIZE,
        }
    }

    #[must_use]
    #[inline]
    pub const fn encoded_size(self) -> usize {
        HEADER_SIZE + self.payload_size()
    }
}

impl TryFrom<u8> for MessageKind {
    type Error = ProtocolError;

    #[inline]
    fn try_from(value: u8) -> ProtocolResult<Self> {
        match value {
            1 => Ok(Self::OrderAdded),
            2 => Ok(Self::OrderModified),
            3 => Ok(Self::OrderCancelled),
            4 => Ok(Self::OrderDeleted),
            5 => Ok(Self::OrderExecuted),
            6 => Ok(Self::Trade),
            7 => Ok(Self::PriceLevelUpdated),
            8 => Ok(Self::BookCleared),
            9 => Ok(Self::TradingStatusChanged),
            _ => Err(ProtocolError::UnknownMessageKind(value)),
        }
    }
}

impl From<EventKind> for MessageKind {
    #[inline]
    fn from(value: EventKind) -> Self {
        match value {
            EventKind::OrderAdded => Self::OrderAdded,
            EventKind::OrderModified => Self::OrderModified,
            EventKind::OrderCancelled => Self::OrderCancelled,
            EventKind::OrderDeleted => Self::OrderDeleted,
            EventKind::OrderExecuted => Self::OrderExecuted,
            EventKind::Trade => Self::Trade,
            EventKind::PriceLevelUpdated => Self::PriceLevelUpdated,
            EventKind::BookCleared => Self::BookCleared,
            EventKind::TradingStatusChanged => Self::TradingStatusChanged,
        }
    }
}

/// A canonical event associated with a protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMessage {
    pub version: ProtocolVersion,
    pub envelope: EventEnvelope,
}

impl ProtocolMessage {
    #[must_use]
    #[inline]
    pub const fn new(version: ProtocolVersion, envelope: EventEnvelope) -> Self {
        Self { version, envelope }
    }

    #[must_use]
    #[inline]
    pub const fn current(envelope: EventEnvelope) -> Self {
        Self::new(ProtocolVersion::CURRENT, envelope)
    }

    #[must_use]
    #[inline]
    pub fn kind(self) -> MessageKind {
        self.envelope.kind().into()
    }

    #[must_use]
    #[inline]
    pub fn encoded_size(self) -> usize {
        self.kind().encoded_size()
    }

    #[inline]
    pub fn header(self) -> ProtocolResult<MessageHeader> {
        self.version.validate()?;

        let kind = self.kind();

        let payload_length = u16::try_from(kind.payload_size())
            .map_err(|_| ProtocolError::PayloadTooLarge(kind.payload_size()))?;

        Ok(MessageHeader::new(
            self.version,
            kind,
            payload_length,
            self.envelope.header,
        ))
    }
}

impl From<EventEnvelope> for ProtocolMessage {
    #[inline]
    fn from(envelope: EventEnvelope) -> Self {
        Self::current(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_kind_roundtrip() {
        for code in 1..=9 {
            let kind = MessageKind::try_from(code).unwrap();

            assert_eq!(kind.code(), code);
        }
    }

    #[test]
    fn unknown_message_kind() {
        assert_eq!(
            MessageKind::try_from(255),
            Err(ProtocolError::UnknownMessageKind(255))
        );
    }

    #[test]
    fn order_added_size() {
        assert_eq!(MessageKind::OrderAdded.payload_size(), 25);
        assert_eq!(MessageKind::OrderAdded.encoded_size(), 65);
    }

    #[test]
    fn explicit_event_kind_mapping() {
        assert_eq!(MessageKind::from(EventKind::Trade), MessageKind::Trade);
    }
}
