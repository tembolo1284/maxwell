//! Allocation-free binary buffer writer.
//!
//! Writes into caller-owned storage.
//! Failed individual writes do not modify the buffer or cursor.

use crate::error::{ProtocolError, ProtocolResult};

#[derive(Debug)]
pub struct ByteWriter<'a> {
    buffer: &'a mut [u8],
    position: usize,
}

impl<'a> ByteWriter<'a> {
    #[must_use]
    #[inline]
    pub const fn new(buffer: &'a mut [u8]) -> Self {
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
    pub fn written(&self) -> &[u8] {
        &self.buffer[..self.position]
    }

    /// Checks that `count` more bytes fit.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if the remaining capacity is insufficient.
    #[inline]
    pub const fn ensure_capacity(&self, count: usize) -> ProtocolResult<()> {
        let remaining = self.remaining();

        if count > remaining {
            return Err(ProtocolError::BufferTooSmall {
                needed: count,
                remaining,
            });
        }

        Ok(())
    }

    /// Writes raw bytes.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if the remaining capacity is insufficient.
    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) -> ProtocolResult<()> {
        self.ensure_capacity(bytes.len())?;

        let start = self.position;
        let end = start + bytes.len();

        self.buffer[start..end].copy_from_slice(bytes);
        self.position = end;

        Ok(())
    }

    /// Writes one byte.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if the remaining capacity is insufficient.
    #[inline]
    pub fn write_u8(&mut self, value: u8) -> ProtocolResult<()> {
        self.write_bytes(&[value])
    }

    /// Writes a little-endian `u16`.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if the remaining capacity is insufficient.
    #[inline]
    pub fn write_u16(&mut self, value: u16) -> ProtocolResult<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    /// Writes a little-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if the remaining capacity is insufficient.
    #[inline]
    pub fn write_u32(&mut self, value: u32) -> ProtocolResult<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    /// Writes a little-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if the remaining capacity is insufficient.
    #[inline]
    pub fn write_u64(&mut self, value: u64) -> ProtocolResult<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    /// Writes a little-endian `i64`.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if the remaining capacity is insufficient.
    #[inline]
    pub fn write_i64(&mut self, value: i64) -> ProtocolResult<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    /// Writes `count` zero bytes.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if the remaining capacity is insufficient.
    #[inline]
    pub fn write_zeroes(&mut self, count: usize) -> ProtocolResult<()> {
        self.ensure_capacity(count)?;

        let end = self.position + count;

        self.buffer[self.position..end].fill(0);
        self.position = end;

        Ok(())
    }

    #[must_use]
    #[inline]
    pub fn finish(self) -> &'a mut [u8] {
        &mut self.buffer[..self.position]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_little_endian() {
        let mut buffer = [0_u8; 4];
        let mut writer = ByteWriter::new(&mut buffer);

        writer.write_u32(0x1234_5678).unwrap();

        assert_eq!(writer.written(), &[0x78, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn write_signed_integer() {
        let mut buffer = [0_u8; 8];
        let mut writer = ByteWriter::new(&mut buffer);

        writer.write_i64(-500).unwrap();

        assert_eq!(writer.written(), &(-500_i64).to_le_bytes());
    }

    #[test]
    fn insufficient_capacity() {
        let mut buffer = [0xAA_u8; 3];

        {
            let mut writer = ByteWriter::new(&mut buffer);

            assert_eq!(
                writer.write_u64(100),
                Err(ProtocolError::BufferTooSmall {
                    needed: 8,
                    remaining: 3,
                })
            );

            assert_eq!(writer.position(), 0);
        }

        assert_eq!(buffer, [0xAA; 3]);
    }

    #[test]
    fn write_zeroes() {
        let mut buffer = [0xFF_u8; 4];
        let mut writer = ByteWriter::new(&mut buffer);

        writer.write_zeroes(3).unwrap();

        assert_eq!(writer.written(), &[0, 0, 0]);
        assert_eq!(writer.remaining(), 1);
    }

    #[test]
    fn finish_returns_written_region() {
        let mut buffer = [0_u8; 16];
        let mut writer = ByteWriter::new(&mut buffer);

        writer.write_u16(500).unwrap();

        assert_eq!(writer.finish().len(), 2);
    }

    #[test]
    fn empty_write() {
        let mut buffer = [];
        let mut writer = ByteWriter::new(&mut buffer);

        assert_eq!(writer.write_bytes(&[]), Ok(()));
        assert!(writer.write_u8(1).is_err());
    }
}
