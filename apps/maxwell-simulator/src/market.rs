//! Independent authoritative simulated market state.
//!
//! This implementation deliberately does not depend on
//! maxwell-book. It provides an independent reference
//! for validating reconstructed market state.

use std::collections::HashMap;

use maxwell_core::{
    EventEnvelope, FeedId, Instrument, InstrumentConfig,
    InstrumentId, MarketEvent, OrderId, Price, Quantity,
    SequenceNumber, Side,
};

use crate::config::MarketConfig;
use crate::error::{SimulatorError, SimulatorResult};

// ============================================================
// Resting order
// ============================================================

/// Authoritative resting order representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatedOrder {
    pub order_id: OrderId,
    pub side: Side,
    pub price: Price,
    pub quantity: Quantity,
}

// ============================================================
// Instrument market
// ============================================================

/// Independent order state for one simulated instrument.
///
/// Orders are stored densely for efficient random selection.
/// An index maps order identifiers to their vector positions.
#[derive(Debug)]
pub struct InstrumentMarket {
    instrument: Instrument,
    orders: Vec<SimulatedOrder>,
    index: HashMap<OrderId, usize>,
    capacity: usize,
    last_sequence: Option<SequenceNumber>,
}

impl InstrumentMarket {
    pub fn new(
        instrument: Instrument,
        capacity: usize,
    ) -> SimulatorResult<Self> {
        if capacity == 0 {
            return Err(SimulatorError::InvalidConfig(
                "instrument order capacity must be positive",
            ));
        }

        let mut orders = Vec::new();
        let mut index = HashMap::new();

        orders
            .try_reserve_exact(capacity)
            .map_err(|_| SimulatorError::AllocationFailed)?;

        index
            .try_reserve(capacity)
            .map_err(|_| SimulatorError::AllocationFailed)?;

        Ok(Self {
            instrument,
            orders,
            index,
            capacity,
            last_sequence: None,
        })
    }

    #[must_use]
    pub const fn instrument(&self) -> &Instrument {
        &self.instrument
    }

    #[must_use]
    pub const fn id(&self) -> InstrumentId {
        self.instrument.id()
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.orders.len() >= self.capacity
    }

    #[must_use]
    pub const fn last_sequence(
        &self,
    ) -> Option<SequenceNumber> {
        self.last_sequence
    }

    #[must_use]
    pub fn orders(&self) -> &[SimulatedOrder] {
        &self.orders
    }

    #[must_use]
    pub fn order(
        &self,
        order_id: OrderId,
    ) -> Option<&SimulatedOrder> {
        self.index
            .get(&order_id)
            .and_then(|index| self.orders.get(*index))
    }

    fn position(
        &self,
        order_id: OrderId,
    ) -> SimulatorResult<usize> {
        self.index
            .get(&order_id)
            .copied()
            .ok_or(SimulatorError::UnknownOrder {
                instrument_id: self.id(),
                order_id,
            })
    }

    // --------------------------------------------------------
    // Order insertion
    // --------------------------------------------------------

    fn add(
        &mut self,
        order: SimulatedOrder,
    ) -> SimulatorResult<()> {
        if self.index.contains_key(&order.order_id) {
            return Err(SimulatorError::DuplicateOrder {
                instrument_id: self.id(),
                order_id: order.order_id,
            });
        }

        if self.is_full() {
            return Err(SimulatorError::CapacityExceeded {
                resource: "instrument orders",
                capacity: self.capacity,
                required: self.orders.len() + 1,
            });
        }

        self.instrument.validate_limit_order(
            order.price,
            order.quantity,
        )?;

        let position = self.orders.len();

        self.orders.push(order);
        self.index.insert(order.order_id, position);

        Ok(())
    }

    // --------------------------------------------------------
    // Order replacement
    // --------------------------------------------------------

    fn modify(
        &mut self,
        order_id: OrderId,
        price: Price,
        quantity: Quantity,
    ) -> SimulatorResult<()> {
        let position = self.position(order_id)?;

        self.instrument.validate_limit_order(
            price,
            quantity,
        )?;

        let order = &mut self.orders[position];

        order.price = price;
        order.quantity = quantity;

        Ok(())
    }

    // --------------------------------------------------------
    // Order removal
    // --------------------------------------------------------

    fn remove_at(
        &mut self,
        position: usize,
    ) -> SimulatedOrder {
        let removed = self.orders.swap_remove(position);

        self.index.remove(&removed.order_id);

        // swap_remove may have moved the final order
        // into the removed order's former position.
        if let Some(moved) = self.orders.get(position) {
            let index = self
                .index
                .get_mut(&moved.order_id)
                .expect("moved order must have an index");

            *index = position;
        }

        removed
    }

    fn delete(
        &mut self,
        order_id: OrderId,
    ) -> SimulatorResult<()> {
        let position = self.position(order_id)?;

        self.remove_at(position);

        Ok(())
    }

    // --------------------------------------------------------
    // Quantity reduction
    // --------------------------------------------------------

    fn reduce(
        &mut self,
        order_id: OrderId,
        quantity: Quantity,
    ) -> SimulatorResult<()> {
        let position = self.position(order_id)?;

        if quantity.is_zero()
            || !self.instrument.is_quantity_lot_aligned(quantity)
        {
            return Err(
                SimulatorError::InvalidMarketOperation(
                    "invalid order reduction quantity",
                ),
            );
        }

        let current = self.orders[position].quantity.raw();

        let remaining = current
            .checked_sub(quantity.raw())
            .ok_or(
                SimulatorError::InvalidMarketOperation(
                    "reduction exceeds resting order quantity",
                ),
            )?;

        if remaining == 0 {
            self.remove_at(position);
        } else {
            let remaining = Quantity::new(remaining);

            self.instrument
                .validate_order_quantity(remaining)?;

            self.orders[position].quantity = remaining;
        }

        Ok(())
    }

    // --------------------------------------------------------
    // Event application
    // --------------------------------------------------------

    fn apply(
        &mut self,
        envelope: EventEnvelope,
    ) -> SimulatorResult<()> {
        if envelope.instrument_id() != self.id() {
            return Err(
                SimulatorError::InvalidMarketOperation(
                    "event instrument does not match market",
                ),
            );
        }

        envelope.event.validate().map_err(|_| {
            SimulatorError::InvalidMarketOperation(
                "event failed stateless validation",
            )
        })?;

        match envelope.event {
            MarketEvent::OrderAdded(event) => {
                self.add(SimulatedOrder {
                    order_id: event.order_id,
                    side: event.side,
                    price: event.price,
                    quantity: event.quantity,
                })?;
            }

            MarketEvent::OrderModified(event) => {
                self.modify(
                    event.order_id,
                    event.new_price,
                    event.new_quantity,
                )?;
            }

            MarketEvent::OrderCancelled(event) => {
                self.reduce(
                    event.order_id,
                    event.cancelled_quantity,
                )?;
            }

            MarketEvent::OrderDeleted(event) => {
                self.delete(event.order_id)?;
            }

            MarketEvent::OrderExecuted(event) => {
                self.instrument
                    .validate_price(event.execution_price)?;

                self.reduce(
                    event.order_id,
                    event.executed_quantity,
                )?;
            }

            _ => {
                return Err(
                    SimulatorError::InvalidMarketOperation(
                        "unsupported simulated market event",
                    ),
                );
            }
        }

        self.last_sequence = Some(envelope.sequence());

        Ok(())
    }

    // --------------------------------------------------------
    // Consistency validation
    // --------------------------------------------------------

    /// Performs a full consistency check of the order index.
    ///
    /// Intended for tests and diagnostics, not the hot path.
    pub fn validate_consistency(&self) -> SimulatorResult<()> {
        if self.orders.len() != self.index.len() {
            return Err(SimulatorError::InvariantViolation(
                "order vector and index sizes differ",
            ));
        }

        if self.orders.len() > self.capacity {
            return Err(SimulatorError::InvariantViolation(
                "instrument order capacity exceeded",
            ));
        }

        for (position, order) in self.orders.iter().enumerate() {
            if self.index.get(&order.order_id)
                != Some(&position)
            {
                return Err(
                    SimulatorError::InvariantViolation(
                        "order index position is incorrect",
                    ),
                );
            }

            self.instrument.validate_limit_order(
                order.price,
                order.quantity,
            )?;
        }

        Ok(())
    }
}

// ============================================================
// Authoritative market
// ============================================================

/// Authoritative state for one sequenced simulated feed.
///
/// Instrument identifiers are assigned contiguously,
/// starting at one, allowing direct indexed lookup.
#[derive(Debug)]
pub struct Market {
    config: MarketConfig,
    feed: FeedId,
    instruments: Vec<InstrumentMarket>,
    next_expected: SequenceNumber,
    last_sequence: Option<SequenceNumber>,
}

impl Market {
    pub fn new(
        config: MarketConfig,
        feed: FeedId,
    ) -> SimulatorResult<Self> {
        config.validate()?;

        let mut instruments = Vec::new();

        instruments
            .try_reserve_exact(config.instrument_count)
            .map_err(|_| SimulatorError::AllocationFailed)?;

        let distance = config.tick_size
            * i64::from(config.max_price_distance_ticks);

        let minimum_price =
            config.reference_price - distance;

        let maximum_price =
            config.reference_price + distance;

        let maximum_quantity = config.lot_size
            * u64::from(config.max_order_quantity_lots);

        for index in 0..config.instrument_count {
            let id = u32::try_from(index + 1)
                .map_err(|_| {
                    SimulatorError::ArithmeticOverflow(
                        "instrument identifier",
                    )
                })?;

            let instrument = Instrument::new(
                InstrumentConfig {
                    id: InstrumentId::new(id),
                    price_scale: 4,
                    quantity_scale: 0,
                    tick_size: Price::new(config.tick_size),
                    lot_size: Quantity::new(config.lot_size),
                    min_price: Price::new(minimum_price),
                    max_price: Price::new(maximum_price),
                    min_quantity: Quantity::new(config.lot_size),
                    max_quantity: Quantity::new(maximum_quantity),
                },
            )?;

            instruments.push(InstrumentMarket::new(
                instrument,
                config.max_orders_per_instrument,
            )?);
        }

        Ok(Self {
            config,
            feed,
            instruments,
            next_expected: SequenceNumber::ONE,
            last_sequence: None,
        })
    }

    #[must_use]
    pub const fn config(&self) -> MarketConfig {
        self.config
    }

    #[must_use]
    pub const fn feed(&self) -> FeedId {
        self.feed
    }

    #[must_use]
    pub const fn next_expected(&self) -> SequenceNumber {
        self.next_expected
    }

    #[must_use]
    pub const fn last_sequence(
        &self,
    ) -> Option<SequenceNumber> {
        self.last_sequence
    }

    #[must_use]
    pub fn instrument_count(&self) -> usize {
        self.instruments.len()
    }

    #[must_use]
    pub fn instruments(&self) -> &[InstrumentMarket] {
        &self.instruments
    }

    #[must_use]
    pub fn total_orders(&self) -> usize {
        self.instruments
            .iter()
            .map(InstrumentMarket::len)
            .sum()
    }

    #[must_use]
    pub fn instrument(
        &self,
        id: InstrumentId,
    ) -> Option<&InstrumentMarket> {
        let index = id.raw().checked_sub(1)?;

        self.instruments.get(index as usize)
    }

    /// Applies one authoritative event in sequence order.
    ///
    /// The sequence cursor advances only after successful
    /// market mutation.
    pub fn apply(
        &mut self,
        envelope: EventEnvelope,
    ) -> SimulatorResult<()> {
        if envelope.header.feed() != self.feed {
            return Err(
                SimulatorError::InvalidMarketOperation(
                    "event feed does not match authoritative market",
                ),
            );
        }

        let sequence = envelope.sequence();

        if sequence != self.next_expected {
            return Err(SimulatorError::UnexpectedSequence {
                expected: self.next_expected,
                actual: sequence,
            });
        }

        // Validate sequence advancement before mutation.
        let next = sequence
            .raw()
            .checked_add(1)
            .ok_or(SimulatorError::SequenceExhausted)?;

        let id = envelope.instrument_id();

        let index = id
            .raw()
            .checked_sub(1)
            .ok_or(SimulatorError::UnknownInstrument(id))?;

        let instrument = self
            .instruments
            .get_mut(index as usize)
            .ok_or(SimulatorError::UnknownInstrument(id))?;

        instrument.apply(envelope)?;

        self.last_sequence = Some(sequence);
        self.next_expected = SequenceNumber::new(next);

        Ok(())
    }

    /// Validates every instrument's authoritative state.
    pub fn validate_consistency(&self) -> SimulatorResult<()> {
        for instrument in &self.instruments {
            instrument.validate_consistency()?;
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

    use maxwell_core::{
        EventHeader, EventId, OrderAdded, OrderCancelled,
        OrderDeleted, OrderModified, SessionId, StreamId,
    };

    fn feed() -> FeedId {
        FeedId::new(
            StreamId::new(1),
            SessionId::new(1),
        )
    }

    fn market() -> Market {
        Market::new(MarketConfig::default(), feed()).unwrap()
    }

    fn envelope(
        sequence: u64,
        event: MarketEvent,
    ) -> EventEnvelope {
        EventEnvelope::new(
            EventHeader::new(
                EventId::new(
                    feed(),
                    SequenceNumber::new(sequence),
                ),
                InstrumentId::new(1),
                None,
            ),
            event,
        )
    }

    fn add(sequence: u64, id: u64) -> EventEnvelope {
        envelope(
            sequence,
            MarketEvent::OrderAdded(OrderAdded {
                order_id: OrderId::new(id),
                side: Side::Buy,
                price: Price::new(1_000_000),
                quantity: Quantity::new(100),
            }),
        )
    }

    #[test]
    fn market_starts_empty() {
        let market = market();

        assert_eq!(market.instrument_count(), 10);
        assert_eq!(market.total_orders(), 0);
        assert_eq!(market.last_sequence(), None);
        assert_eq!(
            market.next_expected(),
            SequenceNumber::ONE,
        );
    }

    #[test]
    fn addition_advances_sequence() {
        let mut market = market();

        market.apply(add(1, 100)).unwrap();

        assert_eq!(market.total_orders(), 1);

        assert_eq!(
            market.next_expected(),
            SequenceNumber::new(2),
        );

        market.validate_consistency().unwrap();
    }

    #[test]
    fn sequence_gap_is_rejected_without_mutation() {
        let mut market = market();

        assert!(matches!(
            market.apply(add(2, 100)),
            Err(SimulatorError::UnexpectedSequence { .. }),
        ));

        assert_eq!(market.total_orders(), 0);
        assert_eq!(
            market.next_expected(),
            SequenceNumber::ONE,
        );
    }

    #[test]
    fn duplicate_order_is_rejected_atomically() {
        let mut market = market();

        market.apply(add(1, 100)).unwrap();

        assert!(matches!(
            market.apply(add(2, 100)),
            Err(SimulatorError::DuplicateOrder { .. }),
        ));

        assert_eq!(market.total_orders(), 1);

        assert_eq!(
            market.next_expected(),
            SequenceNumber::new(2),
        );
    }

    #[test]
    fn modification_replaces_price_and_quantity() {
        let mut market = market();

        market.apply(add(1, 100)).unwrap();

        market.apply(envelope(
            2,
            MarketEvent::OrderModified(OrderModified {
                order_id: OrderId::new(100),
                new_price: Price::new(999_900),
                new_quantity: Quantity::new(50),
            }),
        ))
        .unwrap();

        let order = market
            .instrument(InstrumentId::new(1))
            .unwrap()
            .order(OrderId::new(100))
            .unwrap();

        assert_eq!(order.price, Price::new(999_900));
        assert_eq!(order.quantity, Quantity::new(50));
    }

    #[test]
    fn partial_cancellation_reduces_quantity() {
        let mut market = market();

        market.apply(add(1, 100)).unwrap();

        market.apply(envelope(
            2,
            MarketEvent::OrderCancelled(OrderCancelled {
                order_id: OrderId::new(100),
                cancelled_quantity: Quantity::new(40),
            }),
        ))
        .unwrap();

        let order = market
            .instrument(InstrumentId::new(1))
            .unwrap()
            .order(OrderId::new(100))
            .unwrap();

        assert_eq!(order.quantity, Quantity::new(60));
    }

    #[test]
    fn full_cancellation_removes_order() {
        let mut market = market();

        market.apply(add(1, 100)).unwrap();

        market.apply(envelope(
            2,
            MarketEvent::OrderCancelled(OrderCancelled {
                order_id: OrderId::new(100),
                cancelled_quantity: Quantity::new(100),
            }),
        ))
        .unwrap();

        assert_eq!(market.total_orders(), 0);
        market.validate_consistency().unwrap();
    }

    #[test]
    fn excessive_cancellation_is_atomic() {
        let mut market = market();

        market.apply(add(1, 100)).unwrap();

        assert!(market.apply(envelope(
            2,
            MarketEvent::OrderCancelled(OrderCancelled {
                order_id: OrderId::new(100),
                cancelled_quantity: Quantity::new(101),
            }),
        ))
        .is_err());

        assert_eq!(market.total_orders(), 1);

        assert_eq!(
            market.next_expected(),
            SequenceNumber::new(2),
        );
    }

    #[test]
    fn swap_removal_repairs_order_index() {
        let mut market = market();

        for sequence in 1..=3 {
            market.apply(add(sequence, sequence)).unwrap();
        }

        market.apply(envelope(
            4,
            MarketEvent::OrderDeleted(OrderDeleted {
                order_id: OrderId::new(1),
            }),
        ))
        .unwrap();

        let instrument = market
            .instrument(InstrumentId::new(1))
            .unwrap();

        assert!(instrument.order(OrderId::new(1)).is_none());
        assert!(instrument.order(OrderId::new(2)).is_some());
        assert!(instrument.order(OrderId::new(3)).is_some());

        market.validate_consistency().unwrap();
    }

    #[test]
    fn instrument_capacity_is_enforced() {
        let config = MarketConfig {
            max_orders_per_instrument: 1,
            ..MarketConfig::default()
        };

        let mut market = Market::new(config, feed()).unwrap();

        market.apply(add(1, 100)).unwrap();

        assert!(matches!(
            market.apply(add(2, 200)),
            Err(SimulatorError::CapacityExceeded { .. }),
        ));

        assert_eq!(market.total_orders(), 1);
    }
}
