//! Fixed-width binary message header.
//!
//! Header serialization is explicit and independent of
//! Rust struct layout, alignment, or padding.

use maxwell_core::{
    EventHeader, EventId, ExchangeTimestamp, FeedId, InstrumentId, SequenceNumber, SessionId,
    StreamId,
};

use crate::constants::{FLAG_EXCHANGE_TIMESTAMP, HEADER_SIZE, KNOWN_FLAGS, PROTOCOL_MAGIC};

use crate::error::{ProtocolError, ProtocolResult};
use crate::message::MessageKind;
use crate::reader::ByteReader;
use crate::version::ProtocolVersion;
use crate::writer::ByteWriter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    pub version: ProtocolVersion,
    pub kind: MessageKind,
    pub payload_length: u16,
    pub event_header: EventHeader,
}

impl MessageHeader {
    #[must_use]
    #[inline]
    pub const fn new(
        version: ProtocolVersion,
        kind: MessageKind,
        payload_length: u16,
        event_header: EventHeader,
    ) -> Self {
        Self {
            version,
            kind,
            payload_length,
            event_header,
        }
    }

    #[must_use]
    #[inline]
    pub const fn encoded_length(self) -> usize {
        HEADER_SIZE + self.payload_length as usize
    }

    #[must_use]
    #[inline]
    pub const fn flags(self) -> u8 {
        if self.event_header.exchange_timestamp.is_some() {
            FLAG_EXCHANGE_TIMESTAMP
        } else {
            0
        }
    }

    /// Writes the fixed-width header.
    ///
    /// # Errors
    ///
    /// Returns an error if the version is unsupported or the
    /// writer lacks capacity.
    #[inline]
    pub fn encode(self, writer: &mut ByteWriter<'_>) -> ProtocolResult<()> {
        self.version.validate()?;
        writer.ensure_capacity(HEADER_SIZE)?;

        let event = self.event_header;

        writer.write_bytes(&PROTOCOL_MAGIC)?;
        writer.write_u8(self.version.major)?;
        writer.write_u8(self.version.minor)?;
        writer.write_u8(self.kind.code())?;
        writer.write_u8(self.flags())?;
        writer.write_u16(self.payload_length)?;
        writer.write_u32(event.stream_id().raw())?;
        writer.write_u64(event.session_id().raw())?;
        writer.write_u64(event.sequence().raw())?;
        writer.write_u32(event.instrument_id.raw())?;

        let timestamp = event.exchange_timestamp.map_or(0, ExchangeTimestamp::raw);

        writer.write_u64(timestamp)?;

        Ok(())
    }

    /// Reads and validates one fixed-width header.
    ///
    /// # Errors
    ///
    /// Returns `InvalidMagic`, `InvalidFlags`, or
    /// `NonCanonicalEncoding` for a malformed header, or an error if
    /// the input is truncated or the version is unsupported.
    /// The reader is left unchanged on failure.
    #[inline]
    pub fn decode(reader: &mut ByteReader<'_>) -> ProtocolResult<Self> {
        // Parse through a temporary cursor so failures do
        // not consume bytes from the caller's reader.
        let mut cursor = reader.clone();

        let magic = cursor.read_array::<2>()?;

        if magic != PROTOCOL_MAGIC {
            return Err(ProtocolError::InvalidMagic);
        }

        let version = ProtocolVersion::new(cursor.read_u8()?, cursor.read_u8()?);

        version.validate()?;

        let kind = MessageKind::try_from(cursor.read_u8()?)?;
        let flags = cursor.read_u8()?;

        if flags & !KNOWN_FLAGS != 0 {
            return Err(ProtocolError::InvalidFlags(flags));
        }

        let payload_length = cursor.read_u16()?;

        let stream_id = StreamId::new(cursor.read_u32()?);
        let session_id = SessionId::new(cursor.read_u64()?);
        let sequence = SequenceNumber::new(cursor.read_u64()?);
        let instrument_id = InstrumentId::new(cursor.read_u32()?);
        let raw_timestamp = cursor.read_u64()?;

        let exchange_timestamp = if flags & FLAG_EXCHANGE_TIMESTAMP != 0 {
            Some(ExchangeTimestamp::new(raw_timestamp))
        } else {
            if raw_timestamp != 0 {
                return Err(ProtocolError::NonCanonicalEncoding);
            }

            None
        };

        let event_header = EventHeader::new(
            EventId::new(FeedId::new(stream_id, session_id), sequence),
            instrument_id,
            exchange_timestamp,
        );

        *reader = cursor;

        Ok(Self::new(version, kind, payload_length, event_header))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header() -> MessageHeader {
        MessageHeader::new(
            ProtocolVersion::V1,
            MessageKind::OrderAdded,
            25,
            EventHeader::new(
                EventId::new(
                    FeedId::new(StreamId::new(1), SessionId::new(100)),
                    SequenceNumber::new(500),
                ),
                InstrumentId::new(42),
                Some(ExchangeTimestamp::new(1_000_000)),
            ),
        )
    }

    #[test]
    fn header_roundtrip() {
        let original = sample_header();
        let mut buffer = [0_u8; HEADER_SIZE];

        let mut writer = ByteWriter::new(&mut buffer);

        original.encode(&mut writer).unwrap();

        assert_eq!(writer.position(), HEADER_SIZE);

        let mut reader = ByteReader::new(writer.written());

        assert_eq!(MessageHeader::decode(&mut reader), Ok(original));

        assert!(reader.is_empty());
    }

    #[test]
    fn missing_timestamp_roundtrip() {
        let mut original = sample_header();
        original.event_header.exchange_timestamp = None;

        let mut buffer = [0_u8; HEADER_SIZE];
        let mut writer = ByteWriter::new(&mut buffer);

        original.encode(&mut writer).unwrap();

        let mut reader = ByteReader::new(writer.written());

        assert_eq!(MessageHeader::decode(&mut reader), Ok(original));
    }

    #[test]
    fn invalid_magic_preserves_cursor() {
        let mut reader = ByteReader::new(&[0; HEADER_SIZE]);

        assert_eq!(
            MessageHeader::decode(&mut reader),
            Err(ProtocolError::InvalidMagic)
        );

        assert_eq!(reader.position(), 0);
    }

    #[test]
    fn encoded_length() {
        assert_eq!(sample_header().encoded_length(), HEADER_SIZE + 25);
    }
}
