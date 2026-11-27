//! Level 3 market data: individual resting orders.
//!
//! Unsorted iteration requires no additional allocation.
//! Canonical sorted output is available for diagnostics
//! and snapshot generation.

use maxwell_core::{OrderId, Price, Side};

use crate::book::OrderBook;
use crate::error::{BookError, BookResult};
use crate::order::RestingOrder;

#[derive(Debug, Clone, Copy)]
pub struct Level3<'a> {
    book: &'a OrderBook,
}

impl<'a> Level3<'a> {
    #[must_use]
    #[inline]
    pub const fn new(book: &'a OrderBook) -> Self {
        Self { book }
    }

    #[must_use]
    #[inline]
    pub const fn book(&self) -> &'a OrderBook {
        self.book
    }

    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.book.order_count()
    }

    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.book.is_empty()
    }

    #[must_use]
    #[inline]
    pub fn order(&self, order_id: OrderId) -> Option<RestingOrder> {
        self.book.orders().get(order_id)
    }

    /// Iterates all orders in unspecified registry order.
    pub fn iter(&self) -> impl Iterator<Item = RestingOrder> + '_ {
        self.book.orders().iter().copied()
    }

    /// Iterates orders at one side and price.
    ///
    /// O(registry capacity) in the initial implementation.
    pub fn orders_at(&self, side: Side, price: Price) -> impl Iterator<Item = RestingOrder> + '_ {
        self.iter()
            .filter(move |order| order.side() == side && order.price() == price)
    }

    /// Produces deterministic price-priority ordered output.
    ///
    /// Priority sequence is used as the secondary key.
    /// Order ID provides a deterministic final tie-breaker.
    ///
    /// This is not a guarantee of actual exchange queue
    /// position unless the source feed supplies sufficient
    /// priority information.
    ///
    /// # Errors
    ///
    /// Returns `AllocationFailed` if the output cannot be reserved.
    pub fn sorted_side(&self, side: Side) -> BookResult<Vec<RestingOrder>> {
        let mut orders = Vec::new();

        orders
            .try_reserve_exact(self.len())
            .map_err(|_| BookError::AllocationFailed)?;

        orders.extend(self.iter().filter(|order| order.side() == side));

        orders.sort_unstable_by(|left, right| {
            let price_order = match side {
                Side::Buy => right.price().cmp(&left.price()),
                Side::Sell => left.price().cmp(&right.price()),
            };

            price_order
                .then_with(|| left.priority_sequence().cmp(&right.priority_sequence()))
                .then_with(|| left.order_id().cmp(&right.order_id()))
        });

        Ok(orders)
    }

    /// Produces a deterministic list of orders at a price.
    ///
    /// # Errors
    ///
    /// Returns `AllocationFailed` if the output cannot be reserved.
    pub fn sorted_orders_at(&self, side: Side, price: Price) -> BookResult<Vec<RestingOrder>> {
        let count = self
            .book
            .ladder(side)
            .level(price)
            .map_or(0, |level| level.order_count() as usize);

        let mut orders = Vec::new();

        orders
            .try_reserve_exact(count)
            .map_err(|_| BookError::AllocationFailed)?;

        orders.extend(self.orders_at(side, price));

        orders.sort_unstable_by_key(|order| (order.priority_sequence(), order.order_id()));

        Ok(orders)
    }
}
