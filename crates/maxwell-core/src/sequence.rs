//! Market data sequence numbers and sequence ranges.
//!
//! Provides strongly typed sequence primitives for deterministic
//! event processing, gap detection, and feed recovery.
//!
//! Sequence numbers are monotonically increasing within an
//! exchange-defined stream and session.
//!
//! Sequence numbers never wrap implicitly. Session resets and
//! sequence exhaustion must be handled explicitly.
//!
//! Stateful sequence tracking belongs to maxwell-sequencer.

use core::fmt;

// ============================================================
// Sequence number
// ============================================================

/// A market data sequence number.
///
/// Sequence numbers are comparable only within the same
/// logical stream and session.
///
/// Zero is representable at the domain level. Whether zero
/// is a valid initial sequence is protocol-specific.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    // ========================================================
    // Constants
    // ========================================================

    /// Zero sequence number.
    pub const ZERO: Self = Self(0);

    /// Sequence number one.
    pub const ONE: Self = Self(1);

    /// Minimum representable sequence number.
    pub const MIN: Self = Self::ZERO;

    /// Maximum representable sequence number.
    pub const MAX: Self = Self(u64::MAX);

    // ========================================================
    // Construction and access
    // ========================================================

    /// Constructs a sequence number from its raw value.
    #[must_use]
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying integer representation.
    #[must_use]
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Returns whether the sequence number is zero.
    #[must_use]
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    // ========================================================
    // Sequence arithmetic
    // ========================================================

    /// Returns the immediately following sequence number.
    ///
    /// Returns None when the sequence space is exhausted.
    #[must_use]
    #[inline]
    pub const fn next(self) -> Option<Self> {
        self.checked_add(1)
    }

    /// Returns the immediately preceding sequence number.
    ///
    /// Returns None for sequence zero.
    #[must_use]
    #[inline]
    pub const fn previous(self) -> Option<Self> {
        self.checked_sub(1)
    }

    /// Advances the sequence number by the specified amount.
    ///
    /// Returns None on overflow.
    #[must_use]
    #[inline]
    pub const fn checked_add(self, amount: u64) -> Option<Self> {
        match self.0.checked_add(amount) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Moves the sequence number backward.
    ///
    /// Returns None on underflow.
    #[must_use]
    #[inline]
    pub const fn checked_sub(self, amount: u64) -> Option<Self> {
        match self.0.checked_sub(amount) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    // ========================================================
    // Sequence distance
    // ========================================================

    /// Returns the forward distance from an earlier sequence.
    ///
    /// Returns None if earlier is greater than self.
    #[must_use]
    #[inline]
    pub const fn distance_from(self, earlier: Self) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }

    /// Returns the absolute distance between two sequences.
    #[must_use]
    #[inline]
    pub const fn abs_diff(self, other: Self) -> u64 {
        self.0.abs_diff(other.0)
    }

    // ========================================================
    // Sequence relationships
    // ========================================================

    /// Returns whether self immediately follows previous.
    #[must_use]
    #[inline]
    pub const fn is_successor_of(self, previous: Self) -> bool {
        match previous.next() {
            Some(expected) => self.0 == expected.0,
            None => false,
        }
    }

    /// Returns whether self immediately precedes next.
    #[must_use]
    #[inline]
    pub const fn is_predecessor_of(self, next: Self) -> bool {
        next.is_successor_of(self)
    }

    /// Returns whether self is strictly ahead of other.
    #[must_use]
    #[inline]
    pub const fn is_ahead_of(self, other: Self) -> bool {
        self.0 > other.0
    }

    /// Returns whether self is strictly behind other.
    #[must_use]
    #[inline]
    pub const fn is_behind(self, other: Self) -> bool {
        self.0 < other.0
    }
}

// ============================================================
// Conversions
// ============================================================

impl From<SequenceNumber> for u64 {
    #[inline]
    fn from(value: SequenceNumber) -> Self {
        value.0
    }
}

// ============================================================
// Formatting
// ============================================================

impl fmt::Display for SequenceNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

// ============================================================
// Sequence range
// ============================================================

/// A nonempty, inclusive range of sequence numbers.
///
/// Both endpoints belong to the same stream and session.
///
/// The start sequence must not exceed the end sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SequenceRange {
    start: SequenceNumber,
    end: SequenceNumber,
}

impl SequenceRange {
    // ========================================================
    // Construction
    // ========================================================

    /// Constructs an inclusive sequence range.
    ///
    /// Returns None if start exceeds end.
    #[must_use]
    #[inline]
    pub const fn new(start: SequenceNumber, end: SequenceNumber) -> Option<Self> {
        if start.raw() > end.raw() {
            return None;
        }

        Some(Self { start, end })
    }

    /// Constructs a range containing exactly one sequence.
    #[must_use]
    #[inline]
    pub const fn single(sequence: SequenceNumber) -> Self {
        Self {
            start: sequence,
            end: sequence,
        }
    }

    /// Constructs a range from a starting sequence and count.
    ///
    /// Returns None for a zero count or sequence overflow.
    #[must_use]
    #[inline]
    pub const fn from_count(start: SequenceNumber, count: u64) -> Option<Self> {
        if count == 0 {
            return None;
        }

        match start.checked_add(count - 1) {
            Some(end) => Some(Self { start, end }),
            None => None,
        }
    }

    /// Constructs the missing range between an expected and
    /// received sequence number.
    ///
    /// The received sequence is excluded from the range.
    ///
    /// Returns None if received is not ahead of expected.
    #[must_use]
    #[inline]
    pub const fn missing(expected: SequenceNumber, received: SequenceNumber) -> Option<Self> {
        if received.raw() <= expected.raw() {
            return None;
        }

        match received.previous() {
            Some(end) => Some(Self {
                start: expected,
                end,
            }),
            None => None,
        }
    }

    // ========================================================
    // Accessors
    // ========================================================

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

    /// Returns the inclusive number of sequences in the range.
    ///
    /// Uses u128 to represent the complete u64 sequence space.
    #[must_use]
    #[inline]
    pub const fn count(self) -> u128 {
        (self.end.raw() - self.start.raw()) as u128 + 1
    }

    /// Returns the inclusive count as a u64.
    ///
    /// Returns None if the range contains 2^64 sequences.
    #[must_use]
    #[inline]
    pub const fn checked_count(self) -> Option<u64> {
        (self.end.raw() - self.start.raw()).checked_add(1)
    }

    /// Returns whether the range contains exactly one sequence.
    #[must_use]
    #[inline]
    pub const fn is_single(self) -> bool {
        self.start.raw() == self.end.raw()
    }

    // ========================================================
    // Range relationships
    // ========================================================

    /// Returns whether the sequence is inside the range.
    #[must_use]
    #[inline]
    pub const fn contains(self, sequence: SequenceNumber) -> bool {
        sequence.raw() >= self.start.raw() && sequence.raw() <= self.end.raw()
    }

    /// Returns whether another range is fully contained.
    #[must_use]
    #[inline]
    pub const fn contains_range(self, other: Self) -> bool {
        self.contains(other.start) && self.contains(other.end)
    }

    /// Returns whether two ranges overlap.
    #[must_use]
    #[inline]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start.raw() <= other.end.raw() && other.start.raw() <= self.end.raw()
    }

    /// Returns whether two nonoverlapping ranges are adjacent.
    ///
    /// Sequence space never wraps at u64::MAX.
    #[must_use]
    #[inline]
    pub const fn is_adjacent(self, other: Self) -> bool {
        other.start.is_successor_of(self.end) || self.start.is_successor_of(other.end)
    }

    // ========================================================
    // Range operations
    // ========================================================

    /// Returns the intersection of two overlapping ranges.
    ///
    /// Returns None when the ranges do not overlap.
    #[must_use]
    #[inline]
    pub const fn intersection(self, other: Self) -> Option<Self> {
        let start = if self.start.raw() >= other.start.raw() {
            self.start
        } else {
            other.start
        };

        let end = if self.end.raw() <= other.end.raw() {
            self.end
        } else {
            other.end
        };

        Self::new(start, end)
    }

    /// Merges overlapping or adjacent sequence ranges.
    ///
    /// Returns None if merging would introduce sequences
    /// not present in either original range.
    #[must_use]
    #[inline]
    pub const fn merge(self, other: Self) -> Option<Self> {
        if !self.overlaps(other) && !self.is_adjacent(other) {
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

    /// Splits the range immediately before the given sequence.
    ///
    /// Returns None if either resulting range would be empty.
    #[must_use]
    #[inline]
    pub const fn split_at(self, sequence: SequenceNumber) -> Option<(Self, Self)> {
        if sequence.raw() <= self.start.raw() || sequence.raw() > self.end.raw() {
            return None;
        }

        match sequence.previous() {
            Some(previous) => Some((
                Self {
                    start: self.start,
                    end: previous,
                },
                Self {
                    start: sequence,
                    end: self.end,
                },
            )),
            None => None,
        }
    }

    /// Returns the portion of the range strictly after the
    /// specified sequence.
    ///
    /// Returns None if no sequences remain.
    #[must_use]
    #[inline]
    pub const fn remaining_after(self, sequence: SequenceNumber) -> Option<Self> {
        if sequence.raw() < self.start.raw() {
            return Some(self);
        }

        if sequence.raw() >= self.end.raw() {
            return None;
        }

        match sequence.next() {
            Some(start) => Some(Self {
                start,
                end: self.end,
            }),
            None => None,
        }
    }

    /// Returns an allocation-free iterator over the range.
    #[must_use]
    #[inline]
    pub const fn iter(self) -> SequenceRangeIter {
        SequenceRangeIter {
            remaining: Some(self),
        }
    }
}

// ============================================================
// Range formatting
// ============================================================

impl fmt::Display for SequenceRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..={}", self.start, self.end)
    }
}

// ============================================================
// Sequence range iterator
// ============================================================

/// Allocation-free iterator over an inclusive sequence range.
///
/// Iteration terminates correctly even when the final sequence
/// is u64::MAX.
#[derive(Debug, Clone)]
pub struct SequenceRangeIter {
    remaining: Option<SequenceRange>,
}

impl Iterator for SequenceRangeIter {
    type Item = SequenceNumber;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let range = self.remaining?;
        let current = range.start();

        self.remaining = range.remaining_after(current);

        Some(current)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.remaining {
            None => (0, Some(0)),
            Some(range) => match usize::try_from(range.count()) {
                Ok(count) => (count, Some(count)),
                Err(_) => (usize::MAX, None),
            },
        }
    }
}

impl DoubleEndedIterator for SequenceRangeIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let range = self.remaining?;
        let current = range.end();

        self.remaining = if range.is_single() {
            None
        } else {
            current.previous().map(|end| SequenceRange {
                start: range.start(),
                end,
            })
        };

        Some(current)
    }
}

impl core::iter::FusedIterator for SequenceRangeIter {}

impl IntoIterator for SequenceRange {
    type Item = SequenceNumber;
    type IntoIter = SequenceRangeIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// ============================================================
// Sequence classification
// ============================================================

/// Describes an incoming sequence relative to an expected
/// sequence number.
///
/// An older sequence is not necessarily a verified duplicate.
/// Duplicate verification and recovery policies belong to
/// maxwell-sequencer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceRelation {
    /// Received sequence matches the expected sequence.
    Expected,

    /// Received sequence precedes the expected sequence.
    Behind { distance: u64 },

    /// Received sequence is ahead of the expected sequence.
    Gap { missing: SequenceRange },
}

impl SequenceRelation {
    /// Classifies a received sequence against an expected one.
    ///
    /// Both sequences must belong to the same stream and session.
    #[must_use]
    #[inline]
    pub const fn classify(expected: SequenceNumber, received: SequenceNumber) -> Self {
        if received.raw() == expected.raw() {
            return Self::Expected;
        }

        if received.raw() < expected.raw() {
            return Self::Behind {
                distance: expected.raw() - received.raw(),
            };
        }

        // received > expected guarantees that received - 1
        // is representable and not less than expected.
        Self::Gap {
            missing: SequenceRange {
                start: expected,
                end: SequenceNumber::new(received.raw() - 1),
            },
        }
    }

    /// Returns whether the sequence is the expected one.
    #[must_use]
    #[inline]
    pub const fn is_expected(self) -> bool {
        matches!(self, Self::Expected)
    }

    /// Returns whether the sequence precedes expectations.
    #[must_use]
    #[inline]
    pub const fn is_behind(self) -> bool {
        matches!(self, Self::Behind { .. })
    }

    /// Returns whether a sequence gap exists.
    #[must_use]
    #[inline]
    pub const fn is_gap(self) -> bool {
        matches!(self, Self::Gap { .. })
    }

    /// Returns the missing sequence range, if present.
    #[must_use]
    #[inline]
    pub const fn missing_range(self) -> Option<SequenceRange> {
        match self {
            Self::Gap { missing } => Some(missing),
            _ => None,
        }
    }
}

// ============================================================
// Compile-time layout guarantees
// ============================================================

const _: () = {
    assert!(size_of::<SequenceNumber>() == size_of::<u64>());

    assert!(align_of::<SequenceNumber>() == align_of::<u64>());
};

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    const fn seq(value: u64) -> SequenceNumber {
        SequenceNumber::new(value)
    }

    fn range(start: u64, end: u64) -> SequenceRange {
        SequenceRange::new(seq(start), seq(end)).unwrap()
    }

    // ========================================================
    // Sequence number tests
    // ========================================================

    #[test]
    fn construction_and_conversion() {
        let sequence = seq(42);

        assert_eq!(sequence.raw(), 42);
        assert_eq!(u64::from(sequence), 42);
    }

    #[test]
    fn constants() {
        assert_eq!(SequenceNumber::ZERO.raw(), 0);
        assert_eq!(SequenceNumber::ONE.raw(), 1);
        assert_eq!(SequenceNumber::MIN.raw(), 0);
        assert_eq!(SequenceNumber::MAX.raw(), u64::MAX);
    }

    #[test]
    fn zero_predicate() {
        assert!(SequenceNumber::ZERO.is_zero());
        assert!(!SequenceNumber::ONE.is_zero());
    }

    #[test]
    fn sequence_ordering() {
        assert!(seq(100) < seq(200));
        assert!(seq(200) > seq(100));
        assert_eq!(seq(100), seq(100));
    }

    #[test]
    fn next_sequence() {
        assert_eq!(seq(100).next(), Some(seq(101)));
    }

    #[test]
    fn next_sequence_overflow() {
        assert_eq!(SequenceNumber::MAX.next(), None);
    }

    #[test]
    fn previous_sequence() {
        assert_eq!(seq(100).previous(), Some(seq(99)));
    }

    #[test]
    fn previous_sequence_underflow() {
        assert_eq!(SequenceNumber::ZERO.previous(), None);
    }

    #[test]
    fn checked_addition() {
        assert_eq!(seq(100).checked_add(50), Some(seq(150)));
        assert_eq!(SequenceNumber::MAX.checked_add(1), None);
    }

    #[test]
    fn checked_subtraction() {
        assert_eq!(seq(100).checked_sub(50), Some(seq(50)));
        assert_eq!(seq(100).checked_sub(101), None);
    }

    #[test]
    fn sequence_distance() {
        assert_eq!(seq(150).distance_from(seq(100)), Some(50));
        assert_eq!(seq(100).distance_from(seq(100)), Some(0));
        assert_eq!(seq(100).distance_from(seq(150)), None);
    }

    #[test]
    fn absolute_distance() {
        assert_eq!(seq(100).abs_diff(seq(150)), 50);
        assert_eq!(seq(150).abs_diff(seq(100)), 50);

        assert_eq!(SequenceNumber::MAX.abs_diff(SequenceNumber::ZERO), u64::MAX,);
    }

    #[test]
    fn successor_relationship() {
        assert!(seq(101).is_successor_of(seq(100)));
        assert!(!seq(102).is_successor_of(seq(100)));
        assert!(!seq(100).is_successor_of(seq(100)));
    }

    #[test]
    fn successor_does_not_wrap() {
        assert!(!SequenceNumber::ZERO.is_successor_of(SequenceNumber::MAX,),);
    }

    #[test]
    fn predecessor_relationship() {
        assert!(seq(100).is_predecessor_of(seq(101)));
        assert!(!seq(100).is_predecessor_of(seq(102)));
    }

    #[test]
    fn ahead_and_behind() {
        assert!(seq(101).is_ahead_of(seq(100)));
        assert!(seq(100).is_behind(seq(101)));

        assert!(!seq(100).is_ahead_of(seq(100)));
        assert!(!seq(100).is_behind(seq(100)));
    }

    #[test]
    fn sequence_display() {
        assert_eq!(seq(12_345).to_string(), "12345");
    }

    // ========================================================
    // Sequence range tests
    // ========================================================

    #[test]
    fn valid_range() {
        let range = range(100, 200);

        assert_eq!(range.start(), seq(100));
        assert_eq!(range.end(), seq(200));
        assert_eq!(range.count(), 101);
    }

    #[test]
    fn inverted_range() {
        assert_eq!(SequenceRange::new(seq(200), seq(100)), None,);
    }

    #[test]
    fn single_sequence_range() {
        let range = SequenceRange::single(seq(100));

        assert_eq!(range.start(), seq(100));
        assert_eq!(range.end(), seq(100));
        assert_eq!(range.count(), 1);
        assert!(range.is_single());
    }

    #[test]
    fn range_from_count() {
        assert_eq!(
            SequenceRange::from_count(seq(100), 10),
            Some(range(100, 109)),
        );
    }

    #[test]
    fn range_from_zero_count() {
        assert_eq!(SequenceRange::from_count(seq(100), 0), None,);
    }

    #[test]
    fn range_from_count_overflow() {
        assert_eq!(SequenceRange::from_count(SequenceNumber::MAX, 2), None,);
    }

    #[test]
    fn range_ending_at_maximum() {
        assert_eq!(
            SequenceRange::from_count(SequenceNumber::MAX, 1),
            Some(SequenceRange::single(SequenceNumber::MAX)),
        );
    }

    #[test]
    fn missing_range() {
        assert_eq!(
            SequenceRange::missing(seq(100), seq(105)),
            Some(range(100, 104)),
        );
    }

    #[test]
    fn single_missing_sequence() {
        assert_eq!(
            SequenceRange::missing(seq(100), seq(101)),
            Some(SequenceRange::single(seq(100))),
        );
    }

    #[test]
    fn no_missing_range() {
        assert_eq!(SequenceRange::missing(seq(100), seq(100)), None,);

        assert_eq!(SequenceRange::missing(seq(100), seq(99)), None,);
    }

    #[test]
    fn range_count() {
        let range = range(100, 200);

        assert_eq!(range.count(), 101);
        assert_eq!(range.checked_count(), Some(101));
    }

    #[test]
    fn full_sequence_space_count() {
        let range = range(0, u64::MAX);

        assert_eq!(range.count(), 1_u128 << 64);
        assert_eq!(range.checked_count(), None);
    }

    #[test]
    fn range_contains_sequence() {
        let range = range(100, 200);

        assert!(range.contains(seq(100)));
        assert!(range.contains(seq(150)));
        assert!(range.contains(seq(200)));

        assert!(!range.contains(seq(99)));
        assert!(!range.contains(seq(201)));
    }

    #[test]
    fn range_contains_range() {
        let outer = range(100, 200);

        assert!(outer.contains_range(range(120, 180)));
        assert!(outer.contains_range(outer));

        assert!(!outer.contains_range(range(50, 150)));
        assert!(!outer.contains_range(range(150, 250)));
    }

    #[test]
    fn overlapping_ranges() {
        assert!(range(100, 200).overlaps(range(150, 250)));
        assert!(range(100, 200).overlaps(range(200, 300)));

        assert!(!range(100, 200).overlaps(range(201, 300)));
    }

    #[test]
    fn adjacent_ranges() {
        assert!(range(100, 200).is_adjacent(range(201, 300)));
        assert!(range(201, 300).is_adjacent(range(100, 200)));

        assert!(!range(100, 200).is_adjacent(range(202, 300)));
        assert!(!range(100, 200).is_adjacent(range(200, 300)));
    }

    #[test]
    fn adjacency_does_not_wrap() {
        let first = SequenceRange::single(SequenceNumber::ZERO);
        let last = SequenceRange::single(SequenceNumber::MAX);

        assert!(!first.is_adjacent(last));
        assert!(!last.is_adjacent(first));
    }

    #[test]
    fn range_intersection() {
        assert_eq!(
            range(100, 200).intersection(range(150, 250)),
            Some(range(150, 200)),
        );
    }

    #[test]
    fn single_point_intersection() {
        assert_eq!(
            range(100, 200).intersection(range(200, 300)),
            Some(SequenceRange::single(seq(200))),
        );
    }

    #[test]
    fn disjoint_intersection() {
        assert_eq!(range(100, 200).intersection(range(201, 300)), None,);
    }

    #[test]
    fn merge_overlapping_ranges() {
        assert_eq!(
            range(100, 200).merge(range(150, 250)),
            Some(range(100, 250)),
        );
    }

    #[test]
    fn merge_adjacent_ranges() {
        assert_eq!(
            range(100, 200).merge(range(201, 300)),
            Some(range(100, 300)),
        );
    }

    #[test]
    fn merge_disjoint_ranges() {
        assert_eq!(range(100, 200).merge(range(202, 300)), None,);
    }

    #[test]
    fn merge_contained_range() {
        assert_eq!(
            range(100, 200).merge(range(120, 180)),
            Some(range(100, 200)),
        );
    }

    #[test]
    fn split_range() {
        assert_eq!(
            range(100, 200).split_at(seq(150)),
            Some((range(100, 149), range(150, 200))),
        );
    }

    #[test]
    fn split_range_at_end() {
        assert_eq!(
            range(100, 200).split_at(seq(200)),
            Some((range(100, 199), SequenceRange::single(seq(200)),)),
        );
    }

    #[test]
    fn invalid_range_split() {
        let range = range(100, 200);

        assert_eq!(range.split_at(seq(99)), None);
        assert_eq!(range.split_at(seq(100)), None);
        assert_eq!(range.split_at(seq(201)), None);
    }

    #[test]
    fn remaining_range() {
        assert_eq!(
            range(100, 200).remaining_after(seq(150)),
            Some(range(151, 200)),
        );
    }

    #[test]
    fn remaining_range_before_start() {
        assert_eq!(
            range(100, 200).remaining_after(seq(99)),
            Some(range(100, 200)),
        );
    }

    #[test]
    fn exhausted_remaining_range() {
        assert_eq!(range(100, 200).remaining_after(seq(200)), None,);

        assert_eq!(range(100, 200).remaining_after(seq(201)), None,);
    }

    #[test]
    fn range_display() {
        assert_eq!(range(100, 200).to_string(), "100..=200");
    }

    // ========================================================
    // Iterator tests
    // ========================================================

    #[test]
    fn forward_iteration() {
        let sequences: Vec<_> = range(100, 103).into_iter().collect();

        assert_eq!(sequences, vec![seq(100), seq(101), seq(102), seq(103)],);
    }

    #[test]
    fn reverse_iteration() {
        let sequences: Vec<_> = range(100, 103).into_iter().rev().collect();

        assert_eq!(sequences, vec![seq(103), seq(102), seq(101), seq(100)],);
    }

    #[test]
    fn mixed_direction_iteration() {
        let mut iterator = range(100, 103).iter();

        assert_eq!(iterator.next(), Some(seq(100)));
        assert_eq!(iterator.next_back(), Some(seq(103)));
        assert_eq!(iterator.next(), Some(seq(101)));
        assert_eq!(iterator.next_back(), Some(seq(102)));

        assert_eq!(iterator.next(), None);
        assert_eq!(iterator.next_back(), None);
    }

    #[test]
    fn iteration_at_maximum_sequence() {
        let mut iterator = range(u64::MAX - 1, u64::MAX).iter();

        assert_eq!(iterator.next(), Some(seq(u64::MAX - 1)));
        assert_eq!(iterator.next(), Some(SequenceNumber::MAX));
        assert_eq!(iterator.next(), None);
    }

    #[test]
    fn reverse_iteration_at_zero() {
        let mut iterator = range(0, 1).iter();

        assert_eq!(iterator.next_back(), Some(seq(1)));
        assert_eq!(iterator.next_back(), Some(seq(0)));
        assert_eq!(iterator.next_back(), None);
    }

    #[test]
    fn iterator_size_hint() {
        let mut iterator = range(100, 102).iter();

        assert_eq!(iterator.size_hint(), (3, Some(3)));

        assert_eq!(iterator.next(), Some(seq(100)));
        assert_eq!(iterator.size_hint(), (2, Some(2)));

        assert_eq!(iterator.next_back(), Some(seq(102)));
        assert_eq!(iterator.size_hint(), (1, Some(1)));

        assert_eq!(iterator.next(), Some(seq(101)));
        assert_eq!(iterator.size_hint(), (0, Some(0)));
    }

    #[test]
    fn full_range_size_hint() {
        let iterator = range(0, u64::MAX).iter();

        assert_eq!(iterator.size_hint(), (usize::MAX, None),);
    }

    #[test]
    fn fused_iteration() {
        let mut iterator = SequenceRange::single(seq(100)).iter();

        assert_eq!(iterator.next(), Some(seq(100)));
        assert_eq!(iterator.next(), None);
        assert_eq!(iterator.next(), None);
        assert_eq!(iterator.next_back(), None);
    }

    // ========================================================
    // Sequence classification tests
    // ========================================================

    #[test]
    fn expected_sequence() {
        let relation = SequenceRelation::classify(seq(100), seq(100));

        assert_eq!(relation, SequenceRelation::Expected);
        assert!(relation.is_expected());
        assert!(!relation.is_behind());
        assert!(!relation.is_gap());
        assert_eq!(relation.missing_range(), None);
    }

    #[test]
    fn sequence_behind() {
        let relation = SequenceRelation::classify(seq(100), seq(95));

        assert_eq!(relation, SequenceRelation::Behind { distance: 5 },);

        assert!(relation.is_behind());
        assert!(!relation.is_expected());
        assert!(!relation.is_gap());
    }

    #[test]
    fn sequence_gap() {
        let relation = SequenceRelation::classify(seq(100), seq(105));

        assert_eq!(
            relation,
            SequenceRelation::Gap {
                missing: range(100, 104),
            },
        );

        assert!(relation.is_gap());
        assert!(!relation.is_expected());
        assert!(!relation.is_behind());

        assert_eq!(relation.missing_range(), Some(range(100, 104)),);
    }

    #[test]
    fn single_sequence_gap() {
        assert_eq!(
            SequenceRelation::classify(seq(100), seq(101)),
            SequenceRelation::Gap {
                missing: SequenceRange::single(seq(100)),
            },
        );
    }

    #[test]
    fn maximum_sequence_gap() {
        let relation = SequenceRelation::classify(SequenceNumber::ZERO, SequenceNumber::MAX);

        assert_eq!(relation.missing_range(), Some(range(0, u64::MAX - 1)),);
    }

    #[test]
    fn maximum_sequence_behind() {
        assert_eq!(
            SequenceRelation::classify(SequenceNumber::MAX, SequenceNumber::ZERO,),
            SequenceRelation::Behind { distance: u64::MAX },
        );
    }

    #[test]
    fn maximum_expected_sequence() {
        assert_eq!(
            SequenceRelation::classify(SequenceNumber::MAX, SequenceNumber::MAX,),
            SequenceRelation::Expected,
        );
    }

    #[test]
    fn const_construction() {
        const SEQUENCE: SequenceNumber = SequenceNumber::new(100);

        const NEXT: Option<SequenceNumber> = SEQUENCE.next();

        const RELATION: SequenceRelation =
            SequenceRelation::classify(SequenceNumber::new(100), SequenceNumber::new(105));

        assert_eq!(NEXT, Some(seq(101)));
        assert!(RELATION.is_gap());
    }
}
