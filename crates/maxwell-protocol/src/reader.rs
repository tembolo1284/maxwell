//! Allocation-free binary buffer reader.
//!
//! Failed reads leave the cursor unchanged.
//! Multibyte integers use little-endian encoding.

use crate::error::{ProtocolError, ProtocolResult};

#[derive(Debug, Clone)]
pub struct ByteReader<'a> {
    buffer: &'a [u8],
    position: usize,
}

impl<'a> ByteReader<'a> {
    #[must_use]
    #[inline]
    pub const fn new(buffer: &'a [u8]) -> Self {
        Self {
            buffer,
            position: 0,
        }
    }

    #[must_use]
    #[inline]
    pub const fn position(&self) -> usize {
        self.position
    }

    #[must_use]
    #[inline]
    pub const fn remaining(&self) -> usize {
        self.buffer.len() - self.position
    }

    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    #[must_use]
    #[inline]
    pub fn remaining_bytes(&self) -> &'a [u8] {
        &self.buffer[self.position..]
    }

    /// Reads `count` bytes and advances the cursor.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEnd` if fewer bytes remain than requested.
    #[inline]
    pub fn read_bytes(&mut self, count: usize) -> ProtocolResult<&'a [u8]> {
        let remaining = self.remaining();

        if count > remaining {
            return Err(ProtocolError::UnexpectedEnd {
                needed: count,
                remaining,
            });
        }

        let start = self.position;
        let end = start + count;

        self.position = end;

        Ok(&self.buffer[start..end])
    }

    /// Reads a fixed-size byte array.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEnd` if fewer bytes remain than requested.
    #[inline]
    pub fn read_array<const N: usize>(&mut self) -> ProtocolResult<[u8; N]> {
        let bytes = self.read_bytes(N)?;
        let mut result = [0_u8; N];

        result.copy_from_slice(bytes);

        Ok(result)
    }

    /// Reads one byte.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEnd` if fewer bytes remain than requested.
    #[inline]
    pub fn read_u8(&mut self) -> ProtocolResult<u8> {
        Ok(self.read_array::<1>()?[0])
    }

    /// Reads a little-endian `u16`.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEnd` if fewer bytes remain than requested.
    #[inline]
    pub fn read_u16(&mut self) -> ProtocolResult<u16> {
        Ok(u16::from_le_bytes(self.read_array()?))
    }

    /// Reads a little-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEnd` if fewer bytes remain than requested.
    #[inline]
    pub fn read_u32(&mut self) -> ProtocolResult<u32> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    /// Reads a little-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEnd` if fewer bytes remain than requested.
    #[inline]
    pub fn read_u64(&mut self) -> ProtocolResult<u64> {
        Ok(u64::from_le_bytes(self.read_array()?))
    }

    /// Reads a little-endian `i64`.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEnd` if fewer bytes remain than requested.
    #[inline]
    pub fn read_i64(&mut self) -> ProtocolResult<i64> {
        Ok(i64::from_le_bytes(self.read_array()?))
    }

    /// Advances the cursor by `count` bytes.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEnd` if fewer bytes remain than requested.
    #[inline]
    pub fn skip(&mut self, count: usize) -> ProtocolResult<()> {
        self.read_bytes(count)?;
        Ok(())
    }

    /// Asserts that every byte has been consumed.
    ///
    /// # Errors
    ///
    /// Returns `TrailingBytes` if unread bytes remain.
    #[inline]
    pub const fn finish(self) -> ProtocolResult<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(ProtocolError::TrailingBytes(self.remaining()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_little_endian() {
        let mut reader = ByteReader::new(&[0x78, 0x56, 0x34, 0x12]);

        assert_eq!(reader.read_u32(), Ok(0x1234_5678));
        assert!(reader.is_empty());
    }

    #[test]
    fn read_signed_integer() {
        let bytes = (-500_i64).to_le_bytes();
        let mut reader = ByteReader::new(&bytes);

        assert_eq!(reader.read_i64(), Ok(-500));
    }

    #[test]
    fn failed_read_preserves_position() {
        let mut reader = ByteReader::new(&[1, 2, 3]);

        assert_eq!(
            reader.read_u64(),
            Err(ProtocolError::UnexpectedEnd {
                needed: 8,
                remaining: 3,
            })
        );

        assert_eq!(reader.position(), 0);
        assert_eq!(reader.read_u8(), Ok(1));
    }

    #[test]
    fn read_empty_buffer() {
        let mut reader = ByteReader::new(&[]);

        assert!(reader.read_u8().is_err());
        assert!(reader.is_empty());
    }

    #[test]
    fn oversized_read() {
        let mut reader = ByteReader::new(&[1]);

        assert!(reader.read_bytes(usize::MAX).is_err());
        assert_eq!(reader.position(), 0);
    }

    #[test]
    fn skip_bytes() {
        let mut reader = ByteReader::new(&[1, 2, 3]);

        assert_eq!(reader.skip(2), Ok(()));
        assert_eq!(reader.read_u8(), Ok(3));
    }

    #[test]
    fn trailing_bytes() {
        let reader = ByteReader::new(&[1, 2]);

        assert_eq!(reader.finish(), Err(ProtocolError::TrailingBytes(2)));
    }

    #[test]
    fn zero_length_read() {
        let mut reader = ByteReader::new(&[1]);

        assert_eq!(reader.read_bytes(0), Ok(&[][..]));
        assert_eq!(reader.position(), 0);
    }
}
