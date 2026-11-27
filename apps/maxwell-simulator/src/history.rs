//! Bounded authoritative event history.
//!
//! Events are recorded in contiguous sequence order,
//! before delivery fault injection.
//!
//! Sequence-based indexing provides constant-time lookup
//! without a secondary hash table.

use std::collections::VecDeque;

use maxwell_core::{
    EventEnvelope, FeedId, SequenceNumber,
};

use crate::error::{SimulatorError, SimulatorResult};

// ============================================================
// History statistics
// ============================================================

/// Cumulative authoritative history statistics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HistoryStats {
    pub recorded: u64,
    pub evicted: u64,
}

// ============================================================
// Authoritative event history
// ============================================================

/// Bounded contiguous event history for a single feed.
///
/// Once capacity is reached, recording a new event
/// evicts the oldest retained event.
#[derive(Debug)]
pub struct EventHistory {
    feed: FeedId,
    events: VecDeque<EventEnvelope>,
    capacity: usize,
    next_expected: SequenceNumber,
    stats: HistoryStats,
}

impl EventHistory {
    /// Creates an empty history beginning at sequence one.
    pub fn new(
        feed: FeedId,
        capacity: usize,
    ) -> SimulatorResult<Self> {
        Self::with_start(
            feed,
            capacity,
            SequenceNumber::ONE,
        )
    }

    /// Creates an empty history with an explicit initial
    /// sequence number.
    pub fn with_start(
        feed: FeedId,
        capacity: usize,
        next_expected: SequenceNumber,
    ) -> SimulatorResult<Self> {
        if capacity == 0 {
            return Err(SimulatorError::InvalidConfig(
                "event history capacity must be positive",
            ));
        }

        if next_expected.raw() == 0 {
            return Err(SimulatorError::InvalidArgument(
                "initial history sequence must be positive",
            ));
        }

        let mut events = VecDeque::new();

        events
            .try_reserve_exact(capacity)
            .map_err(|_| SimulatorError::AllocationFailed)?;

        Ok(Self {
            feed,
            events,
            capacity,
            next_expected,
            stats: HistoryStats::default(),
        })
    }

    // --------------------------------------------------------
    // Observation
    // --------------------------------------------------------

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
        self.events.len() == self.capacity
    }

    #[must_use]
    pub const fn next_expected(&self) -> SequenceNumber {
        self.next_expected
    }

    #[must_use]
    pub const fn stats(&self) -> HistoryStats {
        self.stats
    }

    #[must_use]
    pub fn oldest_sequence(&self) -> Option<SequenceNumber> {
        self.events
            .front()
            .map(|event| event.sequence())
    }

    #[must_use]
    pub fn newest_sequence(&self) -> Option<SequenceNumber> {
        self.events
            .back()
            .map(|event| event.sequence())
    }

    /// Returns the inclusive retained sequence bounds.
    #[must_use]
    pub fn retained_bounds(
        &self,
    ) -> Option<(SequenceNumber, SequenceNumber)> {
        Some((
            self.oldest_sequence()?,
            self.newest_sequence()?,
        ))
    }

    // --------------------------------------------------------
    // Event recording
    // --------------------------------------------------------

    /// Validates whether an event can be recorded.
    ///
    /// History capacity does not cause backpressure because
    /// the oldest retained event may be evicted.
    pub fn validate_next(
        &self,
        event: &EventEnvelope,
    ) -> SimulatorResult<()> {
        if event.header.feed() != self.feed {
            return Err(
                SimulatorError::InvalidMarketOperation(
                    "event feed does not match history feed",
                ),
            );
        }

        if event.sequence() != self.next_expected {
            return Err(SimulatorError::UnexpectedSequence {
                expected: self.next_expected,
                actual: event.sequence(),
            });
        }

        if event.sequence().raw() == u64::MAX {
            return Err(SimulatorError::SequenceExhausted);
        }

        Ok(())
    }

    /// Records one authoritative event.
    ///
    /// Returns the evicted event, if any.
    ///
    /// Validation occurs before any history mutation.
    pub fn record(
        &mut self,
        event: EventEnvelope,
    ) -> SimulatorResult<Option<EventEnvelope>> {
        self.validate_next(&event)?;

        let next = event.sequence().raw() + 1;

        let evicted = if self.is_full() {
            let oldest = self.events.pop_front();

            self.stats.evicted =
                self.stats.evicted.saturating_add(1);

            oldest
        } else {
            None
        };

        self.events.push_back(event);

        self.next_expected = SequenceNumber::new(next);

        self.stats.recorded =
            self.stats.recorded.saturating_add(1);

        Ok(evicted)
    }

    // --------------------------------------------------------
    // Individual event lookup
    // --------------------------------------------------------

    fn offset(
        &self,
        sequence: SequenceNumber,
    ) -> Option<usize> {
        let oldest = self.oldest_sequence()?;

        let distance = sequence
            .raw()
            .checked_sub(oldest.raw())?;

        let offset = usize::try_from(distance).ok()?;

        if offset >= self.events.len() {
            return None;
        }

        Some(offset)
    }

    /// Returns whether a sequence is currently retained.
    #[must_use]
    pub fn contains(
        &self,
        sequence: SequenceNumber,
    ) -> bool {
        self.offset(sequence).is_some()
    }

    /// Returns a retained event by its authoritative sequence.
    #[must_use]
    pub fn get(
        &self,
        sequence: SequenceNumber,
    ) -> Option<&EventEnvelope> {
        let offset = self.offset(sequence)?;

        self.events.get(offset)
    }

    // --------------------------------------------------------
    // Retransmission ranges
    // --------------------------------------------------------

    /// Returns whether an entire inclusive sequence range
    /// is currently retained.
    #[must_use]
    pub fn contains_range(
        &self,
        start: SequenceNumber,
        end: SequenceNumber,
    ) -> bool {
        start.raw() != 0
            && start <= end
            && self.contains(start)
            && self.contains(end)
    }

    /// Returns an allocation-free iterator over an inclusive
    /// authoritative sequence range.
    ///
    /// The entire range must be retained. Partial recovery
    /// ranges are never silently returned.
    pub fn range(
        &self,
        start: SequenceNumber,
        end: SequenceNumber,
    ) -> SimulatorResult<HistoryRange<'_>> {
        if start.raw() == 0 || start > end {
            return Err(SimulatorError::InvalidHistoryRange {
                start,
                end,
            });
        }

        let unavailable = || {
            SimulatorError::HistoryUnavailable {
                start,
                end,
            }
        };

        let first = self.offset(start)
            .ok_or_else(unavailable)?;

        let last = self.offset(end)
            .ok_or_else(unavailable)?;

        Ok(HistoryRange {
            inner: self.events.range(first..=last),
        })
    }

    /// Returns all retained events in authoritative order.
    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = &EventEnvelope>
           + DoubleEndedIterator
           + '_ {
        self.events.iter()
    }

    // --------------------------------------------------------
    // Diagnostics
    // --------------------------------------------------------

    /// Validates history capacity, feed identity, and
    /// contiguous sequence ordering.
    ///
    /// Intended for tests and diagnostics.
    pub fn validate_consistency(&self) -> SimulatorResult<()> {
        if self.events.len() > self.capacity {
            return Err(SimulatorError::InvariantViolation(
                "history capacity exceeded",
            ));
        }

        let Some(oldest) = self.oldest_sequence() else {
            return Ok(());
        };

        let mut expected = oldest.raw();

        for event in &self.events {
            if event.header.feed() != self.feed {
                return Err(
                    SimulatorError::InvariantViolation(
                        "history contains a foreign feed",
                    ),
                );
            }

            if event.sequence().raw() != expected {
                return Err(
                    SimulatorError::InvariantViolation(
                        "history sequence is not contiguous",
                    ),
                );
            }

            expected = expected
                .checked_add(1)
                .ok_or(SimulatorError::SequenceExhausted)?;
        }

        if expected != self.next_expected.raw() {
            return Err(SimulatorError::InvariantViolation(
                "history cursor does not follow newest event",
            ));
        }

        Ok(())
    }
}

// ============================================================
// History range iterator
// ============================================================

/// Borrowed allocation-free retransmission range.
#[derive(Debug)]
pub struct HistoryRange<'a> {
    inner: std::collections::vec_deque::Iter<
        'a,
        EventEnvelope,
    >,
}

impl<'a> Iterator for HistoryRange<'a> {
    type Item = &'a EventEnvelope;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for HistoryRange<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back()
    }
}

impl ExactSizeIterator for HistoryRange<'_> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl std::iter::FusedIterator for HistoryRange<'_> {}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{
        EventHeader, EventId, InstrumentId, MarketEvent,
        OrderAdded, OrderId, Price, Quantity, SessionId,
        Side, StreamId,
    };

    fn feed() -> FeedId {
        FeedId::new(
            StreamId::new(1),
            SessionId::new(1),
        )
    }

    fn sequence(value: u64) -> SequenceNumber {
        SequenceNumber::new(value)
    }

    fn event(value: u64) -> EventEnvelope {
        EventEnvelope::new(
            EventHeader::new(
                EventId::new(
                    feed(),
                    sequence(value),
                ),
                InstrumentId::new(1),
                None,
            ),
            MarketEvent::OrderAdded(OrderAdded {
                order_id: OrderId::new(value),
                side: Side::Buy,
                price: Price::new(100),
                quantity: Quantity::new(10),
            }),
        )
    }

    fn history(capacity: usize) -> EventHistory {
        EventHistory::new(feed(), capacity).unwrap()
    }

    #[test]
    fn history_starts_empty() {
        let history = history(4);

        assert!(history.is_empty());
        assert_eq!(history.capacity(), 4);
        assert_eq!(history.oldest_sequence(), None);
        assert_eq!(history.newest_sequence(), None);
        assert_eq!(history.retained_bounds(), None);

        assert_eq!(
            history.next_expected(),
            SequenceNumber::ONE,
        );
    }

    #[test]
    fn zero_capacity_is_rejected() {
        assert!(EventHistory::new(feed(), 0).is_err());
    }

    #[test]
    fn zero_initial_sequence_is_rejected() {
        assert!(
            EventHistory::with_start(
                feed(),
                4,
                sequence(0),
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_initial_sequence_is_supported() {
        let mut history = EventHistory::with_start(
            feed(),
            4,
            sequence(100),
        )
        .unwrap();

        history.record(event(100)).unwrap();

        assert_eq!(
            history.oldest_sequence(),
            Some(sequence(100)),
        );

        assert_eq!(
            history.next_expected(),
            sequence(101),
        );
    }

    #[test]
    fn contiguous_recording_preserves_events() {
        let mut history = history(4);

        for value in 1..=4 {
            assert!(
                history.record(event(value)).unwrap().is_none()
            );
        }

        assert!(history.is_full());

        assert_eq!(
            history.retained_bounds(),
            Some((sequence(1), sequence(4))),
        );

        assert_eq!(
            history.next_expected(),
            sequence(5),
        );

        assert_eq!(history.stats().recorded, 4);
        assert_eq!(history.stats().evicted, 0);

        history.validate_consistency().unwrap();
    }

    #[test]
    fn full_history_evicts_oldest_event() {
        let mut history = history(3);

        for value in 1..=3 {
            history.record(event(value)).unwrap();
        }

        let evicted =
            history.record(event(4)).unwrap().unwrap();

        assert_eq!(evicted.sequence(), sequence(1));
        assert_eq!(history.len(), 3);

        assert_eq!(
            history.retained_bounds(),
            Some((sequence(2), sequence(4))),
        );

        assert_eq!(history.stats().evicted, 1);
        assert_eq!(history.stats().recorded, 4);
    }

    #[test]
    fn repeated_eviction_preserves_contiguous_history() {
        let mut history = history(8);

        for value in 1..=1_000 {
            history.record(event(value)).unwrap();
        }

        assert_eq!(history.len(), 8);

        assert_eq!(
            history.retained_bounds(),
            Some((sequence(993), sequence(1_000))),
        );

        assert_eq!(history.stats().evicted, 992);

        history.validate_consistency().unwrap();
    }

    #[test]
    fn capacity_one_retains_latest_event() {
        let mut history = history(1);

        for value in 1..=10 {
            history.record(event(value)).unwrap();
        }

        assert_eq!(history.len(), 1);

        assert_eq!(
            history.get(sequence(10)),
            Some(&event(10)),
        );

        assert_eq!(history.stats().evicted, 9);
    }

    #[test]
    fn sequence_gap_is_rejected_atomically() {
        let mut history = history(4);

        history.record(event(1)).unwrap();

        assert!(matches!(
            history.record(event(3)),
            Err(SimulatorError::UnexpectedSequence { .. }),
        ));

        assert_eq!(history.len(), 1);
        assert_eq!(history.next_expected(), sequence(2));
        assert_eq!(history.stats().recorded, 1);
    }

    #[test]
    fn duplicate_sequence_is_rejected() {
        let mut history = history(4);

        history.record(event(1)).unwrap();

        assert!(matches!(
            history.record(event(1)),
            Err(SimulatorError::UnexpectedSequence { .. }),
        ));

        assert_eq!(history.len(), 1);
    }

    #[test]
    fn foreign_feed_is_rejected() {
        let mut history = history(4);

        let mut foreign = event(1);

        foreign.header.id.feed = FeedId::new(
            StreamId::new(2),
            SessionId::new(1),
        );

        assert!(history.record(foreign).is_err());
        assert!(history.is_empty());
    }

    #[test]
    fn exhausted_sequence_is_rejected_before_mutation() {
        let mut history = EventHistory::with_start(
            feed(),
            4,
            sequence(u64::MAX),
        )
        .unwrap();

        assert!(matches!(
            history.record(event(u64::MAX)),
            Err(SimulatorError::SequenceExhausted),
        ));

        assert!(history.is_empty());
    }

    #[test]
    fn individual_lookup_works_after_wraparound() {
        let mut history = history(4);

        for value in 1..=10 {
            history.record(event(value)).unwrap();
        }

        for value in 7..=10 {
            assert_eq!(
                history.get(sequence(value)),
                Some(&event(value)),
            );
        }

        assert!(history.get(sequence(6)).is_none());
        assert!(history.get(sequence(11)).is_none());
    }

    #[test]
    fn inclusive_range_returns_all_requested_events() {
        let mut history = history(8);

        for value in 1..=10 {
            history.record(event(value)).unwrap();
        }

        let values: Vec<_> = history
            .range(sequence(4), sequence(8))
            .unwrap()
            .map(|event| event.sequence().raw())
            .collect();

        assert_eq!(values, vec![4, 5, 6, 7, 8]);
    }

    #[test]
    fn range_supports_single_event() {
        let mut history = history(4);

        history.record(event(1)).unwrap();

        let mut range = history
            .range(sequence(1), sequence(1))
            .unwrap();

        assert_eq!(range.len(), 1);
        assert_eq!(range.next(), Some(&event(1)));
        assert_eq!(range.len(), 0);
        assert_eq!(range.next(), None);
    }

    #[test]
    fn partially_evicted_range_is_rejected() {
        let mut history = history(4);

        for value in 1..=8 {
            history.record(event(value)).unwrap();
        }

        assert!(matches!(
            history.range(sequence(4), sequence(6)),
            Err(SimulatorError::HistoryUnavailable { .. }),
        ));
    }

    #[test]
    fn range_extending_beyond_newest_event_is_rejected() {
        let mut history = history(4);

        history.record(event(1)).unwrap();

        assert!(matches!(
            history.range(sequence(1), sequence(2)),
            Err(SimulatorError::HistoryUnavailable { .. }),
        ));
    }

    #[test]
    fn reversed_range_is_rejected() {
        let history = history(4);

        assert!(matches!(
            history.range(sequence(5), sequence(1)),
            Err(SimulatorError::InvalidHistoryRange { .. }),
        ));
    }

    #[test]
    fn zero_start_range_is_rejected() {
        let history = history(4);

        assert!(matches!(
            history.range(sequence(0), sequence(1)),
            Err(SimulatorError::InvalidHistoryRange { .. }),
        ));
    }

    #[test]
    fn range_supports_reverse_iteration() {
        let mut history = history(4);

        for value in 1..=4 {
            history.record(event(value)).unwrap();
        }

        let values: Vec<_> = history
            .range(sequence(2), sequence(4))
            .unwrap()
            .rev()
            .map(|event| event.sequence().raw())
            .collect();

        assert_eq!(values, vec![4, 3, 2]);
    }

    #[test]
    fn contains_range_requires_complete_retention() {
        let mut history = history(4);

        for value in 1..=8 {
            history.record(event(value)).unwrap();
        }

        assert!(
            history.contains_range(
                sequence(5),
                sequence(8),
            )
        );

        assert!(
            !history.contains_range(
                sequence(4),
                sequence(8),
            )
        );

        assert!(
            !history.contains_range(
                sequence(5),
                sequence(9),
            )
        );
    }

    #[test]
    fn full_history_iteration_is_ordered() {
        let mut history = history(4);

        for value in 1..=8 {
            history.record(event(value)).unwrap();
        }

        let values: Vec<_> = history
            .iter()
            .map(|event| event.sequence().raw())
            .collect();

        assert_eq!(values, vec![5, 6, 7, 8]);
    }
}
