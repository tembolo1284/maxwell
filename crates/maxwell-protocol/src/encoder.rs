//! Allocation-free market data encoder.
//!
//! Encodes into caller-provided buffers. Validation and
//! capacity checks occur before the first write.

use maxwell_core::EventEnvelope;

use crate::error::ProtocolResult;
use crate::message::ProtocolMessage;
use crate::writer::ByteWriter;
use crate::{validation, wire};

#[derive(Debug, Clone, Copy, Default)]
pub struct Encoder;

impl Encoder {
    #[must_use]
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    /// Returns the required encoded size.
    ///
    /// # Errors
    ///
    /// Returns an error if the message fails validation.
    #[inline]
    pub fn required_size(&self, message: ProtocolMessage) -> ProtocolResult<usize> {
        validation::validate_message(message)?;

        Ok(message.encoded_size())
    }

    /// Encodes a complete protocol message.
    ///
    /// Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns an error if the message fails validation or does
    /// not fit in `buffer`.
    #[inline]
    pub fn encode(&self, message: ProtocolMessage, buffer: &mut [u8]) -> ProtocolResult<usize> {
        let mut writer = ByteWriter::new(buffer);

        self.encode_into(message, &mut writer)
    }

    /// Appends a message to an existing binary writer.
    ///
    /// Allows multiple frames to be encoded into one buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the message fails validation or does
    /// not fit in the writer's remaining capacity.
    #[inline]
    pub fn encode_into(
        &self,
        message: ProtocolMessage,
        writer: &mut ByteWriter<'_>,
    ) -> ProtocolResult<usize> {
        validation::validate_message(message)?;

        let header = message.header()?;
        let required = header.encoded_length();

        writer.ensure_capacity(required)?;

        let start = writer.position();

        header.encode(writer)?;

        wire::encode_payload(message.version, message.envelope.event, writer)?;

        let written = writer.position() - start;

        debug_assert_eq!(written, required);

        Ok(written)
    }

    /// Encodes a canonical event using the current version.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Self::encode`].
    #[inline]
    pub fn encode_event(
        &self,
        envelope: EventEnvelope,
        buffer: &mut [u8],
    ) -> ProtocolResult<usize> {
        self.encode(ProtocolMessage::current(envelope), buffer)
    }

    /// Encodes a message and returns its written byte region.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Self::encode`].
    #[inline]
    pub fn encode_slice<'a>(
        &self,
        message: ProtocolMessage,
        buffer: &'a mut [u8],
    ) -> ProtocolResult<&'a [u8]> {
        let written = self.encode(message, buffer)?;

        Ok(&buffer[..written])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{
        EventHeader, EventId, FeedId, InstrumentId, MarketEvent, OrderAdded, OrderId, Price,
        Quantity, SequenceNumber, SessionId, Side, StreamId,
    };

    use crate::error::ProtocolError;

    fn message() -> ProtocolMessage {
        ProtocolMessage::current(EventEnvelope::new(
            EventHeader::new(
                EventId::new(
                    FeedId::new(StreamId::new(1), SessionId::new(1)),
                    SequenceNumber::new(1),
                ),
                InstrumentId::new(42),
                None,
            ),
            MarketEvent::OrderAdded(OrderAdded {
                order_id: OrderId::new(100),
                price: Price::new(10_000),
                quantity: Quantity::new(50),
                side: Side::Buy,
            }),
        ))
    }

    #[test]
    fn required_size() {
        assert_eq!(Encoder::new().required_size(message()), Ok(65));
    }

    #[test]
    fn insufficient_buffer_unchanged() {
        let mut buffer = [0xAA_u8; 64];

        assert_eq!(
            Encoder::new().encode(message(), &mut buffer),
            Err(ProtocolError::BufferTooSmall {
                needed: 65,
                remaining: 64,
            })
        );

        assert_eq!(buffer, [0xAA; 64]);
    }

    #[test]
    fn exact_capacity() {
        let mut buffer = [0_u8; 65];

        assert_eq!(Encoder::new().encode(message(), &mut buffer), Ok(65));
    }

    #[test]
    fn unused_buffer_unchanged() {
        let mut buffer = [0xAA_u8; 128];

        let written = Encoder::new().encode(message(), &mut buffer).unwrap();

        assert_eq!(written, 65);
        assert!(buffer[written..].iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn encode_slice_length() {
        let mut buffer = [0_u8; 128];

        let encoded = Encoder::new().encode_slice(message(), &mut buffer).unwrap();

        assert_eq!(encoded.len(), 65);
    }
}
