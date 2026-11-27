//! Fixed-capacity circular event reorder buffer.
//!
//! Allocation occurs during construction only.
//! Insertion and removal are O(1).
//!
//! Each slot retains the full sequence number through its
//! `EventEnvelope`, preventing circular index aliasing.

use maxwell_core::{EventEnvelope, SequenceNumber};

use crate::config::MAX_REORDER_CAPACITY;
use crate::error::{SequencerError, SequencerResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug)]
pub struct ReorderBuffer {
    slots: Box<[Option<EventEnvelope>]>,
    mask: u64,
    len: usize,
}

impl ReorderBuffer {
    /// Allocates a fixed-capacity reorder buffer.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid capacity or allocation
    /// failure.
    pub fn new(capacity: usize) -> SequencerResult<Self> {
        if capacity == 0 || capacity > MAX_REORDER_CAPACITY || !capacity.is_power_of_two() {
            return Err(SequencerError::InvalidReorderCapacity(capacity));
        }

        let mut slots = Vec::new();

        slots
            .try_reserve_exact(capacity)
            .map_err(|_| SequencerError::AllocationFailed)?;

        slots.resize(capacity, None);

        Ok(Self {
            slots: slots.into_boxed_slice(),
            mask: (capacity - 1) as u64,
            len: 0,
        })
    }

    #[must_use]
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.len == self.capacity()
    }

    #[inline]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the mask is derived from a usize capacity"
    )]
    const fn index(&self, sequence: SequenceNumber) -> usize {
        // The mask is derived from a bounded usize capacity.
        (sequence.raw() & self.mask) as usize
    }

    #[must_use]
    #[inline]
    pub fn contains(&self, sequence: SequenceNumber) -> bool {
        self.get(sequence).is_some()
    }

    #[must_use]
    #[inline]
    pub fn get(&self, sequence: SequenceNumber) -> Option<&EventEnvelope> {
        self.slots[self.index(sequence)]
            .as_ref()
            .filter(|event| event.header.sequence() == sequence)
    }

    /// Inserts a future event within the active window.
    ///
    /// The window is [expected, expected + capacity).
    /// Its upper boundary is evaluated by subtraction
    /// rather than addition to avoid integer overflow.
    ///
    /// The caller must validate feed identity.
    ///
    /// # Errors
    ///
    /// Returns `SequenceOutsideWindow` if the event is beyond the
    /// reorder window, `ConflictingDuplicate` if a different event
    /// holds the sequence, or `BufferCollision` if its slot is taken.
    pub fn insert(
        &mut self,
        expected: SequenceNumber,
        event: EventEnvelope,
    ) -> SequencerResult<InsertOutcome> {
        let received = event.header.sequence();

        let distance = received.raw().checked_sub(expected.raw());

        if distance.is_none_or(|d| d >= self.capacity() as u64) {
            return Err(SequencerError::SequenceOutsideWindow {
                expected,
                received,
                capacity: self.capacity(),
            });
        }

        let index = self.index(received);

        if let Some(existing) = &self.slots[index] {
            let existing_sequence = existing.header.sequence();

            if existing_sequence != received {
                return Err(SequencerError::BufferCollision {
                    existing: existing_sequence,
                    incoming: received,
                });
            }

            if *existing == event {
                return Ok(InsertOutcome::Duplicate);
            }

            return Err(SequencerError::ConflictingDuplicate(received));
        }

        self.slots[index] = Some(event);
        self.len += 1;

        Ok(InsertOutcome::Inserted)
    }

    /// Removes the exact requested sequence, if present.
    #[inline]
    pub fn take(&mut self, sequence: SequenceNumber) -> Option<EventEnvelope> {
        let index = self.index(sequence);

        let matches = self.slots[index]
            .as_ref()
            .is_some_and(|event| event.header.sequence() == sequence);

        if !matches {
            return None;
        }

        let event = self.slots[index].take();

        self.len -= 1;

        event
    }

    /// Finds the nearest buffered sequence at or above
    /// the supplied watermark.
    ///
    /// O(capacity). Intended for recovery diagnostics,
    /// not the steady-state event processing path.
    #[must_use]
    pub fn lowest_sequence(&self, expected: SequenceNumber) -> Option<SequenceNumber> {
        self.slots
            .iter()
            .flatten()
            .map(|event| event.header.sequence())
            .filter(|sequence| *sequence >= expected)
            .min()
    }

    /// Clears all buffered events without reallocating.
    pub fn clear(&mut self) {
        self.slots.fill(None);
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{
        EventHeader, EventId, FeedId, InstrumentId, MarketEvent, OrderDeleted, OrderId, SessionId,
        StreamId,
    };

    fn seq(value: u64) -> SequenceNumber {
        SequenceNumber::new(value)
    }

    fn event(sequence: u64) -> EventEnvelope {
        EventEnvelope::new(
            EventHeader::new(
                EventId::new(
                    FeedId::new(StreamId::new(1), SessionId::new(1)),
                    seq(sequence),
                ),
                InstrumentId::new(42),
                None,
            ),
            MarketEvent::OrderDeleted(OrderDeleted {
                order_id: OrderId::new(sequence),
            }),
        )
    }

    #[test]
    fn insert_and_remove() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        assert_eq!(
            buffer.insert(seq(100), event(102)),
            Ok(InsertOutcome::Inserted),
        );

        assert_eq!(buffer.len(), 1);
        assert!(buffer.contains(seq(102)));
        assert_eq!(buffer.take(seq(102)), Some(event(102)));
        assert!(buffer.is_empty());
    }

    #[test]
    fn duplicate_does_not_increase_length() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        buffer.insert(seq(100), event(102)).unwrap();

        assert_eq!(
            buffer.insert(seq(100), event(102)),
            Ok(InsertOutcome::Duplicate),
        );

        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn conflicting_duplicate_rejected() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        buffer.insert(seq(100), event(102)).unwrap();

        let mut conflicting = event(102);

        conflicting.event = MarketEvent::OrderDeleted(OrderDeleted {
            order_id: OrderId::new(999),
        });

        assert_eq!(
            buffer.insert(seq(100), conflicting),
            Err(SequencerError::ConflictingDuplicate(seq(102))),
        );
    }

    #[test]
    fn window_boundary() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        assert!(buffer.insert(seq(100), event(107)).is_ok());
        assert!(buffer.insert(seq(100), event(108)).is_err());
    }

    #[test]
    fn stale_sequence_rejected() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        assert!(buffer.insert(seq(100), event(99)).is_err());
    }

    #[test]
    fn circular_slot_reuse() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        buffer.insert(seq(100), event(102)).unwrap();
        buffer.take(seq(102)).unwrap();

        buffer.insert(seq(108), event(110)).unwrap();

        assert!(!buffer.contains(seq(102)));
        assert!(buffer.contains(seq(110)));
    }

    #[test]
    fn slot_collision_rejected() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        buffer.insert(seq(100), event(102)).unwrap();

        assert!(matches!(
            buffer.insert(seq(108), event(110)),
            Err(SequencerError::BufferCollision { .. }),
        ));
    }

    #[test]
    fn maximum_sequence_supported() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        buffer.insert(seq(u64::MAX - 2), event(u64::MAX)).unwrap();

        assert!(buffer.take(seq(u64::MAX)).is_some());
    }

    #[test]
    fn clear_preserves_capacity() {
        let mut buffer = ReorderBuffer::new(8).unwrap();

        buffer.insert(seq(100), event(102)).unwrap();
        buffer.clear();

        assert!(buffer.is_empty());
        assert_eq!(buffer.capacity(), 8);
    }
}
