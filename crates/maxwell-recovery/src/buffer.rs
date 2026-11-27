//! Bounded, sequence-ordered recovery buffer.

use std::collections::BTreeMap;

use maxwell_core::{EventEnvelope, FeedId, SequenceNumber};

use crate::error::{RecoveryError, RecoveryResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug)]
pub struct RecoveryBuffer {
    feed: FeedId,
    capacity: usize,
    events: BTreeMap<SequenceNumber, EventEnvelope>,
}

impl RecoveryBuffer {
    /// Creates a bounded buffer for one feed.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfiguration` if `capacity` is zero.
    pub const fn new(feed: FeedId, capacity: usize) -> RecoveryResult<Self> {
        if capacity == 0 {
            return Err(RecoveryError::InvalidConfiguration(
                "recovery buffer capacity must be positive",
            ));
        }

        Ok(Self {
            feed,
            capacity,
            events: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn feed(&self) -> FeedId {
        self.feed
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity
    }

    #[must_use]
    pub fn first_sequence(&self) -> Option<SequenceNumber> {
        self.events.first_key_value().map(|(key, _)| *key)
    }

    #[must_use]
    pub fn last_sequence(&self) -> Option<SequenceNumber> {
        self.events.last_key_value().map(|(key, _)| *key)
    }

    #[must_use]
    pub fn get(&self, sequence: SequenceNumber) -> Option<&EventEnvelope> {
        self.events.get(&sequence)
    }

    #[must_use]
    pub fn contains(&self, sequence: SequenceNumber) -> bool {
        self.events.contains_key(&sequence)
    }

    /// Stores an out-of-order event for later replay.
    ///
    /// # Errors
    ///
    /// Returns `FeedMismatch` or `InvalidEvent` for an event that
    /// does not belong here, `ConflictingEvent` if a different event
    /// already holds the sequence, or `BufferFull` at capacity.
    pub fn insert(&mut self, event: EventEnvelope) -> RecoveryResult<InsertOutcome> {
        if event.feed() != self.feed {
            return Err(RecoveryError::FeedMismatch {
                expected: self.feed,
                received: event.feed(),
            });
        }

        event.validate().map_err(|_| RecoveryError::InvalidEvent)?;

        let sequence = event.sequence();

        if let Some(existing) = self.events.get(&sequence) {
            return if *existing == event {
                Ok(InsertOutcome::Duplicate)
            } else {
                Err(RecoveryError::ConflictingEvent { sequence })
            };
        }

        if self.is_full() {
            return Err(RecoveryError::BufferFull {
                capacity: self.capacity,
            });
        }

        self.events.insert(sequence, event);

        Ok(InsertOutcome::Inserted)
    }

    pub fn remove(&mut self, sequence: SequenceNumber) -> Option<EventEnvelope> {
        self.events.remove(&sequence)
    }

    /// Removes events already represented by a snapshot.
    pub fn discard_through(&mut self, watermark: SequenceNumber) -> usize {
        let previous_length = self.len();

        self.events.retain(|sequence, _| *sequence > watermark);

        previous_length - self.len()
    }

    pub fn discard_before(&mut self, sequence: SequenceNumber) -> usize {
        let previous_length = self.len();

        self.events.retain(|key, _| *key >= sequence);

        previous_length - self.len()
    }

    #[must_use]
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &EventEnvelope> {
        self.events.values()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Explicit session transition.
    ///
    /// No buffered events survive a session change.
    pub fn reset(&mut self, feed: FeedId) {
        self.clear();
        self.feed = feed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{
        EventHeader, EventId, InstrumentId, MarketEvent, SessionId, StreamId, TradingStatus,
        TradingStatusChanged,
    };

    fn feed() -> FeedId {
        FeedId::new(StreamId::new(1), SessionId::new(1))
    }

    fn event(sequence: u64) -> EventEnvelope {
        EventEnvelope::new(
            EventHeader::new(
                EventId::new(feed(), SequenceNumber::new(sequence)),
                InstrumentId::new(1),
                None,
            ),
            MarketEvent::TradingStatusChanged(TradingStatusChanged {
                status: TradingStatus::Open,
            }),
        )
    }

    #[test]
    fn events_are_sorted() {
        let mut buffer = RecoveryBuffer::new(feed(), 10).unwrap();

        for sequence in [5, 2, 4, 1, 3] {
            buffer.insert(event(sequence)).unwrap();
        }

        let sequences: Vec<u64> = buffer.iter().map(|event| event.sequence().raw()).collect();

        assert_eq!(sequences, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn duplicates_do_not_consume_capacity() {
        let mut buffer = RecoveryBuffer::new(feed(), 1).unwrap();

        assert_eq!(buffer.insert(event(1)).unwrap(), InsertOutcome::Inserted,);

        assert_eq!(buffer.insert(event(1)).unwrap(), InsertOutcome::Duplicate,);

        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn capacity_is_enforced() {
        let mut buffer = RecoveryBuffer::new(feed(), 1).unwrap();

        buffer.insert(event(1)).unwrap();

        assert!(matches!(
            buffer.insert(event(2)),
            Err(RecoveryError::BufferFull { .. }),
        ));
    }

    #[test]
    fn discard_snapshot_covered_events() {
        let mut buffer = RecoveryBuffer::new(feed(), 10).unwrap();

        for sequence in 1..=5 {
            buffer.insert(event(sequence)).unwrap();
        }

        assert_eq!(buffer.discard_through(SequenceNumber::new(3)), 3,);

        assert_eq!(buffer.first_sequence(), Some(SequenceNumber::new(4)),);
    }
}
