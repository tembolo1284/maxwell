//! Fixed-point quantity representation.
//!
//! Quantities are represented as unsigned 64-bit integers in
//! instrument-defined atomic quantity units.
//!
//! Order direction is represented separately by `Side`.
//!
//! Design guarantees:
//!
//! - Nonnegative quantities.
//! - Exact integer representation.
//! - Explicit overflow and underflow handling.
//! - No heap allocations during arithmetic.
//! - No floating-point dependencies.
//! - No dependency on external wire representations.

use core::fmt;

// ============================================================
// Quantity
// ============================================================

/// A nonnegative quantity expressed in instrument-defined
/// atomic quantity units.
///
/// Zero is valid at the domain level, although individual
/// market events may require strictly positive quantities.
///
/// Quantities should only be combined when their instrument
/// and numerical scale are compatible.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Quantity(u64);

impl Quantity {
    // ========================================================
    // Constants
    // ========================================================

    /// Zero quantity.
    pub const ZERO: Self = Self(0);

    /// One atomic quantity unit.
    pub const ONE: Self = Self(1);

    /// Minimum representable quantity.
    pub const MIN: Self = Self::ZERO;

    /// Maximum representable quantity.
    pub const MAX: Self = Self(u64::MAX);

    // ========================================================
    // Construction and access
    // ========================================================

    /// Constructs a quantity from its raw integer value.
    ///
    /// This does not validate instrument-specific lot sizes
    /// or quantity limits.
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

    // ========================================================
    // Predicates
    // ========================================================

    /// Returns whether the quantity is zero.
    #[must_use]
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Returns whether the quantity is strictly positive.
    #[must_use]
    #[inline]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    // ========================================================
    // Checked arithmetic
    // ========================================================

    /// Adds two compatible quantities.
    ///
    /// Returns `None` on overflow.
    #[must_use]
    #[inline]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.0.checked_add(rhs.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Subtracts a compatible quantity.
    ///
    /// Returns `None` if the result would be negative.
    #[must_use]
    #[inline]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Multiplies the quantity by an unsigned integer scalar.
    ///
    /// Returns `None` on overflow.
    #[must_use]
    #[inline]
    pub const fn checked_mul(self, rhs: u64) -> Option<Self> {
        match self.0.checked_mul(rhs) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Divides the quantity by an unsigned integer scalar.
    ///
    /// Division truncates toward zero.
    ///
    /// Returns `None` on division by zero.
    #[must_use]
    #[inline]
    pub const fn checked_div(self, rhs: u64) -> Option<Self> {
        match self.0.checked_div(rhs) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    // ========================================================
    // Quantity distance
    // ========================================================

    /// Returns the absolute difference between two compatible
    /// quantities.
    #[must_use]
    #[inline]
    pub const fn abs_diff(self, rhs: Self) -> Self {
        Self(self.0.abs_diff(rhs.0))
    }

    // ========================================================
    // Lot alignment
    // ========================================================

    /// Returns whether the quantity is an integer multiple
    /// of the specified positive lot size.
    ///
    /// Returns false for a zero lot size.
    #[must_use]
    #[inline]
    pub const fn is_lot_aligned(self, lot_size: u64) -> bool {
        lot_size != 0 && self.0.is_multiple_of(lot_size)
    }

    /// Rounds the quantity down to the nearest lot.
    ///
    /// Returns `None` for a zero lot size.
    #[must_use]
    #[inline]
    pub const fn floor_to_lot(self, lot_size: u64) -> Option<Self> {
        if lot_size == 0 {
            return None;
        }

        let remainder = self.0 % lot_size;

        Some(Self(self.0 - remainder))
    }

    /// Rounds the quantity up to the nearest lot.
    ///
    /// Returns `None` for a zero lot size or overflow.
    #[must_use]
    #[inline]
    pub const fn ceil_to_lot(self, lot_size: u64) -> Option<Self> {
        if lot_size == 0 {
            return None;
        }

        let remainder = self.0 % lot_size;

        if remainder == 0 {
            return Some(self);
        }

        let adjustment = lot_size - remainder;

        match self.0.checked_add(adjustment) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    // ========================================================
    // Quantity selection
    // ========================================================

    /// Returns the smaller of two compatible quantities.
    #[must_use]
    #[inline]
    pub const fn min(self, rhs: Self) -> Self {
        if self.0 <= rhs.0 { self } else { rhs }
    }

    /// Returns the larger of two compatible quantities.
    #[must_use]
    #[inline]
    pub const fn max(self, rhs: Self) -> Self {
        if self.0 >= rhs.0 { self } else { rhs }
    }
}

// ============================================================
// Conversions
// ============================================================

impl From<Quantity> for u64 {
    #[inline]
    fn from(value: Quantity) -> Self {
        value.0
    }
}

// ============================================================
// Formatting
// ============================================================

impl fmt::Display for Quantity {
    /// Displays the raw integer quantity.
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
    assert!(size_of::<Quantity>() == size_of::<u64>());
    assert!(align_of::<Quantity>() == align_of::<u64>());
};

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_and_conversion() {
        let quantity = Quantity::new(1_000);

        assert_eq!(quantity.raw(), 1_000);
        assert_eq!(u64::from(quantity), 1_000);
    }

    #[test]
    fn constants() {
        assert_eq!(Quantity::ZERO.raw(), 0);
        assert_eq!(Quantity::ONE.raw(), 1);
        assert_eq!(Quantity::MIN.raw(), 0);
        assert_eq!(Quantity::MAX.raw(), u64::MAX);
    }

    #[test]
    fn predicates() {
        assert!(Quantity::ZERO.is_zero());
        assert!(!Quantity::ZERO.is_positive());

        assert!(Quantity::ONE.is_positive());
        assert!(!Quantity::ONE.is_zero());
    }

    #[test]
    fn ordering() {
        assert!(Quantity::ZERO < Quantity::ONE);
        assert!(Quantity::new(100) < Quantity::new(200));
    }

    #[test]
    fn checked_addition() {
        assert_eq!(
            Quantity::new(100).checked_add(Quantity::new(50)),
            Some(Quantity::new(150)),
        );
    }

    #[test]
    fn addition_overflow() {
        assert_eq!(Quantity::MAX.checked_add(Quantity::ONE), None,);
    }

    #[test]
    fn checked_subtraction() {
        assert_eq!(
            Quantity::new(100).checked_sub(Quantity::new(40)),
            Some(Quantity::new(60)),
        );
    }

    #[test]
    fn subtraction_to_zero() {
        assert_eq!(
            Quantity::new(100).checked_sub(Quantity::new(100)),
            Some(Quantity::ZERO),
        );
    }

    #[test]
    fn subtraction_underflow() {
        assert_eq!(Quantity::new(100).checked_sub(Quantity::new(101)), None,);

        assert_eq!(Quantity::ZERO.checked_sub(Quantity::ONE), None,);
    }

    #[test]
    fn checked_multiplication() {
        assert_eq!(Quantity::new(100).checked_mul(3), Some(Quantity::new(300)),);
    }

    #[test]
    fn multiplication_by_zero() {
        assert_eq!(Quantity::MAX.checked_mul(0), Some(Quantity::ZERO),);
    }

    #[test]
    fn multiplication_overflow() {
        assert_eq!(Quantity::MAX.checked_mul(2), None);
    }

    #[test]
    fn checked_division() {
        assert_eq!(Quantity::new(100).checked_div(4), Some(Quantity::new(25)),);
    }

    #[test]
    fn division_truncation() {
        assert_eq!(Quantity::new(103).checked_div(5), Some(Quantity::new(20)),);
    }

    #[test]
    fn division_by_zero() {
        assert_eq!(Quantity::new(100).checked_div(0), None);
    }

    #[test]
    fn absolute_difference() {
        assert_eq!(
            Quantity::new(100).abs_diff(Quantity::new(150)),
            Quantity::new(50),
        );

        assert_eq!(Quantity::MAX.abs_diff(Quantity::ZERO), Quantity::MAX,);
    }

    #[test]
    fn lot_alignment() {
        assert!(Quantity::new(100).is_lot_aligned(10));
        assert!(Quantity::ZERO.is_lot_aligned(10));

        assert!(!Quantity::new(103).is_lot_aligned(10));
        assert!(!Quantity::new(100).is_lot_aligned(0));
    }

    #[test]
    fn floor_to_lot() {
        assert_eq!(
            Quantity::new(107).floor_to_lot(10),
            Some(Quantity::new(100)),
        );
    }

    #[test]
    fn floor_to_zero() {
        assert_eq!(Quantity::new(7).floor_to_lot(10), Some(Quantity::ZERO),);
    }

    #[test]
    fn ceil_to_lot() {
        assert_eq!(Quantity::new(107).ceil_to_lot(10), Some(Quantity::new(110)),);
    }

    #[test]
    fn aligned_quantity_rounding() {
        let quantity = Quantity::new(100);

        assert_eq!(quantity.floor_to_lot(10), Some(quantity));
        assert_eq!(quantity.ceil_to_lot(10), Some(quantity));
    }

    #[test]
    fn zero_quantity_rounding() {
        assert_eq!(Quantity::ZERO.floor_to_lot(10), Some(Quantity::ZERO),);

        assert_eq!(Quantity::ZERO.ceil_to_lot(10), Some(Quantity::ZERO),);
    }

    #[test]
    fn invalid_lot_size() {
        let quantity = Quantity::new(100);

        assert_eq!(quantity.floor_to_lot(0), None);
        assert_eq!(quantity.ceil_to_lot(0), None);
    }

    #[test]
    fn lot_rounding_overflow() {
        assert_eq!(Quantity::MAX.ceil_to_lot(2), None);
    }

    #[test]
    fn minimum_quantity() {
        assert_eq!(Quantity::new(100).min(Quantity::new(50)), Quantity::new(50),);
    }

    #[test]
    fn maximum_quantity() {
        assert_eq!(
            Quantity::new(100).max(Quantity::new(50)),
            Quantity::new(100),
        );
    }

    #[test]
    fn display_raw_quantity() {
        assert_eq!(Quantity::new(1_000).to_string(), "1000");
    }

    #[test]
    fn const_construction() {
        const QUANTITY: Quantity = Quantity::new(100);
        const RESULT: Option<Quantity> = QUANTITY.checked_sub(Quantity::new(40));

        assert_eq!(RESULT, Some(Quantity::new(60)));
    }
}
