//! Deterministic pseudo-random number generation.
//!
//! SplitMix64 provides a small, reproducible RNG suitable
//! for synthetic market workloads. It is not intended
//! for cryptographic applications.

use crate::config::BASIS_POINTS;
use crate::error::{SimulatorError, SimulatorResult};

const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const MIX_A: u64 = 0xBF58_476D_1CE4_E5B9;
const MIX_B: u64 = 0x94D0_49BB_1331_11EB;

pub const MARKET_STREAM: u64 = 1;
pub const FAULT_STREAM: u64 = 2;

// ============================================================
// Random number generator
// ============================================================

/// Deterministic SplitMix64 random number generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Creates a generator from an arbitrary seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Creates a deterministic domain-separated RNG stream.
    #[must_use]
    pub fn for_stream(seed: u64, stream: u64) -> Self {
        let mut root = Self::new(
            seed ^ stream.wrapping_mul(GAMMA),
        );

        Self::new(root.next_u64())
    }

    /// Returns the current internal state.
    #[must_use]
    pub const fn state(&self) -> u64 {
        self.state
    }

    /// Generates the next unsigned 64-bit random value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);

        let mut value = self.state;

        value = (value ^ (value >> 30))
            .wrapping_mul(MIX_A);

        value = (value ^ (value >> 27))
            .wrapping_mul(MIX_B);

        value ^ (value >> 31)
    }

    /// Generates a random integer in [0, upper).
    ///
    /// Rejection sampling avoids modulo bias.
    pub fn below(
        &mut self,
        upper: u64,
    ) -> SimulatorResult<u64> {
        if upper == 0 {
            return Err(SimulatorError::InvalidArgument(
                "random upper bound must be positive",
            ));
        }

        let threshold = upper.wrapping_neg() % upper;

        loop {
            let value = self.next_u64();

            if value >= threshold {
                return Ok(value % upper);
            }
        }
    }

    /// Generates a valid index into a nonempty collection.
    pub fn index(
        &mut self,
        length: usize,
    ) -> SimulatorResult<usize> {
        let upper = u64::try_from(length)
            .map_err(|_| {
                SimulatorError::ArithmeticOverflow(
                    "random index upper bound",
                )
            })?;

        let index = self.below(upper)?;

        usize::try_from(index).map_err(|_| {
            SimulatorError::ArithmeticOverflow(
                "random index conversion",
            )
        })
    }

    /// Generates a random integer in [minimum, maximum].
    pub fn inclusive(
        &mut self,
        minimum: u64,
        maximum: u64,
    ) -> SimulatorResult<u64> {
        if minimum > maximum {
            return Err(SimulatorError::InvalidArgument(
                "random range minimum exceeds maximum",
            ));
        }

        let distance = maximum - minimum;

        // The complete u64 domain requires no reduction.
        if distance == u64::MAX {
            return Ok(self.next_u64());
        }

        Ok(minimum + self.below(distance + 1)?)
    }

    /// Generates a random Boolean value.
    pub fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 != 0
    }

    /// Generates a uniformly distributed basis-point value.
    ///
    /// The result is in [0, 10000).
    pub fn basis_points(&mut self) -> u32 {
        let upper = u64::from(BASIS_POINTS);
        let threshold = upper.wrapping_neg() % upper;

        loop {
            let value = self.next_u64();

            if value >= threshold {
                return (value % upper) as u32;
            }
        }
    }

    /// Returns true with the specified basis-point probability.
    pub fn chance(
        &mut self,
        probability_bps: u32,
    ) -> SimulatorResult<bool> {
        if probability_bps > BASIS_POINTS {
            return Err(SimulatorError::InvalidArgument(
                "probability exceeds 10000 basis points",
            ));
        }

        if probability_bps == 0 {
            return Ok(false);
        }

        if probability_bps == BASIS_POINTS {
            return Ok(true);
        }

        Ok(self.basis_points() < probability_bps)
    }

    /// Randomizes a slice using Fisher-Yates shuffling.
    pub fn shuffle<T>(
        &mut self,
        values: &mut [T],
    ) -> SimulatorResult<()> {
        for index in (1..values.len()).rev() {
            let selected = self.index(index + 1)?;

            values.swap(index, selected);
        }

        Ok(())
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_splitmix64_reference_values() {
        let mut rng = SplitMix64::new(0);

        assert_eq!(
            rng.next_u64(),
            0xE220_A839_7B1D_CDAF,
        );

        assert_eq!(
            rng.next_u64(),
            0x6E78_9E6A_A1B9_65F4,
        );

        assert_eq!(
            rng.next_u64(),
            0x06C4_5D18_8009_454F,
        );
    }

    #[test]
    fn identical_seeds_produce_identical_sequences() {
        let mut first = SplitMix64::new(42);
        let mut second = SplitMix64::new(42);

        for _ in 0..10_000 {
            assert_eq!(
                first.next_u64(),
                second.next_u64(),
            );
        }
    }

    #[test]
    fn different_seeds_produce_different_sequences() {
        let mut first = SplitMix64::new(42);
        let mut second = SplitMix64::new(43);

        assert_ne!(
            first.next_u64(),
            second.next_u64(),
        );
    }

    #[test]
    fn streams_are_domain_separated() {
        let mut market =
            SplitMix64::for_stream(42, MARKET_STREAM);

        let mut faults =
            SplitMix64::for_stream(42, FAULT_STREAM);

        assert_ne!(
            market.next_u64(),
            faults.next_u64(),
        );
    }

    #[test]
    fn identical_streams_are_reproducible() {
        let mut first =
            SplitMix64::for_stream(42, MARKET_STREAM);

        let mut second =
            SplitMix64::for_stream(42, MARKET_STREAM);

        for _ in 0..100 {
            assert_eq!(
                first.next_u64(),
                second.next_u64(),
            );
        }
    }

    #[test]
    fn bounded_values_remain_inside_range() {
        let mut rng = SplitMix64::new(42);

        for upper in [1, 2, 3, 7, 100, u64::MAX] {
            for _ in 0..1_000 {
                assert!(rng.below(upper).unwrap() < upper);
            }
        }
    }

    #[test]
    fn zero_upper_bound_is_rejected() {
        let mut rng = SplitMix64::new(42);

        assert!(rng.below(0).is_err());
        assert!(rng.index(0).is_err());
    }

    #[test]
    fn inclusive_range_supports_single_value() {
        let mut rng = SplitMix64::new(42);

        assert_eq!(rng.inclusive(7, 7).unwrap(), 7);
    }

    #[test]
    fn inclusive_range_supports_full_domain() {
        let mut first = SplitMix64::new(42);
        let mut second = SplitMix64::new(42);

        assert_eq!(
            first.inclusive(0, u64::MAX).unwrap(),
            second.next_u64(),
        );
    }

    #[test]
    fn reversed_range_is_rejected() {
        let mut rng = SplitMix64::new(42);

        assert!(rng.inclusive(10, 1).is_err());
    }

    #[test]
    fn probability_boundaries_are_exact() {
        let mut rng = SplitMix64::new(42);

        assert!(!rng.chance(0).unwrap());
        assert!(rng.chance(BASIS_POINTS).unwrap());
        assert!(rng.chance(BASIS_POINTS + 1).is_err());
    }

    #[test]
    fn basis_point_samples_are_bounded() {
        let mut rng = SplitMix64::new(42);

        for _ in 0..10_000 {
            assert!(rng.basis_points() < BASIS_POINTS);
        }
    }

    #[test]
    fn shuffle_preserves_all_elements() {
        let mut rng = SplitMix64::new(42);

        let mut values = [1, 2, 3, 4, 5, 6, 7, 8];

        rng.shuffle(&mut values).unwrap();

        values.sort_unstable();

        assert_eq!(values, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn shuffle_supports_empty_and_single_element_slices() {
        let mut rng = SplitMix64::new(42);

        let mut empty: [u64; 0] = [];
        let mut single = [42];

        rng.shuffle(&mut empty).unwrap();
        rng.shuffle(&mut single).unwrap();

        assert_eq!(single, [42]);
    }
}
