//! Aggregated price level representation.
//!
//! A price level stores aggregate resting quantity and
//! the number of individual resting orders.
//!
//! The price itself is implicit in the ladder index.

use maxwell_core::Quantity;

use crate::error::{BookError, BookResult};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceLevel {
    quantity: Quantity,
    order_count: u32,
}

impl PriceLevel {
    pub const EMPTY: Self = Self {
        quantity: Quantity::ZERO,
        order_count: 0,
    };

    /// Creates a level from aggregate quantity and order count.
    ///
    /// # Errors
    ///
    /// Returns `InvalidPriceLevel` if only one of the quantity and
    /// the order count is zero, or the quantity is smaller than the
    /// order count.
    pub const fn new(quantity: Quantity, order_count: u32) -> BookResult<Self> {
        if quantity.is_zero() != (order_count == 0) || quantity.raw() < order_count as u64 {
            return Err(BookError::InvalidPriceLevel);
        }

        Ok(Self {
            quantity,
            order_count,
        })
    }

    #[must_use]
    #[inline]
    pub const fn quantity(self) -> Quantity {
        self.quantity
    }

    #[must_use]
    #[inline]
    pub const fn order_count(self) -> u32 {
        self.order_count
    }

    #[must_use]
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.order_count == 0
    }

    /// Adds one resting order.
    ///
    /// All arithmetic is validated before mutation.
    ///
    /// # Errors
    ///
    /// Returns `ZeroQuantity` for a zero quantity, or
    /// `QuantityOverflow` or `OrderCountOverflow` if an aggregate
    /// would overflow.
    #[inline]
    pub fn add_order(&mut self, quantity: Quantity) -> BookResult<()> {
        if quantity.is_zero() {
            return Err(BookError::ZeroQuantity);
        }

        let next_quantity = self
            .quantity
            .checked_add(quantity)
            .ok_or(BookError::QuantityOverflow)?;

        let next_count = self
            .order_count
            .checked_add(1)
            .ok_or(BookError::OrderCountOverflow)?;

        *self = Self::new(next_quantity, next_count)?;

        Ok(())
    }

    /// Reduces aggregate quantity without removing an order.
    ///
    /// The caller must ensure that the individual order
    /// retains a positive remaining quantity.
    ///
    /// # Errors
    ///
    /// Returns `ZeroQuantity` for a zero quantity, or
    /// `QuantityUnderflow` if it exceeds the aggregate quantity.
    #[inline]
    pub fn reduce_quantity(&mut self, quantity: Quantity) -> BookResult<()> {
        if quantity.is_zero() {
            return Err(BookError::ZeroQuantity);
        }

        let next_quantity = self
            .quantity
            .checked_sub(quantity)
            .ok_or(BookError::QuantityUnderflow)?;

        *self = Self::new(next_quantity, self.order_count)?;

        Ok(())
    }

    /// Increases aggregate quantity without adding an order.
    ///
    /// # Errors
    ///
    /// Returns `ZeroQuantity` for a zero quantity,
    /// `InvalidPriceLevel` if the level is empty, or
    /// `QuantityOverflow` if the aggregate would overflow.
    #[inline]
    pub fn increase_quantity(&mut self, quantity: Quantity) -> BookResult<()> {
        if quantity.is_zero() {
            return Err(BookError::ZeroQuantity);
        }

        if self.is_empty() {
            return Err(BookError::InvalidPriceLevel);
        }

        let next_quantity = self
            .quantity
            .checked_add(quantity)
            .ok_or(BookError::QuantityOverflow)?;

        *self = Self::new(next_quantity, self.order_count)?;

        Ok(())
    }

    /// Removes one complete resting order.
    ///
    /// # Errors
    ///
    /// Returns `ZeroQuantity` for a zero quantity, or
    /// `QuantityUnderflow` or `OrderCountUnderflow` if an aggregate
    /// would underflow.
    #[inline]
    pub fn remove_order(&mut self, quantity: Quantity) -> BookResult<()> {
        if quantity.is_zero() {
            return Err(BookError::ZeroQuantity);
        }

        let next_quantity = self
            .quantity
            .checked_sub(quantity)
            .ok_or(BookError::QuantityUnderflow)?;

        let next_count = self
            .order_count
            .checked_sub(1)
            .ok_or(BookError::OrderCountUnderflow)?;

        *self = Self::new(next_quantity, next_count)?;

        Ok(())
    }

    #[inline]
    pub const fn clear(&mut self) {
        *self = Self::EMPTY;
    }
}

impl Default for PriceLevel {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qty(value: u64) -> Quantity {
        Quantity::new(value)
    }

    #[test]
    fn empty_level() {
        let level = PriceLevel::default();

        assert!(level.is_empty());
        assert_eq!(level.quantity(), Quantity::ZERO);
        assert_eq!(level.order_count(), 0);
    }

    #[test]
    fn aggregate_multiple_orders() {
        let mut level = PriceLevel::EMPTY;

        level.add_order(qty(100)).unwrap();
        level.add_order(qty(200)).unwrap();

        assert_eq!(level.quantity(), qty(300));
        assert_eq!(level.order_count(), 2);
    }

    #[test]
    fn partial_reduction_preserves_count() {
        let mut level = PriceLevel::new(qty(100), 1).unwrap();

        level.reduce_quantity(qty(40)).unwrap();

        assert_eq!(level.quantity(), qty(60));
        assert_eq!(level.order_count(), 1);
    }

    #[test]
    fn remove_last_order() {
        let mut level = PriceLevel::new(qty(100), 1).unwrap();

        level.remove_order(qty(100)).unwrap();

        assert!(level.is_empty());
    }

    #[test]
    fn overflow_does_not_mutate_level() {
        let mut level = PriceLevel::new(Quantity::MAX, 1).unwrap();

        let previous = level;

        assert_eq!(level.add_order(qty(1)), Err(BookError::QuantityOverflow),);

        assert_eq!(level, previous);
    }

    #[test]
    fn underflow_does_not_mutate_level() {
        let mut level = PriceLevel::new(qty(100), 1).unwrap();
        let previous = level;

        assert_eq!(
            level.remove_order(qty(101)),
            Err(BookError::QuantityUnderflow),
        );

        assert_eq!(level, previous);
    }

    #[test]
    fn zero_quantity_with_nonzero_count_rejected() {
        assert!(PriceLevel::new(Quantity::ZERO, 1).is_err());
    }

    #[test]
    fn quantity_without_orders_rejected() {
        assert!(PriceLevel::new(qty(100), 0).is_err());
    }

    #[test]
    fn insufficient_quantity_for_order_count_rejected() {
        assert!(PriceLevel::new(qty(1), 2).is_err());
    }
}
