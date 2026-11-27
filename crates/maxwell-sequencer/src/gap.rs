//! Missing sequence range representation.
//!
//! Gaps use inclusive boundaries.

use maxwell_core::SequenceNumber;

use crate::error::{SequencerError, SequencerResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SequenceGap {
    start: SequenceNumber,
    end: SequenceNumber,
}

impl SequenceGap {
    /// Constructs a nonempty inclusive gap.
    ///
    /// # Errors
    ///
    /// Returns InvalidGap when start exceeds end.
    #[inline]
    pub const fn new(start: SequenceNumber, end: SequenceNumber) -> SequencerResult<Self> {
        if start.raw() > end.raw() {
            return Err(SequencerError::InvalidGap);
        }

        Ok(Self { start, end })
    }

    /// Constructs the missing range preceding an observed
    /// future sequence number.
    #[must_use]
    #[inline]
    pub const fn between(expected: SequenceNumber, received: SequenceNumber) -> Option<Self> {
        if received.raw() <= expected.raw() {
            return None;
        }

        Some(Self {
            start: expected,
            end: SequenceNumber::new(received.raw() - 1),
        })
    }

    #[must_use]
    #[inline]
    pub const fn start(self) -> SequenceNumber {
        self.start
    }

    #[must_use]
    #[inline]
    pub const fn end(self) -> SequenceNumber {
        self.end
    }

    /// Uses u128 so the complete u64 sequence space
    /// can be represented without overflowing.
    #[must_use]
    #[inline]
    pub const fn missing_count(self) -> u128 {
        self.end.raw() as u128 - self.start.raw() as u128 + 1
    }

    #[must_use]
    #[inline]
    pub const fn contains(self, sequence: SequenceNumber) -> bool {
        sequence.raw() >= self.start.raw() && sequence.raw() <= self.end.raw()
    }

    #[must_use]
    #[inline]
    pub const fn is_single(self) -> bool {
        self.start.raw() == self.end.raw()
    }

    #[must_use]
    #[inline]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start.raw() <= other.end.raw() && other.start.raw() <= self.end.raw()
    }

    /// Merges overlapping or adjacent gaps.
    ///
    /// Disjoint gaps are not merged because doing so could
    /// incorrectly identify received events as missing.
    ///
    /// Explicit boundary checks prevent integer overflow
    /// and preserve compatibility with stable const functions.
    #[must_use]
    #[inline]
    pub const fn merge(self, other: Self) -> Option<Self> {
        let adjacent = (self.end.raw() != u64::MAX && self.end.raw() + 1 == other.start.raw())
            || (other.end.raw() != u64::MAX && other.end.raw() + 1 == self.start.raw());

        if !self.overlaps(other) && !adjacent {
            return None;
        }

        let start = if self.start.raw() <= other.start.raw() {
            self.start
        } else {
            other.start
        };

        let end = if self.end.raw() >= other.end.raw() {
            self.end
        } else {
            other.end
        };

        Some(Self { start, end })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(value: u64) -> SequenceNumber {
        SequenceNumber::new(value)
    }

    #[test]
    fn inclusive_gap() {
        let gap = SequenceGap::new(seq(10), seq(15)).unwrap();

        assert_eq!(gap.missing_count(), 6);
        assert!(gap.contains(seq(10)));
        assert!(gap.contains(seq(15)));
        assert!(!gap.contains(seq(16)));
    }

    #[test]
    fn invalid_gap() {
        assert_eq!(
            SequenceGap::new(seq(20), seq(10)),
            Err(SequencerError::InvalidGap),
        );
    }

    #[test]
    fn single_missing_event() {
        let gap = SequenceGap::between(seq(10), seq(11)).unwrap();

        assert!(gap.is_single());
        assert_eq!(gap.start(), seq(10));
    }

    #[test]
    fn no_gap_for_expected_sequence() {
        assert!(SequenceGap::between(seq(10), seq(10)).is_none());
    }

    #[test]
    fn full_sequence_space() {
        let gap = SequenceGap::new(seq(0), seq(u64::MAX)).unwrap();

        assert_eq!(gap.missing_count(), 1_u128 << 64);
    }

    #[test]
    fn adjacent_gaps_merge() {
        let first = SequenceGap::new(seq(1), seq(5)).unwrap();
        let second = SequenceGap::new(seq(6), seq(10)).unwrap();

        assert_eq!(first.merge(second).unwrap().missing_count(), 10,);
    }

    #[test]
    fn disjoint_gaps_do_not_merge() {
        let first = SequenceGap::new(seq(1), seq(5)).unwrap();
        let second = SequenceGap::new(seq(7), seq(10)).unwrap();

        assert!(first.merge(second).is_none());
    }

    #[test]
    fn adjacent_gaps_merge_at_max() {
        let first = SequenceGap::new(seq(u64::MAX - 5), seq(u64::MAX - 1)).unwrap();

        let second = SequenceGap::new(seq(u64::MAX), seq(u64::MAX)).unwrap();

        let merged = first.merge(second).unwrap();

        assert_eq!(merged.missing_count(), 6);
        assert_eq!(merged.start(), seq(u64::MAX - 5));
        assert_eq!(merged.end(), seq(u64::MAX));
    }

    #[test]
    fn gaps_do_not_wrap_around() {
        let first = SequenceGap::new(seq(u64::MAX - 5), seq(u64::MAX)).unwrap();

        let second = SequenceGap::new(seq(0), seq(5)).unwrap();

        assert!(first.merge(second).is_none());
        assert!(second.merge(first).is_none());
    }

    #[test]
    fn overlapping_gaps_merge() {
        let first = SequenceGap::new(seq(1), seq(10)).unwrap();
        let second = SequenceGap::new(seq(5), seq(15)).unwrap();

        let merged = first.merge(second).unwrap();

        assert_eq!(merged.start(), seq(1));
        assert_eq!(merged.end(), seq(15));
        assert_eq!(merged.missing_count(), 15);
    }
}
