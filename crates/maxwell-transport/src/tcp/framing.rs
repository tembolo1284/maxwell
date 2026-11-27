
//! Incremental length-prefixed TCP framing.
//!
//! Wire format:
//!
//! [4-byte big-endian payload length][payload]
//!
//! Decoder and encoder storage is preallocated.
//! No allocation is required for ordinary frame processing.

use crate::config::FramingConfig;
use crate::error::{TransportError, TransportResult};
use crate::transport::WriteOutcome;

use super::connection::TcpConnection;

pub const FRAME_HEADER_SIZE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStatus {
    NeedMore {
        consumed: usize,
    },

    Ready {
        consumed: usize,
    },
}

impl FrameStatus {
    #[must_use]
    pub const fn consumed(self) -> usize {
        match self {
            Self::NeedMore { consumed }
            | Self::Ready { consumed } => consumed,
        }
    }

    #[must_use]
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

fn allocate_buffer(length: usize) -> TransportResult<Box<[u8]>> {
    let mut storage = Vec::new();

    storage
        .try_reserve_exact(length)
        .map_err(|_| TransportError::AllocationFailed)?;

    storage.resize(length, 0);

    Ok(storage.into_boxed_slice())
}

// ============================================================
// Frame decoder
// ============================================================

#[derive(Debug)]
pub struct FrameDecoder {
    config: FramingConfig,

    header: [u8; FRAME_HEADER_SIZE],
    header_length: usize,

    payload: Box<[u8]>,
    payload_length: usize,

    expected_length: Option<usize>,

    ready: bool,
    poisoned: bool,
}

impl FrameDecoder {
    pub fn new(config: FramingConfig) -> TransportResult<Self> {
        config.validate()?;

        Ok(Self {
            config,
            header: [0; FRAME_HEADER_SIZE],
            header_length: 0,
            payload: allocate_buffer(config.max_frame_size)?,
            payload_length: 0,
            expected_length: None,
            ready: false,
            poisoned: false,
        })
    }

    #[must_use]
    pub const fn config(&self) -> FramingConfig {
        self.config
    }

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    #[must_use]
    pub const fn is_partial(&self) -> bool {
        !self.ready
            && (self.header_length != 0
                || self.expected_length.is_some())
    }

    #[must_use]
    pub const fn expected_length(&self) -> Option<usize> {
        self.expected_length
    }

    #[must_use]
    pub const fn received_payload_length(&self) -> usize {
        self.payload_length
    }

    /// Returns the complete frame without copying.
    #[must_use]
    pub fn frame(&self) -> Option<&[u8]> {
        if self.ready {
            Some(&self.payload[..self.payload_length])
        } else {
            None
        }
    }

    /// Consumes bytes until either one frame is complete
    /// or the input has been exhausted.
    ///
    /// The caller must retain input[consumed..].
    ///
    /// A ready frame must be processed and released
    /// before decoding the next frame.
    pub fn push(
        &mut self,
        input: &[u8],
    ) -> TransportResult<FrameStatus> {
        if self.poisoned {
            return Err(TransportError::DecoderPoisoned);
        }

        if self.ready {
            return Ok(FrameStatus::Ready {
                consumed: 0,
            });
        }

        let mut consumed = 0;

        // ----------------------------------------------------
        // Accumulate the four-byte frame header.
        // ----------------------------------------------------

        if self.header_length < FRAME_HEADER_SIZE {
            let remaining =
                FRAME_HEADER_SIZE - self.header_length;

            let count = remaining.min(input.len());

            self.header[
                self.header_length..self.header_length + count
            ]
            .copy_from_slice(&input[..count]);

            self.header_length += count;
            consumed += count;

            if self.header_length < FRAME_HEADER_SIZE {
                return Ok(FrameStatus::NeedMore {
                    consumed,
                });
            }

            let expected =
                u32::from_be_bytes(self.header) as usize;

            if expected > self.config.max_frame_size {
                self.poisoned = true;

                return Err(TransportError::FrameTooLarge {
                    actual: expected,
                    maximum: self.config.max_frame_size,
                });
            }

            if expected == 0 && !self.config.allow_empty_frames {
                self.poisoned = true;

                return Err(TransportError::EmptyFrame);
            }

            self.expected_length = Some(expected);

            if expected == 0 {
                self.ready = true;

                return Ok(FrameStatus::Ready {
                    consumed,
                });
            }
        }

        // ----------------------------------------------------
        // Accumulate the frame payload.
        // ----------------------------------------------------

        let expected = self.expected_length
            .ok_or(TransportError::DecoderPoisoned)?;

        let remaining = expected - self.payload_length;

        let available = input.len() - consumed;
        let count = remaining.min(available);

        self.payload[
            self.payload_length..self.payload_length + count
        ]
        .copy_from_slice(
            &input[consumed..consumed + count]
        );

        self.payload_length += count;
        consumed += count;

        if self.payload_length == expected {
            self.ready = true;

            Ok(FrameStatus::Ready {
                consumed,
            })
        } else {
            Ok(FrameStatus::NeedMore {
                consumed,
            })
        }
    }

    /// Releases a successfully processed frame.
    ///
    /// Returns false when no complete frame is available.
    pub fn release(&mut self) -> bool {
        if !self.ready {
            return false;
        }

        self.reset();

        true
    }

    /// Checks whether TCP EOF occurred at a valid
    /// application frame boundary.
    pub fn finish(&self) -> TransportResult<()> {
        if self.poisoned {
            return Err(TransportError::DecoderPoisoned);
        }

        if self.is_partial() {
            return Err(TransportError::TruncatedFrame);
        }

        Ok(())
    }

    /// Resets the decoder without reallocating storage.
    ///
    /// A malformed frame poisons the current stream.
    /// Reset should normally accompany a new connection,
    /// not an attempt to guess the next frame boundary.
    pub fn reset(&mut self) {
        self.header_length = 0;
        self.payload_length = 0;
        self.expected_length = None;
        self.ready = false;
        self.poisoned = false;
    }
}

// ============================================================
// Frame encoder
// ============================================================

#[derive(Debug)]
pub struct FrameEncoder {
    config: FramingConfig,

    storage: Box<[u8]>,

    length: usize,
    offset: usize,
}

impl FrameEncoder {
    pub fn new(config: FramingConfig) -> TransportResult<Self> {
        config.validate()?;

        let capacity =
            FRAME_HEADER_SIZE + config.max_frame_size;

        Ok(Self {
            config,
            storage: allocate_buffer(capacity)?,
            length: 0,
            offset: 0,
        })
    }

    #[must_use]
    pub const fn config(&self) -> FramingConfig {
        self.config
    }

    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.offset < self.length
    }

    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.length - self.offset
    }

    #[must_use]
    pub const fn written(&self) -> usize {
        self.offset
    }

    /// Encodes one payload into preallocated storage.
    ///
    /// An existing partially written frame cannot be
    /// overwritten.
    pub fn prepare(
        &mut self,
        payload: &[u8],
    ) -> TransportResult<()> {
        if self.is_pending() {
            return Err(TransportError::EncoderBusy);
        }

        if payload.len() > self.config.max_frame_size {
            return Err(TransportError::FrameTooLarge {
                actual: payload.len(),
                maximum: self.config.max_frame_size,
            });
        }

        if payload.is_empty() && !self.config.allow_empty_frames {
            return Err(TransportError::EmptyFrame);
        }

        let length = u32::try_from(payload.len())
            .map_err(|_| TransportError::FrameTooLarge {
                actual: payload.len(),
                maximum: u32::MAX as usize,
            })?;

        self.storage[..FRAME_HEADER_SIZE]
            .copy_from_slice(&length.to_be_bytes());

        let total = FRAME_HEADER_SIZE + payload.len();

        self.storage[FRAME_HEADER_SIZE..total]
            .copy_from_slice(payload);

        self.length = total;
        self.offset = 0;

        Ok(())
    }

    /// Returns only bytes that have not yet been written.
    #[must_use]
    #[inline]
    pub fn pending_bytes(&self) -> &[u8] {
        &self.storage[self.offset..self.length]
    }

    /// Advances the write offset after a successful write.
    #[inline]
    pub fn advance(
        &mut self,
        written: usize,
    ) -> TransportResult<()> {
        if written > self.remaining() {
            return Err(TransportError::InvalidWriteProgress);
        }

        self.offset += written;

        Ok(())
    }

    /// Attempts one nonblocking socket write.
    ///
    /// WouldBlock leaves the write offset unchanged.
    pub fn flush_once(
        &mut self,
        connection: &mut TcpConnection,
    ) -> TransportResult<WriteOutcome> {
        if !self.is_pending() {
            return Ok(WriteOutcome::Written(0));
        }

        let outcome = connection.write(self.pending_bytes())?;

        if let WriteOutcome::Written(length) = outcome {
            self.advance(length)?;
        }

        Ok(outcome)
    }

    /// Discards the current frame.
    ///
    /// If any prefix was already transmitted, the caller
    /// must close the connection before resetting.
    pub fn reset(&mut self) {
        self.length = 0;
        self.offset = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> FramingConfig {
        FramingConfig {
            max_frame_size: 1024,
            allow_empty_frames: false,
        }
    }

    #[test]
    fn encode_big_endian_header() {
        let mut encoder = FrameEncoder::new(config()).unwrap();

        encoder.prepare(b"hello").unwrap();

        assert_eq!(
            encoder.pending_bytes(),
            b"\x00\x00\x00\x05hello",
        );
    }

    #[test]
    fn decode_complete_frame() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        let result = decoder
            .push(b"\x00\x00\x00\x05hello")
            .unwrap();

        assert!(result.is_ready());
        assert_eq!(result.consumed(), 9);
        assert_eq!(decoder.frame(), Some(&b"hello"[..]));
    }

    #[test]
    fn fragmented_header() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        assert!(!decoder.push(&[0, 0]).unwrap().is_ready());
        assert!(!decoder.push(&[0, 3]).unwrap().is_ready());

        assert!(decoder.push(b"abc").unwrap().is_ready());
        assert_eq!(decoder.frame(), Some(&b"abc"[..]));
    }

    #[test]
    fn fragmented_payload() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        decoder.push(b"\x00\x00\x00\x05he").unwrap();

        assert!(decoder.is_partial());
        assert_eq!(decoder.received_payload_length(), 2);

        decoder.push(b"llo").unwrap();

        assert_eq!(decoder.frame(), Some(&b"hello"[..]));
    }

    #[test]
    fn oversized_frame_poisons_decoder() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        let result = decoder.push(&2048_u32.to_be_bytes());

        assert!(matches!(
            result,
            Err(TransportError::FrameTooLarge { .. }),
        ));

        assert!(decoder.is_poisoned());

        assert!(matches!(
            decoder.push(b"abc"),
            Err(TransportError::DecoderPoisoned),
        ));
    }

    #[test]
    fn empty_frame_rejected() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        assert!(matches!(
            decoder.push(&[0; 4]),
            Err(TransportError::EmptyFrame),
        ));
    }

    #[test]
    fn empty_frame_allowed() {
        let mut decoder = FrameDecoder::new(FramingConfig {
            allow_empty_frames: true,
            ..config()
        })
        .unwrap();

        assert!(decoder.push(&[0; 4]).unwrap().is_ready());
        assert_eq!(decoder.frame(), Some(&[][..]));
    }

    #[test]
    fn partial_write_progress() {
        let mut encoder = FrameEncoder::new(config()).unwrap();

        encoder.prepare(b"hello").unwrap();
        encoder.advance(4).unwrap();

        assert_eq!(encoder.pending_bytes(), b"hello");

        encoder.advance(2).unwrap();

        assert_eq!(encoder.pending_bytes(), b"llo");

        encoder.advance(3).unwrap();

        assert!(!encoder.is_pending());
    }

    #[test]
    fn pending_frame_cannot_be_overwritten() {
        let mut encoder = FrameEncoder::new(config()).unwrap();

        encoder.prepare(b"first").unwrap();

        assert!(matches!(
            encoder.prepare(b"second"),
            Err(TransportError::EncoderBusy),
        ));
    }

    #[test]
    fn invalid_write_progress_rejected() {
        let mut encoder = FrameEncoder::new(config()).unwrap();

        encoder.prepare(b"hello").unwrap();

        assert!(encoder.advance(10).is_err());
        assert_eq!(encoder.remaining(), 9);
    }

    #[test]
    fn eof_during_header() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        decoder.push(&[0, 0]).unwrap();

        assert!(matches!(
            decoder.finish(),
            Err(TransportError::TruncatedFrame),
        ));
    }

    #[test]
    fn eof_during_payload() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        decoder.push(b"\x00\x00\x00\x05he").unwrap();

        assert!(matches!(
            decoder.finish(),
            Err(TransportError::TruncatedFrame),
        ));
    }

    #[test]
    fn eof_at_frame_boundary() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        decoder.push(b"\x00\x00\x00\x03abc").unwrap();

        assert!(decoder.finish().is_ok());
        assert!(decoder.release());
        assert!(decoder.finish().is_ok());
    }
}

