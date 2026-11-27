//! Stateless protocol validation.
//!
//! Protocol validation is separate from stateful sequence,
//! instrument, and order book validation.

use crate::constants::{HEADER_SIZE, MAX_PAYLOAD_SIZE};

use crate::error::{ProtocolError, ProtocolResult};
use crate::header::MessageHeader;
use crate::message::ProtocolMessage;

// ============================================================
// Header validation
// ============================================================

#[inline]
pub fn validate_header(header: MessageHeader) -> ProtocolResult<()> {
    header.version.validate()?;

    let expected = header.kind.payload_size();
    let actual = usize::from(header.payload_length);

    if actual != expected {
        return Err(ProtocolError::InvalidPayloadLength { expected, actual });
    }

    Ok(())
}

// ============================================================
// Payload validation
// ============================================================

#[inline]
pub fn validate_payload_length(header: MessageHeader, actual: usize) -> ProtocolResult<()> {
    if actual > MAX_PAYLOAD_SIZE {
        return Err(ProtocolError::PayloadTooLarge(actual));
    }

    let expected = usize::from(header.payload_length);

    if actual != expected {
        return Err(ProtocolError::InvalidPayloadLength { expected, actual });
    }

    Ok(())
}

// ============================================================
// Complete message validation
// ============================================================

#[inline]
pub fn validate_message(message: ProtocolMessage) -> ProtocolResult<()> {
    message.version.validate()?;
    message.envelope.validate()?;

    let header = message.header()?;

    validate_header(header)
}

// ============================================================
// Frame validation
// ============================================================

#[inline]
pub fn validate_frame_length(header: MessageHeader, available: usize) -> ProtocolResult<()> {
    validate_header(header)?;

    let expected = HEADER_SIZE + usize::from(header.payload_length);

    if available < expected {
        return Err(ProtocolError::UnexpectedEnd {
            needed: expected,
            remaining: available,
        });
    }

    if available > expected {
        return Err(ProtocolError::TrailingBytes(available - expected));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{
        EventHeader, EventId, FeedId, InstrumentId, SequenceNumber, SessionId, StreamId,
    };

    use crate::{MessageKind, ProtocolVersion};

    fn header() -> MessageHeader {
        MessageHeader::new(
            ProtocolVersion::V1,
            MessageKind::OrderAdded,
            25,
            EventHeader::new(
                EventId::new(
                    FeedId::new(StreamId::new(1), SessionId::new(1)),
                    SequenceNumber::new(1),
                ),
                InstrumentId::new(1),
                None,
            ),
        )
    }

    #[test]
    fn valid_header() {
        assert_eq!(validate_header(header()), Ok(()));
    }

    #[test]
    fn invalid_payload_length() {
        let mut value = header();
        value.payload_length = 10;

        assert_eq!(
            validate_header(value),
            Err(ProtocolError::InvalidPayloadLength {
                expected: 25,
                actual: 10,
            })
        );
    }

    #[test]
    fn valid_frame_length() {
        assert_eq!(validate_frame_length(header(), 65), Ok(()));
    }

    #[test]
    fn truncated_frame() {
        assert_eq!(
            validate_frame_length(header(), 64),
            Err(ProtocolError::UnexpectedEnd {
                needed: 65,
                remaining: 64,
            })
        );
    }

    #[test]
    fn oversized_frame() {
        assert_eq!(
            validate_frame_length(header(), 66),
            Err(ProtocolError::TrailingBytes(1))
        );
    }
}
