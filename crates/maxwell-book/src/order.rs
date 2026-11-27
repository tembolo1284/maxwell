//! Individual resting order representation.

use maxwell_core::{OrderId, Price, Quantity, SequenceNumber, Side};

use crate::error::{BookError, BookResult};

/// An individual resting order.
///
/// Priority sequence records the event establishing the
/// current order's priority. Actual exchange queue priority
/// must be supplied or derivable from authoritative events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestingOrder {
    order_id: OrderId,
    price: Price,
    quantity: Quantity,
    priority_sequence: SequenceNumber,
    side: Side,
}

impl RestingOrder {
    /// Creates a resting order.
    ///
    /// # Errors
    ///
    /// Returns `ZeroQuantity` if `quantity` is zero.
    pub const fn new(
        order_id: OrderId,
        side: Side,
        price: Price,
        quantity: Quantity,
        priority_sequence: SequenceNumber,
    ) -> BookResult<Self> {
        if quantity.is_zero() {
            return Err(BookError::ZeroQuantity);
        }

        Ok(Self {
            order_id,
            price,
            quantity,
            priority_sequence,
            side,
        })
    }

    #[must_use]
    #[inline]
    pub const fn order_id(self) -> OrderId {
        self.order_id
    }

    #[must_use]
    #[inline]
    pub const fn side(self) -> Side {
        self.side
    }

    #[must_use]
    #[inline]
    pub const fn price(self) -> Price {
        self.price
    }

    #[must_use]
    #[inline]
    pub const fn quantity(self) -> Quantity {
        self.quantity
    }

    #[must_use]
    #[inline]
    pub const fn priority_sequence(self) -> SequenceNumber {
        self.priority_sequence
    }

    /// Returns the remaining order after a partial reduction.
    ///
    /// Complete reductions must remove the order instead.
    ///
    /// # Errors
    ///
    /// Returns `InvalidReduction` if the reduction is zero or would
    /// remove the whole order, or `QuantityUnderflow` if it exceeds
    /// the remaining quantity.
    #[inline]
    pub fn reduced(self, quantity: Quantity) -> BookResult<Self> {
        if quantity.is_zero() || quantity >= self.quantity {
            return Err(BookError::InvalidReduction);
        }

        let remaining = self
            .quantity
            .checked_sub(quantity)
            .ok_or(BookError::QuantityUnderflow)?;

        Ok(Self {
            quantity: remaining,
            ..self
        })
    }
}

const _: () = {
    assert!(!core::mem::needs_drop::<RestingOrder>());
};

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> RestingOrder {
        RestingOrder::new(
            OrderId::new(1),
            Side::Buy,
            Price::new(100),
            Quantity::new(500),
            SequenceNumber::new(10),
        )
        .unwrap()
    }

    #[test]
    fn partial_reduction() {
        let reduced = order().reduced(Quantity::new(200)).unwrap();

        assert_eq!(reduced.quantity(), Quantity::new(300));
        assert_eq!(reduced.priority_sequence(), SequenceNumber::new(10),);
    }

    #[test]
    fn full_reduction_rejected() {
        assert!(order().reduced(Quantity::new(500)).is_err());
    }

    #[test]
    fn excessive_reduction_rejected() {
        assert!(order().reduced(Quantity::new(501)).is_err());
    }

    #[test]
    fn zero_reduction_rejected() {
        assert!(order().reduced(Quantity::ZERO).is_err());
    }

    #[test]
    fn zero_quantity_order_rejected() {
        assert!(
            RestingOrder::new(
                OrderId::new(1),
                Side::Buy,
                Price::new(100),
                Quantity::ZERO,
                SequenceNumber::ONE,
            )
            .is_err()
        );
    }
}
