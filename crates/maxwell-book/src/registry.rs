//! Bounded resting order registry.
//!
//! Order identifiers are scoped to the owning instrument
//! book and its current feed session.
//!
//! The initial implementation uses the standard `HashMap`.
//! Capacity is reserved during construction.

use std::collections::HashMap;

use maxwell_core::OrderId;

use crate::config::MAX_ORDERS;
use crate::error::{BookError, BookResult};
use crate::order::RestingOrder;

#[derive(Debug)]
pub struct OrderRegistry {
    orders: HashMap<OrderId, RestingOrder>,
    max_orders: usize,
}

impl OrderRegistry {
    /// Creates a registry with preallocated order storage.
    ///
    /// # Errors
    ///
    /// Returns `InvalidOrderCapacity` if `max_orders` is zero or
    /// above the supported maximum, or
    /// `AllocationFailed` if storage cannot be reserved.
    pub fn new(max_orders: usize) -> BookResult<Self> {
        if max_orders == 0 || max_orders > MAX_ORDERS {
            return Err(BookError::InvalidOrderCapacity(max_orders));
        }

        let mut orders = HashMap::new();

        orders
            .try_reserve(max_orders)
            .map_err(|_| BookError::AllocationFailed)?;

        Ok(Self { orders, max_orders })
    }

    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    #[must_use]
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.max_orders
    }

    #[must_use]
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len() >= self.max_orders
    }

    #[must_use]
    #[inline]
    pub fn contains(&self, order_id: OrderId) -> bool {
        self.orders.contains_key(&order_id)
    }

    #[must_use]
    #[inline]
    pub fn get(&self, order_id: OrderId) -> Option<RestingOrder> {
        self.orders.get(&order_id).copied()
    }

    /// Validates insertion without changing the registry.
    #[inline]
    pub(crate) fn validate_insert(&self, order_id: OrderId) -> BookResult<()> {
        if self.contains(order_id) {
            return Err(BookError::DuplicateOrder(order_id));
        }

        if self.is_full() {
            return Err(BookError::OrderCapacityExceeded);
        }

        Ok(())
    }

    #[inline]
    pub(crate) fn insert(&mut self, order: RestingOrder) -> BookResult<()> {
        self.validate_insert(order.order_id())?;

        // HashMap capacity can decrease as entries are
        // removed. Check before insertion to avoid implicit
        // growth or reallocation on the update path.
        if self.orders.len() >= self.orders.capacity() {
            return Err(BookError::OrderCapacityExceeded);
        }

        self.orders.insert(order.order_id(), order);

        Ok(())
    }

    #[inline]
    pub(crate) fn remove(&mut self, order_id: OrderId) -> BookResult<RestingOrder> {
        self.orders
            .remove(&order_id)
            .ok_or(BookError::UnknownOrder(order_id))
    }

    #[inline]
    pub(crate) fn update(&mut self, order: RestingOrder) -> BookResult<()> {
        let current = self
            .orders
            .get_mut(&order.order_id())
            .ok_or_else(|| BookError::UnknownOrder(order.order_id()))?;

        *current = order;

        Ok(())
    }

    /// Iteration order is unspecified.
    ///
    /// Snapshot consumers must sort orders before producing
    /// deterministic serialized output.
    pub fn iter(&self) -> impl Iterator<Item = &RestingOrder> {
        self.orders.values()
    }

    pub(crate) fn clear(&mut self) {
        self.orders.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{Price, Quantity, SequenceNumber, Side};

    fn order(id: u64) -> RestingOrder {
        RestingOrder::new(
            OrderId::new(id),
            Side::Buy,
            Price::new(100),
            Quantity::new(500),
            SequenceNumber::new(id),
        )
        .unwrap()
    }

    #[test]
    fn insert_and_lookup() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();

        assert_eq!(registry.get(OrderId::new(1)), Some(order(1)));
    }

    #[test]
    fn duplicate_order_rejected() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();

        assert_eq!(
            registry.insert(order(1)),
            Err(BookError::DuplicateOrder(OrderId::new(1))),
        );
    }

    #[test]
    fn capacity_enforced() {
        let mut registry = OrderRegistry::new(1).unwrap();

        registry.insert(order(1)).unwrap();

        assert_eq!(
            registry.insert(order(2)),
            Err(BookError::OrderCapacityExceeded),
        );
    }

    #[test]
    fn remove_order() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();

        assert_eq!(registry.remove(OrderId::new(1)), Ok(order(1)),);

        assert!(registry.is_empty());
    }

    #[test]
    fn update_remaining_quantity() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();

        let reduced = order(1).reduced(Quantity::new(200)).unwrap();

        registry.update(reduced).unwrap();

        assert_eq!(
            registry.get(OrderId::new(1)).unwrap().quantity(),
            Quantity::new(300),
        );
    }

    #[test]
    fn clear_preserves_capacity() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();
        registry.clear();

        assert!(registry.is_empty());
        assert_eq!(registry.capacity(), 16);
    }
}
