//! Allocation-free market data decoder.
//!
//! Supports decoding individual frames and concatenated
//! frames from caller-owned byte buffers.

use maxwell_core::EventEnvelope;

use crate::error::{ProtocolError, ProtocolResult};
use crate::header::MessageHeader;
use crate::message::ProtocolMessage;
use crate::reader::ByteReader;
use crate::{validation, wire};

#[derive(Debug, Clone, Copy, Default)]
pub struct Decoder;

impl Decoder {
    #[must_use]
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    /// Decodes exactly one complete protocol frame.
    ///
    /// Trailing bytes are rejected.
    #[inline]
    pub fn decode(&self, buffer: &[u8]) -> ProtocolResult<ProtocolMessage> {
        let (message, consumed) = self.decode_prefix(buffer)?;

        if consumed != buffer.len() {
            return Err(ProtocolError::TrailingBytes(buffer.len() - consumed));
        }

        Ok(message)
    }

    /// Decodes the first frame from a buffer.
    ///
    /// Returns the decoded message and consumed byte count.
    /// Remaining bytes are left for subsequent decoding.
    #[inline]
    pub fn decode_prefix(&self, buffer: &[u8]) -> ProtocolResult<(ProtocolMessage, usize)> {
        let mut reader = ByteReader::new(buffer);

        let header = MessageHeader::decode(&mut reader)?;

        validation::validate_header(header)?;

        let payload = reader.read_bytes(usize::from(header.payload_length))?;

        let mut payload_reader = ByteReader::new(payload);

        let event = wire::decode_payload(header.version, header.kind, &mut payload_reader)?;

        payload_reader.finish()?;

        let envelope = EventEnvelope::try_new(header.event_header, event)?;

        let message = ProtocolMessage::new(header.version, envelope);

        Ok((message, reader.position()))
    }

    /// Decodes directly into the canonical domain event.
    #[inline]
    pub fn decode_event(&self, buffer: &[u8]) -> ProtocolResult<EventEnvelope> {
        self.decode(buffer).map(|message| message.envelope)
    }

    /// Returns the validated length of the first frame.
    ///
    /// The complete payload does not need to be present.
    /// Useful for transport-layer framing.
    #[inline]
    pub fn frame_length(&self, buffer: &[u8]) -> ProtocolResult<usize> {
        let mut reader = ByteReader::new(buffer);

        let header = MessageHeader::decode(&mut reader)?;

        validation::validate_header(header)?;

        Ok(header.encoded_length())
    }

    /// Decodes concatenated frames without allocating.
    ///
    /// Iteration stops after the first decoding failure.
    #[must_use]
    #[inline]
    pub fn frames<'a>(&self, buffer: &'a [u8]) -> FrameIter<'a> {
        FrameIter {
            decoder: *self,
            remaining: buffer,
            failed: false,
        }
    }
}

// ============================================================
// Frame iterator
// ============================================================

#[derive(Debug, Clone)]
pub struct FrameIter<'a> {
    decoder: Decoder,
    remaining: &'a [u8],
    failed: bool,
}

impl<'a> FrameIter<'a> {
    #[must_use]
    #[inline]
    pub const fn remaining_bytes(&self) -> &'a [u8] {
        self.remaining
    }
}

impl Iterator for FrameIter<'_> {
    type Item = ProtocolResult<ProtocolMessage>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.remaining.is_empty() {
            return None;
        }

        match self.decoder.decode_prefix(self.remaining) {
            Ok((message, consumed)) => {
                self.remaining = &self.remaining[consumed..];

                Some(Ok(message))
            }

            Err(error) => {
                self.failed = true;

                Some(Err(error))
            }
        }
    }
}

impl core::iter::FusedIterator for FrameIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_rejected() {
        assert!(Decoder::new().decode(&[]).is_err());
    }

    #[test]
    fn empty_iterator() {
        assert!(Decoder::new().frames(&[]).next().is_none());
    }

    #[test]
    fn iterator_stops_after_error() {
        let decoder = Decoder::new();
        let mut frames = decoder.frames(&[0, 0]);

        assert!(matches!(frames.next(), Some(Err(_))));
        assert!(frames.next().is_none());
        assert!(frames.next().is_none());
    }

    #[test]
    fn failed_iterator_preserves_remaining_bytes() {
        let decoder = Decoder::new();
        let mut frames = decoder.frames(&[0, 0]);

        let _ = frames.next();

        assert_eq!(frames.remaining_bytes(), &[0, 0]);
    }
}
