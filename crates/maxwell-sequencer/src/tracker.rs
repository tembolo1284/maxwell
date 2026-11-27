//! Constant-time sequence watermark tracking.

use maxwell_core::SequenceNumber;

use crate::dedup::SequenceDisposition;
use crate::error::{SequencerError, SequencerResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceTracker {
    expected: Option<SequenceNumber>,
    last_delivered: Option<SequenceNumber>,
}

impl SequenceTracker {
    /// The initial sequence must be supplied by the caller.
    ///
    /// It must come from an authoritative feed checkpoint,
    /// session announcement, or snapshot.
    #[must_use]
    pub const fn new(expected: SequenceNumber) -> Self {
        Self {
            expected: Some(expected),
            last_delivered: None,
        }
    }

    #[must_use]
    #[inline]
    pub const fn expected(&self) -> Option<SequenceNumber> {
        self.expected
    }

    #[must_use]
    #[inline]
    pub const fn last_delivered(&self) -> Option<SequenceNumber> {
        self.last_delivered
    }

    #[must_use]
    #[inline]
    pub const fn is_exhausted(&self) -> bool {
        self.expected.is_none()
    }

    #[must_use]
    #[inline]
    pub const fn classify(&self, received: SequenceNumber) -> SequenceDisposition {
        SequenceDisposition::classify(self.expected, received)
    }

    /// Commits exactly the next expected sequence.
    ///
    /// Sequence u64::MAX can be committed, after which
    /// the tracker enters an exhausted state.
    ///
    /// Sequence numbers never wrap silently.
    #[inline]
    pub fn commit(&mut self, received: SequenceNumber) -> SequencerResult<()> {
        let expected = self.expected.ok_or(SequencerError::SequenceExhausted)?;

        if received != expected {
            return Err(SequencerError::SequenceMismatch { expected, received });
        }

        self.last_delivered = Some(received);

        self.expected = received.raw().checked_add(1).map(SequenceNumber::new);

        Ok(())
    }

    /// Resets the tracker to an externally established
    /// checkpoint.
    ///
    /// Feed and recovery validation belong to the owning
    /// StreamSequencer.
    pub fn reset(&mut self, expected: SequenceNumber) {
        *self = Self::new(expected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(value: u64) -> SequenceNumber {
        SequenceNumber::new(value)
    }

    #[test]
    fn initial_watermark() {
        let tracker = SequenceTracker::new(seq(100));

        assert_eq!(tracker.expected(), Some(seq(100)));
        assert_eq!(tracker.last_delivered(), None);
    }

    #[test]
    fn contiguous_commit() {
        let mut tracker = SequenceTracker::new(seq(100));

        tracker.commit(seq(100)).unwrap();

        assert_eq!(tracker.expected(), Some(seq(101)));
        assert_eq!(tracker.last_delivered(), Some(seq(100)));
    }

    #[test]
    fn future_sequence_cannot_be_committed() {
        let mut tracker = SequenceTracker::new(seq(100));

        assert!(tracker.commit(seq(102)).is_err());
        assert_eq!(tracker.expected(), Some(seq(100)));
    }

    #[test]
    fn stale_sequence_cannot_be_committed() {
        let mut tracker = SequenceTracker::new(seq(100));

        assert!(tracker.commit(seq(99)).is_err());
    }

    #[test]
    fn sequence_exhaustion() {
        let mut tracker = SequenceTracker::new(seq(u64::MAX));

        tracker.commit(seq(u64::MAX)).unwrap();

        assert!(tracker.is_exhausted());
        assert_eq!(tracker.expected(), None);

        assert_eq!(
            tracker.commit(seq(0)),
            Err(SequencerError::SequenceExhausted),
        );
    }

    #[test]
    fn reset_watermark() {
        let mut tracker = SequenceTracker::new(seq(100));

        tracker.commit(seq(100)).unwrap();
        tracker.reset(seq(500));

        assert_eq!(tracker.expected(), Some(seq(500)));
        assert_eq!(tracker.last_delivered(), None);
    }
}
