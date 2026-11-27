//! Connection-local transport sequence tracking.

use crate::error::ClientError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceStatus {
    InOrder,

    Duplicate { last_accepted: u64, received: u64 },

    Gap { expected: u64, received: u64 },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SequenceTracker {
    last_accepted: u64,
}

impl SequenceTracker {
    #[inline]
    pub const fn new() -> Self {
        Self { last_accepted: 0 }
    }

    #[inline]
    pub const fn last_accepted(&self) -> u64 {
        self.last_accepted
    }

    #[inline]
    pub fn next_expected(&self) -> Option<u64> {
        self.last_accepted.checked_add(1)
    }

    #[inline]
    pub fn reset(&mut self) {
        self.last_accepted = 0;
    }

    /// Classify a sequence without changing tracker state.
    pub fn classify(&self, received: u64) -> Result<SequenceStatus, ClientError> {
        if received == 0 {
            return Err(ClientError::InvalidTransportSequence { received });
        }

        if received <= self.last_accepted {
            return Ok(SequenceStatus::Duplicate {
                last_accepted: self.last_accepted,
                received,
            });
        }

        let expected = self
            .next_expected()
            .ok_or(ClientError::TransportSequenceExhausted)?;

        if received == expected {
            Ok(SequenceStatus::InOrder)
        } else {
            Ok(SequenceStatus::Gap { expected, received })
        }
    }

    /// Accept an in-order sequence.
    ///
    /// Duplicates and gaps do not modify tracker state.
    pub fn observe(&mut self, received: u64) -> Result<SequenceStatus, ClientError> {
        let status = self.classify(received)?;

        if status == SequenceStatus::InOrder {
            self.last_accepted = received;
        }

        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_sequence_one() {
        let tracker = SequenceTracker::new();

        assert_eq!(tracker.last_accepted(), 0);
        assert_eq!(tracker.next_expected(), Some(1));
    }

    #[test]
    fn accepts_contiguous_sequences() {
        let mut tracker = SequenceTracker::new();

        for sequence in 1..=1000 {
            assert_eq!(tracker.observe(sequence), Ok(SequenceStatus::InOrder));
        }

        assert_eq!(tracker.last_accepted(), 1000);
    }

    #[test]
    fn detects_duplicate() {
        let mut tracker = SequenceTracker::new();

        tracker.observe(1).unwrap();

        assert_eq!(
            tracker.observe(1),
            Ok(SequenceStatus::Duplicate {
                last_accepted: 1,
                received: 1,
            })
        );

        assert_eq!(tracker.next_expected(), Some(2));
    }

    #[test]
    fn detects_gap_without_advancing() {
        let mut tracker = SequenceTracker::new();

        tracker.observe(1).unwrap();

        assert_eq!(
            tracker.observe(4),
            Ok(SequenceStatus::Gap {
                expected: 2,
                received: 4,
            })
        );

        assert_eq!(tracker.last_accepted(), 1);
    }

    #[test]
    fn rejects_sequence_zero() {
        let mut tracker = SequenceTracker::new();

        assert_eq!(
            tracker.observe(0),
            Err(ClientError::InvalidTransportSequence { received: 0 })
        );
    }

    #[test]
    fn classification_does_not_mutate_state() {
        let tracker = SequenceTracker::new();

        assert_eq!(tracker.classify(1), Ok(SequenceStatus::InOrder));

        assert_eq!(tracker.last_accepted(), 0);
    }

    #[test]
    fn reset_restores_initial_state() {
        let mut tracker = SequenceTracker::new();

        tracker.observe(1).unwrap();
        tracker.observe(2).unwrap();
        tracker.reset();

        assert_eq!(tracker.next_expected(), Some(1));
    }

    #[test]
    fn sequence_does_not_wrap() {
        let mut tracker = SequenceTracker {
            last_accepted: u64::MAX - 1,
        };

        assert_eq!(tracker.observe(u64::MAX), Ok(SequenceStatus::InOrder));

        assert_eq!(tracker.next_expected(), None);
    }
}
