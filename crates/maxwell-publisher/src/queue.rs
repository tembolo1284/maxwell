//! Bounded, preallocated, single-owner FIFO queue.

use std::collections::VecDeque;

use crate::error::PublisherError;

/// A FIFO with an explicit logical capacity.
///
/// All backing storage is reserved during construction.
/// Successful pushes do not grow the backing allocation.
///
/// This queue is intentionally not a concurrent queue.
/// Synchronization belongs at the owning component boundary.
#[derive(Debug)]
pub struct BoundedQueue<T> {
    entries: VecDeque<T>,
    capacity: usize,
}

impl<T> BoundedQueue<T> {
    pub fn new(capacity: usize) -> Result<Self, PublisherError> {
        if capacity == 0 {
            return Err(PublisherError::InvalidConfig(
                "queue capacity must be greater than zero",
            ));
        }

        let mut entries = VecDeque::new();

        entries
            .try_reserve_exact(capacity)
            .map_err(|_| PublisherError::AllocationFailed)?;

        Ok(Self { entries, capacity })
    }

    /// Insert an item or return ownership to the caller.
    #[inline]
    pub fn try_push(&mut self, item: T) -> Result<(), T> {
        if self.is_full() {
            return Err(item);
        }

        self.entries.push_back(item);
        Ok(())
    }

    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        self.entries.pop_front()
    }

    #[inline]
    pub fn front(&self) -> Option<&T> {
        self.entries.front()
    }

    #[inline]
    pub fn back(&self) -> Option<&T> {
        self.entries.back()
    }

    /// Remove all entries and return the number discarded.
    ///
    /// The backing allocation is retained.
    pub fn clear(&mut self) -> usize {
        let removed = self.entries.len();
        self.entries.clear();
        removed
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.capacity - self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.entries.len() == self.capacity
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &T> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_capacity() {
        assert!(BoundedQueue::<u64>::new(0).is_err());
    }

    #[test]
    fn starts_empty() {
        let queue = BoundedQueue::<u64>::new(4).unwrap();

        assert!(queue.is_empty());
        assert!(!queue.is_full());
        assert_eq!(queue.remaining_capacity(), 4);
    }

    #[test]
    fn preserves_fifo_order() {
        let mut queue = BoundedQueue::new(3).unwrap();

        queue.try_push(10).unwrap();
        queue.try_push(20).unwrap();
        queue.try_push(30).unwrap();

        assert_eq!(queue.pop(), Some(10));
        assert_eq!(queue.pop(), Some(20));
        assert_eq!(queue.pop(), Some(30));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn rejects_push_when_full() {
        let mut queue = BoundedQueue::new(2).unwrap();

        queue.try_push(1).unwrap();
        queue.try_push(2).unwrap();

        assert!(queue.is_full());
        assert_eq!(queue.try_push(3), Err(3));
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn supports_wraparound() {
        let mut queue = BoundedQueue::new(3).unwrap();

        for base in 0..100 {
            queue.try_push(base).unwrap();
            queue.try_push(base + 1).unwrap();

            assert_eq!(queue.pop(), Some(base));

            queue.try_push(base + 2).unwrap();

            assert_eq!(queue.pop(), Some(base + 1));
            assert_eq!(queue.pop(), Some(base + 2));
        }
    }

    #[test]
    fn clear_retains_allocation() {
        let mut queue = BoundedQueue::new(4).unwrap();
        let allocated = queue.entries.capacity();

        queue.try_push(1).unwrap();
        queue.try_push(2).unwrap();

        assert_eq!(queue.clear(), 2);
        assert!(queue.is_empty());
        assert_eq!(queue.entries.capacity(), allocated);
    }

    #[test]
    fn repeated_pushes_do_not_grow_storage() {
        let mut queue = BoundedQueue::new(8).unwrap();
        let allocated = queue.entries.capacity();

        for _ in 0..1000 {
            for value in 0..8 {
                queue.try_push(value).unwrap();
            }

            assert_eq!(queue.entries.capacity(), allocated);

            for value in 0..8 {
                assert_eq!(queue.pop(), Some(value));
            }
        }
    }

    #[test]
    fn supports_non_copy_values() {
        let mut queue = BoundedQueue::new(1).unwrap();

        queue.try_push(String::from("Maxwell")).unwrap();

        assert_eq!(
            queue.try_push(String::from("Planck")),
            Err(String::from("Planck"))
        );

        assert_eq!(queue.pop(), Some(String::from("Maxwell")));
    }
}
