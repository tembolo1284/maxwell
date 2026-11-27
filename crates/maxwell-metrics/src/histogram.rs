//! Fixed-capacity integer latency histograms.
//!
//! Observations are recorded into noncumulative buckets.
//! Values above the largest configured bound are recorded
//! in a separate overflow bucket.
//!
//! Percentiles use integer basis points and nearest-rank
//! estimation. No floating-point arithmetic is required.

use crate::error::MetricsError;

/// Default latency bounds, expressed in nanoseconds.
pub const LATENCY_BOUNDS_NS: [u64; 24] = [
    50,
    100,
    200,
    300,
    500,
    750,
    1_000,
    2_000,
    3_000,
    5_000,
    7_500,
    10_000,
    20_000,
    50_000,
    100_000,
    200_000,
    500_000,
    1_000_000,
    2_000_000,
    5_000_000,
    10_000_000,
    50_000_000,
    100_000_000,
    1_000_000_000,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistogramSnapshot<const N: usize> {
    pub bounds: [u64; N],
    pub buckets: [u64; N],
    pub overflow: u64,
    pub count: u64,
    pub sum: u128,
    pub min: Option<u64>,
    pub max: Option<u64>,
}

impl<const N: usize> HistogramSnapshot<N> {
    /// Integer arithmetic mean, rounded down.
    pub fn mean(&self) -> Option<u64> {
        if self.count == 0 {
            return None;
        }

        let mean = self.sum / u128::from(self.count);

        Some(mean.min(u128::from(u64::MAX)) as u64)
    }

    /// Estimate a percentile using nearest-rank semantics.
    ///
    /// 0 represents the observed minimum.
    /// 10000 represents the observed maximum.
    ///
    /// Intermediate values return an upper-bound estimate
    /// from the corresponding histogram bucket.
    pub fn percentile(&self, basis_points: u16) -> Result<Option<u64>, MetricsError> {
        if basis_points > 10_000 {
            return Err(MetricsError::InvalidPercentile);
        }

        if self.count == 0 {
            return Ok(None);
        }

        if basis_points == 0 {
            return Ok(self.min);
        }

        if basis_points == 10_000 {
            return Ok(self.max);
        }

        let rank = (u128::from(self.count) * u128::from(basis_points)).div_ceil(10_000);

        let mut cumulative = 0u128;

        for index in 0..N {
            cumulative += u128::from(self.buckets[index]);

            if cumulative >= rank {
                let bound = self.bounds[index];

                return Ok(Some(self.max.map_or(bound, |max| bound.min(max))));
            }
        }

        // The percentile falls into the overflow bucket.
        Ok(self.max)
    }

    #[inline]
    pub fn p50(&self) -> Option<u64> {
        self.percentile(5_000).ok().flatten()
    }

    #[inline]
    pub fn p95(&self) -> Option<u64> {
        self.percentile(9_500).ok().flatten()
    }

    #[inline]
    pub fn p99(&self) -> Option<u64> {
        self.percentile(9_900).ok().flatten()
    }

    #[inline]
    pub fn p999(&self) -> Option<u64> {
        self.percentile(9_990).ok().flatten()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Histogram<const N: usize> {
    bounds: [u64; N],
    buckets: [u64; N],
    overflow: u64,
    count: u64,
    sum: u128,
    min: Option<u64>,
    max: Option<u64>,
}

impl<const N: usize> Histogram<N> {
    pub fn new(bounds: [u64; N]) -> Result<Self, MetricsError> {
        if N == 0 || bounds.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(MetricsError::InvalidHistogramBounds);
        }

        Ok(Self {
            bounds,
            buckets: [0; N],
            overflow: 0,
            count: 0,
            sum: 0,
            min: None,
            max: None,
        })
    }

    #[inline]
    pub const fn count(&self) -> u64 {
        self.count
    }

    #[inline]
    pub const fn sum(&self) -> u128 {
        self.sum
    }

    #[inline]
    pub const fn min(&self) -> Option<u64> {
        self.min
    }

    #[inline]
    pub const fn max(&self) -> Option<u64> {
        self.max
    }

    #[inline]
    pub const fn bounds(&self) -> &[u64; N] {
        &self.bounds
    }

    #[inline]
    pub const fn buckets(&self) -> &[u64; N] {
        &self.buckets
    }

    #[inline]
    pub const fn overflow(&self) -> u64 {
        self.overflow
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Record an observation.
    ///
    /// Returns false if the observation count has reached
    /// u64::MAX. No state changes occur in that case.
    ///
    /// Bucket lookup is O(log N).
    #[inline]
    pub fn record(&mut self, value: u64) -> bool {
        if self.count == u64::MAX {
            return false;
        }

        let index = self.bounds.partition_point(|&bound| bound < value);

        if index == N {
            self.overflow += 1;
        } else {
            self.buckets[index] += 1;
        }

        self.count += 1;
        self.sum += u128::from(value);

        self.min = Some(self.min.map_or(value, |min| min.min(value)));

        self.max = Some(self.max.map_or(value, |max| max.max(value)));

        true
    }

    #[inline]
    pub const fn snapshot(&self) -> HistogramSnapshot<N> {
        HistogramSnapshot {
            bounds: self.bounds,
            buckets: self.buckets,
            overflow: self.overflow,
            count: self.count,
            sum: self.sum,
            min: self.min,
            max: self.max,
        }
    }

    /// Merge another histogram with identical bounds.
    ///
    /// Returns false if the combined count would overflow.
    /// Failed merges leave the destination unchanged.
    pub fn merge(&mut self, other: &Self) -> Result<bool, MetricsError> {
        if self.bounds != other.bounds {
            return Err(MetricsError::IncompatibleHistogramBounds);
        }

        let Some(count) = self.count.checked_add(other.count) else {
            return Ok(false);
        };

        for index in 0..N {
            self.buckets[index] += other.buckets[index];
        }

        self.overflow += other.overflow;
        self.count = count;
        self.sum += other.sum;

        if let Some(other_min) = other.min {
            self.min = Some(self.min.map_or(other_min, |min| min.min(other_min)));
        }

        if let Some(other_max) = other.max {
            self.max = Some(self.max.map_or(other_max, |max| max.max(other_max)));
        }

        Ok(true)
    }

    /// Reset observations while retaining bucket bounds.
    pub fn reset(&mut self) {
        self.buckets.fill(0);
        self.overflow = 0;
        self.count = 0;
        self.sum = 0;
        self.min = None;
        self.max = None;
    }
}

impl Default for Histogram<24> {
    fn default() -> Self {
        Self::new(LATENCY_BOUNDS_NS).expect("default latency bounds must be valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_histogram_is_valid() {
        let histogram = Histogram::default();

        assert!(histogram.is_empty());
        assert_eq!(histogram.bounds(), &LATENCY_BOUNDS_NS);
    }

    #[test]
    fn rejects_empty_bounds() {
        assert_eq!(
            Histogram::<0>::new([]),
            Err(MetricsError::InvalidHistogramBounds)
        );
    }

    #[test]
    fn rejects_duplicate_bounds() {
        assert_eq!(
            Histogram::new([10, 20, 20]),
            Err(MetricsError::InvalidHistogramBounds)
        );
    }

    #[test]
    fn rejects_unsorted_bounds() {
        assert_eq!(
            Histogram::new([20, 10, 30]),
            Err(MetricsError::InvalidHistogramBounds)
        );
    }

    #[test]
    fn records_inclusive_bucket_boundaries() {
        let mut histogram = Histogram::new([10, 20, 30]).unwrap();

        for value in [0, 10, 11, 20, 21, 30, 31] {
            assert!(histogram.record(value));
        }

        assert_eq!(histogram.buckets(), &[2, 2, 2]);
        assert_eq!(histogram.overflow(), 1);
        assert_eq!(histogram.count(), 7);
    }

    #[test]
    fn tracks_exact_summary_statistics() {
        let mut histogram = Histogram::new([10, 20, 30]).unwrap();

        for value in [5, 10, 15, 20] {
            histogram.record(value);
        }

        let snapshot = histogram.snapshot();

        assert_eq!(snapshot.count, 4);
        assert_eq!(snapshot.sum, 50);
        assert_eq!(snapshot.min, Some(5));
        assert_eq!(snapshot.max, Some(20));
        assert_eq!(snapshot.mean(), Some(12));
    }

    #[test]
    fn empty_histogram_has_no_percentiles() {
        let histogram = Histogram::new([10, 20]).unwrap();
        let snapshot = histogram.snapshot();

        assert_eq!(snapshot.mean(), None);
        assert_eq!(snapshot.p50(), None);
        assert_eq!(snapshot.p99(), None);
    }

    #[test]
    fn calculates_nearest_rank_percentiles() {
        let mut histogram = Histogram::new([10, 20, 100]).unwrap();

        for _ in 0..50 {
            histogram.record(10);
        }

        for _ in 0..45 {
            histogram.record(20);
        }

        for _ in 0..5 {
            histogram.record(100);
        }

        let snapshot = histogram.snapshot();

        assert_eq!(snapshot.p50(), Some(10));
        assert_eq!(snapshot.p95(), Some(20));
        assert_eq!(snapshot.p99(), Some(100));
    }

    #[test]
    fn percentile_endpoints_are_exact() {
        let mut histogram = Histogram::new([10, 20]).unwrap();

        histogram.record(3);
        histogram.record(17);

        let snapshot = histogram.snapshot();

        assert_eq!(snapshot.percentile(0), Ok(Some(3)));
        assert_eq!(snapshot.percentile(10_000), Ok(Some(17)));
    }

    #[test]
    fn rejects_invalid_percentile() {
        let histogram = Histogram::new([10, 20]).unwrap();

        assert_eq!(
            histogram.snapshot().percentile(10_001),
            Err(MetricsError::InvalidPercentile)
        );
    }

    #[test]
    fn handles_overflow_percentile() {
        let mut histogram = Histogram::new([10, 20]).unwrap();

        histogram.record(5);
        histogram.record(1000);

        assert_eq!(histogram.snapshot().p99(), Some(1000));
    }

    #[test]
    fn merges_compatible_histograms() {
        let mut first = Histogram::new([10, 20]).unwrap();
        let mut second = Histogram::new([10, 20]).unwrap();

        first.record(5);
        first.record(15);

        second.record(10);
        second.record(100);

        assert_eq!(first.merge(&second), Ok(true));

        assert_eq!(first.buckets(), &[2, 1]);
        assert_eq!(first.overflow(), 1);
        assert_eq!(first.count(), 4);
        assert_eq!(first.sum(), 130);
        assert_eq!(first.min(), Some(5));
        assert_eq!(first.max(), Some(100));
    }

    #[test]
    fn incompatible_merge_does_not_mutate_destination() {
        let mut first = Histogram::new([10, 20]).unwrap();
        let second = Histogram::new([10, 30]).unwrap();

        first.record(5);

        let before = first;

        assert_eq!(
            first.merge(&second),
            Err(MetricsError::IncompatibleHistogramBounds)
        );

        assert_eq!(first, before);
    }

    #[test]
    fn handles_maximum_observation() {
        let mut histogram = Histogram::new([10, 20]).unwrap();

        histogram.record(u64::MAX);
        histogram.record(u64::MAX);

        assert_eq!(histogram.sum(), 2 * u128::from(u64::MAX));
        assert_eq!(histogram.snapshot().mean(), Some(u64::MAX));
    }

    #[test]
    fn rejects_count_overflow_without_mutation() {
        let mut histogram = Histogram::new([10]).unwrap();

        histogram.count = u64::MAX;
        histogram.buckets[0] = u64::MAX;
        histogram.min = Some(0);
        histogram.max = Some(0);

        let before = histogram;

        assert!(!histogram.record(1));
        assert_eq!(histogram, before);
    }

    #[test]
    fn rejects_merge_count_overflow() {
        let mut first = Histogram::new([10]).unwrap();
        let mut second = Histogram::new([10]).unwrap();

        first.count = u64::MAX;
        first.buckets[0] = u64::MAX;
        first.min = Some(0);
        first.max = Some(0);

        second.record(1);

        let before = first;

        assert_eq!(first.merge(&second), Ok(false));
        assert_eq!(first, before);
    }

    #[test]
    fn reset_preserves_configuration() {
        let mut histogram = Histogram::new([10, 20]).unwrap();

        histogram.record(5);
        histogram.record(100);

        histogram.reset();

        assert!(histogram.is_empty());
        assert_eq!(histogram.bounds(), &[10, 20]);
        assert_eq!(histogram.buckets(), &[0, 0]);
        assert_eq!(histogram.overflow(), 0);
        assert_eq!(histogram.sum(), 0);
        assert_eq!(histogram.min(), None);
        assert_eq!(histogram.max(), None);
    }
}
