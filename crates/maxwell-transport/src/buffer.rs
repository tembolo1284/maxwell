//! Reusable, fixed-capacity receive buffers.
//!
//! Storage is allocated once during construction.
//! Resetting a buffer does not clear or reallocate memory.

use crate::error::{TransportError, TransportResult};

#[derive(Debug)]
pub struct ReceiveBuffer {
    storage: Box<[u8]>,
    length: usize,
}

impl ReceiveBuffer {
    pub fn new(capacity: usize) -> TransportResult<Self> {
        if capacity == 0 {
            return Err(TransportError::InvalidConfiguration(
                "receive buffer capacity must be positive",
            ));
        }

        let mut storage = Vec::new();

        storage
            .try_reserve_exact(capacity)
            .map_err(|_| TransportError::AllocationFailed)?;

        storage.resize(capacity, 0);

        Ok(Self {
            storage: storage.into_boxed_slice(),
            length: 0,
        })
    }

    #[must_use]
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.storage.len()
    }

    #[must_use]
    #[inline]
    pub const fn len(&self) -> usize {
        self.length
    }

    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    #[must_use]
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.storage[..self.length]
    }

    /// Exposes the entire buffer for a new receive operation.
    ///
    /// Previously committed contents are invalidated.
    #[inline]
    pub fn writable(&mut self) -> &mut [u8] {
        self.length = 0;

        &mut self.storage
    }

    /// Commits the number of bytes received.
    #[inline]
    pub fn commit(&mut self, length: usize) -> TransportResult<()> {
        if length > self.capacity() {
            return Err(TransportError::BufferTooSmall {
                required: length,
                available: self.capacity(),
            });
        }

        self.length = length;

        Ok(())
    }

    #[inline]
    pub fn clear(&mut self) {
        self.length = 0;
    }

    pub fn copy_from(&mut self, data: &[u8]) -> TransportResult<()> {
        if data.len() > self.capacity() {
            return Err(TransportError::BufferTooSmall {
                required: data.len(),
                available: self.capacity(),
            });
        }

        self.storage[..data.len()].copy_from_slice(data);
        self.length = data.len();

        Ok(())
    }
}

impl AsRef<[u8]> for ReceiveBuffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_buffer_is_empty() {
        let buffer = ReceiveBuffer::new(1024).unwrap();

        assert!(buffer.is_empty());
        assert_eq!(buffer.capacity(), 1024);
    }

    #[test]
    fn receive_and_commit() {
        let mut buffer = ReceiveBuffer::new(1024).unwrap();

        buffer.writable()[..4].copy_from_slice(&[1, 2, 3, 4]);

        buffer.commit(4).unwrap();

        assert_eq!(buffer.as_slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn excessive_commit_rejected() {
        let mut buffer = ReceiveBuffer::new(8).unwrap();

        assert!(buffer.commit(9).is_err());
        assert!(buffer.is_empty());
    }

    #[test]
    fn clear_preserves_storage() {
        let mut buffer = ReceiveBuffer::new(128).unwrap();

        buffer.copy_from(&[1, 2, 3]).unwrap();

        let pointer = buffer.as_slice().as_ptr();

        buffer.clear();

        assert_eq!(buffer.capacity(), 128);
        assert_eq!(buffer.as_slice().as_ptr(), pointer);
    }

    #[test]
    fn writable_invalidates_previous_data() {
        let mut buffer = ReceiveBuffer::new(128).unwrap();

        buffer.copy_from(&[1, 2, 3]).unwrap();

        let _ = buffer.writable();

        assert!(buffer.is_empty());
    }

    #[test]
    fn failed_copy_preserves_previous_data() {
        let mut buffer = ReceiveBuffer::new(4).unwrap();

        buffer.copy_from(&[1, 2]).unwrap();

        assert!(buffer.copy_from(&[0; 5]).is_err());
        assert_eq!(buffer.as_slice(), &[1, 2]);
    }
}
