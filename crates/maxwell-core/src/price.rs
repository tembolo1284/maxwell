//! Fixed-point price representation.
//!
//! Prices are represented as signed 64-bit integers in
//! instrument-defined atomic price units.
//!
//! The decimal scale and tick size are defined by instrument
//! metadata rather than by the price itself.
//!
//! Design guarantees:
//!
//! - Exact integer representation.
//! - Support for negative and zero prices.
//! - Explicit overflow handling.
//! - No heap allocations during arithmetic.
//! - No floating-point dependencies.
//! - No dependency on external wire representations.

use core::fmt;

// ============================================================
// Price
// ============================================================

/// A price expressed in instrument-defined atomic price units.
///
/// Prices may be negative, zero, or positive.
///
/// Two prices should only be compared or combined when their
/// instrument and numerical scale are compatible.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Price(i64);

impl Price {
    // ========================================================
    // Constants
    // ========================================================

    /// Zero price.
    pub const ZERO: Self = Self(0);

    /// Minimum representable price.
    pub const MIN: Self = Self(i64::MIN);

    /// Maximum representable price.
    pub const MAX: Self = Self(i64::MAX);

    // ========================================================
    // Construction and access
    // ========================================================

    /// Constructs a price from its raw integer representation.
    ///
    /// This does not validate instrument-specific price limits
    /// or tick alignment.
    #[must_use]
    #[inline]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the underlying integer representation.
    #[must_use]
    #[inline]
    pub const fn raw(self) -> i64 {
        self.0
    }

    // ========================================================
    // Predicates
    // ========================================================

    /// Returns whether the price is zero.
    #[must_use]
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Returns whether the price is positive.
    #[must_use]
    #[inline]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// Returns whether the price is negative.
    #[must_use]
    #[inline]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    // ========================================================
    // Checked arithmetic
    // ========================================================

    /// Adds two compatible prices.
    ///
    /// Returns `None` if the result cannot be represented.
    #[must_use]
    #[inline]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.0.checked_add(rhs.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Subtracts a compatible price.
    ///
    /// Returns `None` if the result cannot be represented.
    #[must_use]
    #[inline]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Multiplies the price by an integer scalar.
    ///
    /// Returns `None` on overflow.
    #[must_use]
    #[inline]
    pub const fn checked_mul(self, rhs: i64) -> Option<Self> {
        match self.0.checked_mul(rhs) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Divides the price by an integer scalar.
    ///
    /// Division truncates toward zero.
    ///
    /// Returns `None` on division by zero or overflow.
    #[must_use]
    #[inline]
    pub const fn checked_div(self, rhs: i64) -> Option<Self> {
        match self.0.checked_div(rhs) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the negated price.
    ///
    /// Returns `None` for the minimum representable price.
    #[must_use]
    #[inline]
    pub const fn checked_neg(self) -> Option<Self> {
        match self.0.checked_neg() {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the absolute price.
    ///
    /// Returns `None` for the minimum representable price.
    #[must_use]
    #[inline]
    pub const fn checked_abs(self) -> Option<Self> {
        match self.0.checked_abs() {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    // ========================================================
    // Price distance
    // ========================================================

    /// Returns the unsigned distance between two compatible
    /// prices, expressed in atomic price units.
    ///
    /// Supports the full range between `Price::MIN` and
    /// `Price::MAX` without overflow.
    #[must_use]
    #[inline]
    pub const fn abs_diff(self, rhs: Self) -> u64 {
        self.0.abs_diff(rhs.0)
    }

    // ========================================================
    // Tick alignment
    // ========================================================

    /// Returns whether the price is aligned to a positive tick
    /// size on a zero-origin price grid.
    ///
    /// Returns false for zero or negative tick sizes.
    #[must_use]
    #[inline]
    pub const fn is_tick_aligned(self, tick_size: i64) -> bool {
        tick_size > 0 && self.0 % tick_size == 0
    }

    /// Rounds the price toward negative infinity to the
    /// nearest tick on a zero-origin price grid.
    ///
    /// Returns `None` for invalid tick sizes or overflow.
    #[must_use]
    #[inline]
    pub const fn floor_to_tick(self, tick_size: i64) -> Option<Self> {
        if tick_size <= 0 {
            return None;
        }

        let remainder = self.0.rem_euclid(tick_size);

        match self.0.checked_sub(remainder) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Rounds the price toward positive infinity to the
    /// nearest tick on a zero-origin price grid.
    ///
    /// Returns `None` for invalid tick sizes or overflow.
    #[must_use]
    #[inline]
    pub const fn ceil_to_tick(self, tick_size: i64) -> Option<Self> {
        if tick_size <= 0 {
            return None;
        }

        let remainder = self.0.rem_euclid(tick_size);

        if remainder == 0 {
            return Some(self);
        }

        let adjustment = tick_size - remainder;

        match self.0.checked_add(adjustment) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

// ============================================================
// Conversions
// ============================================================

impl From<Price> for i64 {
    #[inline]
    fn from(value: Price) -> Self {
        value.0
    }
}

// ============================================================
// Formatting
// ============================================================

impl fmt::Display for Price {
    /// Displays the raw integer price.
    ///
    /// Decimal formatting requires instrument metadata.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

// ============================================================
// Compile-time layout guarantees
// ============================================================

const _: () = {
    assert!(size_of::<Price>() == size_of::<i64>());
    assert!(align_of::<Price>() == align_of::<i64>());
};

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_and_conversion() {
        let price = Price::new(12_345);

        assert_eq!(price.raw(), 12_345);
        assert_eq!(i64::from(price), 12_345);
    }

    #[test]
    fn constants() {
        assert_eq!(Price::ZERO.raw(), 0);
        assert_eq!(Price::MIN.raw(), i64::MIN);
        assert_eq!(Price::MAX.raw(), i64::MAX);
    }

    #[test]
    fn predicates() {
        assert!(Price::ZERO.is_zero());
        assert!(Price::new(100).is_positive());
        assert!(Price::new(-100).is_negative());

        assert!(!Price::ZERO.is_positive());
        assert!(!Price::ZERO.is_negative());
    }

    #[test]
    fn ordering() {
        assert!(Price::new(-100) < Price::ZERO);
        assert!(Price::ZERO < Price::new(100));
        assert!(Price::new(100) < Price::new(200));
    }

    #[test]
    fn checked_addition() {
        assert_eq!(
            Price::new(100).checked_add(Price::new(50)),
            Some(Price::new(150)),
        );
    }

    #[test]
    fn addition_overflow() {
        assert_eq!(Price::MAX.checked_add(Price::new(1)), None,);

        assert_eq!(Price::MIN.checked_add(Price::new(-1)), None,);
    }

    #[test]
    fn checked_subtraction() {
        assert_eq!(
            Price::new(100).checked_sub(Price::new(150)),
            Some(Price::new(-50)),
        );
    }

    #[test]
    fn subtraction_overflow() {
        assert_eq!(Price::MIN.checked_sub(Price::new(1)), None,);

        assert_eq!(Price::MAX.checked_sub(Price::new(-1)), None,);
    }

    #[test]
    fn checked_multiplication() {
        assert_eq!(Price::new(100).checked_mul(3), Some(Price::new(300)),);

        assert_eq!(Price::new(100).checked_mul(-3), Some(Price::new(-300)),);
    }

    #[test]
    fn multiplication_overflow() {
        assert_eq!(Price::MAX.checked_mul(2), None);
        assert_eq!(Price::MIN.checked_mul(-1), None);
    }

    #[test]
    fn checked_division() {
        assert_eq!(Price::new(100).checked_div(4), Some(Price::new(25)),);

        assert_eq!(Price::new(-101).checked_div(4), Some(Price::new(-25)),);
    }

    #[test]
    fn invalid_division() {
        assert_eq!(Price::new(100).checked_div(0), None);
        assert_eq!(Price::MIN.checked_div(-1), None);
    }

    #[test]
    fn checked_negation() {
        assert_eq!(Price::new(100).checked_neg(), Some(Price::new(-100)),);

        assert_eq!(Price::MIN.checked_neg(), None);
    }

    #[test]
    fn checked_absolute_value() {
        assert_eq!(Price::new(-100).checked_abs(), Some(Price::new(100)),);

        assert_eq!(Price::MIN.checked_abs(), None);
    }

    #[test]
    fn absolute_difference() {
        assert_eq!(Price::new(100).abs_diff(Price::new(-50)), 150,);

        assert_eq!(Price::MAX.abs_diff(Price::MIN), u64::MAX,);
    }

    #[test]
    fn tick_alignment() {
        assert!(Price::new(100).is_tick_aligned(5));
        assert!(Price::new(-100).is_tick_aligned(5));
        assert!(Price::ZERO.is_tick_aligned(5));

        assert!(!Price::new(103).is_tick_aligned(5));
        assert!(!Price::new(100).is_tick_aligned(0));
        assert!(!Price::new(100).is_tick_aligned(-5));
    }

    #[test]
    fn floor_positive_price() {
        assert_eq!(Price::new(103).floor_to_tick(5), Some(Price::new(100)),);
    }

    #[test]
    fn floor_negative_price() {
        assert_eq!(Price::new(-103).floor_to_tick(5), Some(Price::new(-105)),);
    }

    #[test]
    fn ceil_positive_price() {
        assert_eq!(Price::new(103).ceil_to_tick(5), Some(Price::new(105)),);
    }

    #[test]
    fn ceil_negative_price() {
        assert_eq!(Price::new(-103).ceil_to_tick(5), Some(Price::new(-100)),);
    }

    #[test]
    fn aligned_price_rounding() {
        let price = Price::new(100);

        assert_eq!(price.floor_to_tick(5), Some(price));
        assert_eq!(price.ceil_to_tick(5), Some(price));
    }

    #[test]
    fn invalid_tick_sizes() {
        let price = Price::new(100);

        assert_eq!(price.floor_to_tick(0), None);
        assert_eq!(price.floor_to_tick(-5), None);
        assert_eq!(price.ceil_to_tick(0), None);
        assert_eq!(price.ceil_to_tick(-5), None);
    }

    #[test]
    fn tick_rounding_overflow() {
        assert_eq!(Price::MIN.floor_to_tick(3), None);
        assert_eq!(Price::MAX.ceil_to_tick(2), None);
    }

    #[test]
    fn display_raw_price() {
        assert_eq!(Price::new(12_345).to_string(), "12345");
        assert_eq!(Price::new(-100).to_string(), "-100");
    }

    #[test]
    fn const_construction() {
        const PRICE: Price = Price::new(100);
        const RESULT: Option<Price> = PRICE.checked_add(Price::new(50));

        assert_eq!(RESULT, Some(Price::new(150)));
    }
}
