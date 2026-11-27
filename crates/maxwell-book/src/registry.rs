//! Bounded resting order registry.
//!
//! Order identifiers are scoped to the owning instrument
//! book and its current feed session.
//!
//! The initial implementation uses the standard `HashMap`.
//! Capacity is reserved during construction.
//!
//! Logical order capacity is enforced independently of
//! the hash table's internal insertion capacity.
//!
//! Sustained insertion and removal may require internal
//! hash table maintenance or additional allocation.

use std::collections::HashMap;

use maxwell_core::OrderId;

use crate::config::MAX_ORDERS;
use crate::error::{BookError, BookResult};
use crate::order::RestingOrder;

// ============================================================
// Order registry
// ============================================================

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

    // --------------------------------------------------------
    // Observation
    // --------------------------------------------------------

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

    /// Returns the configured logical order capacity.
    ///
    /// This value is independent of the underlying hash
    /// table's current insertion capacity.
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

    // --------------------------------------------------------
    // Insertion
    // --------------------------------------------------------

    /// Validates insertion without changing the registry.
    ///
    /// Only logical capacity and order identity are checked.
    /// Internal hash table capacity is handled separately
    /// during insertion.
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

    /// Inserts a resting order.
    ///
    /// The logical capacity remains strictly enforced.
    ///
    /// `HashMap` insertion capacity may decrease during
    /// sustained insertion and removal because of internal
    /// deleted-entry bookkeeping.
    ///
    /// Reserving one additional entry allows the hash table
    /// to reclaim insertion capacity or grow when necessary,
    /// rather than incorrectly rejecting a valid order.
    ///
    /// This operation may perform internal rehashing or
    /// memory allocation.
    #[inline]
    pub(crate) fn insert(&mut self, order: RestingOrder) -> BookResult<()> {
        self.validate_insert(order.order_id())?;

        self.orders
            .try_reserve(1)
            .map_err(|_| BookError::AllocationFailed)?;

        self.orders.insert(order.order_id(), order);

        Ok(())
    }

    // --------------------------------------------------------
    // Removal and modification
    // --------------------------------------------------------

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

    // --------------------------------------------------------
    // Iteration and reset
    // --------------------------------------------------------

    /// Iteration order is unspecified.
    ///
    /// Snapshot consumers must sort orders before producing
    /// deterministic serialized output.
    pub fn iter(&self) -> impl Iterator<Item = &RestingOrder> {
        self.orders.values()
    }

    /// Removes all resting orders while retaining allocated
    /// hash table storage and the configured logical capacity.
    pub(crate) fn clear(&mut self) {
        self.orders.clear();
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{Price, Quantity, SequenceNumber, Side};

    // --------------------------------------------------------
    // Fixtures
    // --------------------------------------------------------

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

    // --------------------------------------------------------
    // Construction
    // --------------------------------------------------------

    #[test]
    fn registry_starts_empty() {
        let registry = OrderRegistry::new(16).unwrap();

        assert_eq!(registry.len(), 0);
        assert_eq!(registry.capacity(), 16);
        assert!(registry.is_empty());
        assert!(!registry.is_full());
    }

    #[test]
    fn zero_capacity_is_rejected() {
        assert!(matches!(
            OrderRegistry::new(0),
            Err(BookError::InvalidOrderCapacity(0)),
        ));
    }

    #[test]
    fn excessive_capacity_is_rejected() {
        let capacity = MAX_ORDERS + 1;

        assert!(matches!(
            OrderRegistry::new(capacity),
            Err(BookError::InvalidOrderCapacity(value))
                if value == capacity,
        ));
    }

    // --------------------------------------------------------
    // Insertion and lookup
    // --------------------------------------------------------

    #[test]
    fn insert_and_lookup() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();

        assert_eq!(registry.get(OrderId::new(1)), Some(order(1)));
        assert!(registry.contains(OrderId::new(1)));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn unknown_order_lookup_returns_none() {
        let registry = OrderRegistry::new(16).unwrap();

        assert_eq!(registry.get(OrderId::new(1)), None);
        assert!(!registry.contains(OrderId::new(1)));
    }

    #[test]
    fn duplicate_order_rejected() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();

        assert_eq!(
            registry.insert(order(1)),
            Err(BookError::DuplicateOrder(OrderId::new(1))),
        );

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get(OrderId::new(1)), Some(order(1)));
    }

    #[test]
    fn capacity_enforced() {
        let mut registry = OrderRegistry::new(1).unwrap();

        registry.insert(order(1)).unwrap();

        assert!(registry.is_full());

        assert_eq!(
            registry.insert(order(2)),
            Err(BookError::OrderCapacityExceeded),
        );

        assert_eq!(registry.len(), 1);
        assert!(!registry.contains(OrderId::new(2)));
    }

    #[test]
    fn duplicate_detection_precedes_capacity_check() {
        let mut registry = OrderRegistry::new(1).unwrap();

        registry.insert(order(1)).unwrap();

        assert!(registry.is_full());

        assert_eq!(
            registry.insert(order(1)),
            Err(BookError::DuplicateOrder(OrderId::new(1))),
        );
    }

    // --------------------------------------------------------
    // Removal
    // --------------------------------------------------------

    #[test]
    fn remove_order() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();

        assert_eq!(registry.remove(OrderId::new(1)), Ok(order(1)),);

        assert!(registry.is_empty());
        assert!(!registry.contains(OrderId::new(1)));
    }

    #[test]
    fn remove_unknown_order_is_rejected() {
        let mut registry = OrderRegistry::new(16).unwrap();

        assert_eq!(
            registry.remove(OrderId::new(1)),
            Err(BookError::UnknownOrder(OrderId::new(1))),
        );

        assert!(registry.is_empty());
    }

    #[test]
    fn removal_restores_logical_capacity() {
        let mut registry = OrderRegistry::new(2).unwrap();

        registry.insert(order(1)).unwrap();
        registry.insert(order(2)).unwrap();

        assert!(registry.is_full());

        registry.remove(OrderId::new(1)).unwrap();

        assert!(!registry.is_full());

        registry.insert(order(3)).unwrap();

        assert!(registry.is_full());
        assert_eq!(registry.len(), 2);

        assert!(!registry.contains(OrderId::new(1)));
        assert!(registry.contains(OrderId::new(2)));
        assert!(registry.contains(OrderId::new(3)));
    }

    // --------------------------------------------------------
    // Modification
    // --------------------------------------------------------

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

        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn update_unknown_order_is_rejected() {
        let mut registry = OrderRegistry::new(16).unwrap();

        assert_eq!(
            registry.update(order(1)),
            Err(BookError::UnknownOrder(OrderId::new(1))),
        );

        assert!(registry.is_empty());
    }

    #[test]
    fn update_is_allowed_at_full_capacity() {
        let mut registry = OrderRegistry::new(1).unwrap();

        registry.insert(order(1)).unwrap();

        assert!(registry.is_full());

        let reduced = order(1).reduced(Quantity::new(100)).unwrap();

        registry.update(reduced).unwrap();

        assert_eq!(
            registry.get(OrderId::new(1)).unwrap().quantity(),
            Quantity::new(400),
        );

        assert!(registry.is_full());
        assert_eq!(registry.len(), 1);
    }

    // --------------------------------------------------------
    // Iteration and reset
    // --------------------------------------------------------

    #[test]
    fn iteration_returns_all_resting_orders() {
        let mut registry = OrderRegistry::new(16).unwrap();

        for id in 1..=8 {
            registry.insert(order(id)).unwrap();
        }

        let mut orders: Vec<_> = registry.iter().map(|order| order.order_id()).collect();

        orders.sort_unstable();

        let expected: Vec<_> = (1..=8).map(OrderId::new).collect();

        assert_eq!(orders, expected);
    }

    #[test]
    fn clear_preserves_capacity() {
        let mut registry = OrderRegistry::new(16).unwrap();

        registry.insert(order(1)).unwrap();
        registry.clear();

        assert!(registry.is_empty());
        assert_eq!(registry.capacity(), 16);
        assert!(!registry.is_full());
    }

    #[test]
    fn cleared_registry_can_be_refilled() {
        let mut registry = OrderRegistry::new(16).unwrap();

        for id in 1..=16 {
            registry.insert(order(id)).unwrap();
        }

        assert!(registry.is_full());

        registry.clear();

        assert!(registry.is_empty());

        for id in 17..=32 {
            registry.insert(order(id)).unwrap();
        }

        assert!(registry.is_full());
        assert_eq!(registry.len(), 16);
        assert_eq!(registry.capacity(), 16);
    }

    // --------------------------------------------------------
    // Sustained order churn regression tests
    // --------------------------------------------------------

    /// Repeated insertion and removal must not exhaust
    /// logical capacity through hash table tombstones.
    #[test]
    fn repeated_order_churn_preserves_capacity() {
        let mut registry = OrderRegistry::new(16).unwrap();

        for id in 1..=100_000 {
            registry.insert(order(id)).unwrap();

            assert_eq!(registry.len(), 1);

            registry.remove(OrderId::new(id)).unwrap();

            assert!(registry.is_empty());
        }

        assert_eq!(registry.capacity(), 16);
    }

    /// Sustained replacement of resting orders at maximum
    /// occupancy must not produce false capacity failures.
    #[test]
    fn sustained_churn_near_capacity() {
        const CAPACITY: usize = 64;

        let mut registry = OrderRegistry::new(CAPACITY).unwrap();

        for id in 1..=CAPACITY as u64 {
            registry.insert(order(id)).unwrap();
        }

        assert!(registry.is_full());

        for id in 1..=100_000u64 {
            let removed = registry.remove(OrderId::new(id)).unwrap();

            assert_eq!(removed.order_id(), OrderId::new(id));
            assert_eq!(registry.len(), CAPACITY - 1);

            registry.insert(order(id + CAPACITY as u64)).unwrap();

            assert_eq!(registry.len(), CAPACITY);
            assert!(registry.is_full());
        }

        assert_eq!(registry.capacity(), CAPACITY);
    }

    /// Internal storage maintenance must not weaken the
    /// configured maximum number of resting orders.
    #[test]
    fn logical_capacity_remains_enforced_after_churn() {
        let mut registry = OrderRegistry::new(4).unwrap();

        for id in 1..=10_000 {
            registry.insert(order(id)).unwrap();
            registry.remove(OrderId::new(id)).unwrap();
        }

        for id in 1..=4 {
            registry.insert(order(id)).unwrap();
        }

        assert_eq!(
            registry.insert(order(5)),
            Err(BookError::OrderCapacityExceeded),
        );

        assert_eq!(registry.len(), 4);
        assert_eq!(registry.capacity(), 4);
        assert!(registry.is_full());
    }

    /// Exercise multiple logical capacities because the
    /// underlying hash table rounds allocation sizes.
    #[test]
    fn sustained_churn_across_different_capacities() {
        for capacity in [1usize, 2, 4, 8, 16, 32, 64, 128, 256] {
            let mut registry = OrderRegistry::new(capacity).unwrap();

            for id in 1..=capacity as u64 {
                registry.insert(order(id)).unwrap();
            }

            for id in 1..=10_000u64 {
                registry.remove(OrderId::new(id)).unwrap();

                registry.insert(order(id + capacity as u64)).unwrap();

                assert_eq!(registry.len(), capacity);
                assert_eq!(registry.capacity(), capacity);
            }

            assert!(registry.is_full());
        }
    }

    /// Repeated clearing and refilling must preserve
    /// registry behavior across simulated session resets.
    #[test]
    fn repeated_clear_and_refill_preserves_capacity() {
        const CAPACITY: usize = 64;

        let mut registry = OrderRegistry::new(CAPACITY).unwrap();

        for cycle in 0..1_000u64 {
            let offset = cycle * CAPACITY as u64;

            for index in 1..=CAPACITY as u64 {
                registry.insert(order(offset + index)).unwrap();
            }

            assert!(registry.is_full());

            registry.clear();

            assert!(registry.is_empty());
            assert_eq!(registry.capacity(), CAPACITY);
        }
    }
}
