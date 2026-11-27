//! Single-instrument order book reconstruction.

use maxwell_core::{
    FeedId, Instrument, InstrumentId, OrderId, Price, Quantity, SequenceNumber, Side,
};

use crate::config::BookConfig;
use crate::error::{BookError, BookResult};
use crate::ladder::PriceLadder;
use crate::level::PriceLevel;
use crate::order::RestingOrder;
use crate::registry::OrderRegistry;
use crate::update::{BookUpdate, BookUpdateKind};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookState {
    Live = 0,
    RecoveryRequired = 1,
}

#[derive(Debug)]
pub struct OrderBook {
    instrument: Instrument,
    feed: FeedId,
    config: BookConfig,
    bids: PriceLadder,
    asks: PriceLadder,
    orders: OrderRegistry,
    last_sequence: Option<SequenceNumber>,
    state: BookState,
}

impl OrderBook {
    pub fn new(instrument: Instrument, feed: FeedId, config: BookConfig) -> BookResult<Self> {
        config.validate()?;

        let bids = PriceLadder::new(instrument, Side::Buy, config.max_price_levels)?;

        let asks = PriceLadder::new(instrument, Side::Sell, config.max_price_levels)?;

        let orders = OrderRegistry::new(config.max_orders)?;

        Ok(Self {
            instrument,
            feed,
            config,
            bids,
            asks,
            orders,
            last_sequence: None,
            state: BookState::Live,
        })
    }

    #[must_use]
    #[inline]
    pub const fn instrument(&self) -> &Instrument {
        &self.instrument
    }

    #[must_use]
    #[inline]
    pub const fn instrument_id(&self) -> InstrumentId {
        self.instrument.id()
    }

    #[must_use]
    #[inline]
    pub const fn feed(&self) -> FeedId {
        self.feed
    }

    #[must_use]
    #[inline]
    pub const fn config(&self) -> BookConfig {
        self.config
    }

    #[must_use]
    #[inline]
    pub const fn state(&self) -> BookState {
        self.state
    }

    #[must_use]
    #[inline]
    pub const fn last_sequence(&self) -> Option<SequenceNumber> {
        self.last_sequence
    }

    #[must_use]
    #[inline]
    pub const fn bids(&self) -> &PriceLadder {
        &self.bids
    }

    #[must_use]
    #[inline]
    pub const fn asks(&self) -> &PriceLadder {
        &self.asks
    }

    #[must_use]
    #[inline]
    pub const fn orders(&self) -> &OrderRegistry {
        &self.orders
    }

    #[must_use]
    #[inline]
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }

    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    #[must_use]
    #[inline]
    pub fn best_bid(&self) -> Option<(Price, PriceLevel)> {
        self.bids.best_level()
    }

    #[must_use]
    #[inline]
    pub fn best_ask(&self) -> Option<(Price, PriceLevel)> {
        self.asks.best_level()
    }

    #[must_use]
    #[inline]
    pub fn is_crossed(&self) -> bool {
        match (self.bids.best_price(), self.asks.best_price()) {
            (Some(bid), Some(ask)) => bid > ask,
            _ => false,
        }
    }

    #[must_use]
    #[inline]
    pub fn is_locked(&self) -> bool {
        match (self.bids.best_price(), self.asks.best_price()) {
            (Some(bid), Some(ask)) => bid == ask,
            _ => false,
        }
    }

    #[must_use]
    #[inline]
    pub const fn ladder(&self, side: Side) -> &PriceLadder {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    #[inline]
    fn ladder_mut(&mut self, side: Side) -> &mut PriceLadder {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    /// Applies one normalized authoritative book update.
    ///
    /// Sequence numbers need only increase because a feed
    /// may carry updates for multiple instruments.
    ///
    /// A failed update marks the book as requiring recovery.
    pub fn apply(&mut self, update: BookUpdate) -> BookResult<()> {
        if self.state == BookState::RecoveryRequired {
            return Err(BookError::RecoveryRequired);
        }

        let result = self.apply_checked(update);

        if result.is_err() {
            self.state = BookState::RecoveryRequired;
        }

        result
    }

    fn apply_checked(&mut self, update: BookUpdate) -> BookResult<()> {
        if update.instrument_id != self.instrument_id() {
            return Err(BookError::InstrumentMismatch {
                expected: self.instrument_id(),
                actual: update.instrument_id,
            });
        }

        if update.feed != self.feed {
            return Err(BookError::RecoveryRequired);
        }

        if let Some(previous) = self.last_sequence {
            if update.sequence <= previous {
                return Err(BookError::NonIncreasingSequence {
                    previous,
                    received: update.sequence,
                });
            }
        }

        match update.kind {
            BookUpdateKind::Add {
                order_id,
                side,
                price,
                quantity,
            } => {
                let order = RestingOrder::new(order_id, side, price, quantity, update.sequence)?;

                self.add_order(order)?;
            }

            BookUpdateKind::Reduce { order_id, quantity }
            | BookUpdateKind::Execute { order_id, quantity } => {
                self.reduce_order(order_id, quantity)?;
            }

            BookUpdateKind::Delete { order_id } => {
                self.delete_order(order_id)?;
            }

            BookUpdateKind::Replace {
                order_id,
                new_price,
                new_quantity,
            } => {
                self.replace_order(order_id, new_price, new_quantity, update.sequence)?;
            }

            BookUpdateKind::Observe => {}
        }

        self.last_sequence = Some(update.sequence);

        Ok(())
    }

    /// Validates a resting order.
    ///
    /// Original order-size minimums are intentionally not
    /// enforced here: partially filled resting orders may
    /// be smaller than the original submission minimum.
    fn validate_resting_order(&self, order: RestingOrder) -> BookResult<()> {
        self.instrument.validate_price(order.price())?;

        if order.quantity().is_zero() {
            return Err(BookError::ZeroQuantity);
        }

        Ok(())
    }

    pub(crate) fn add_order(&mut self, order: RestingOrder) -> BookResult<()> {
        self.validate_resting_order(order)?;
        self.orders.validate_insert(order.order_id())?;

        let side = order.side();
        let price = order.price();

        let index = self
            .ladder(side)
            .index_of(price)
            .ok_or(BookError::InvalidPrice(price))?;

        let mut level = self
            .ladder(side)
            .level_at(index)
            .ok_or(BookError::InvalidPriceLevel)?;

        level.add_order(order.quantity())?;

        // Commit only after all aggregate arithmetic
        // has succeeded.
        self.orders.insert(order)?;
        self.ladder_mut(side).set_level_at(index, level);

        Ok(())
    }

    fn reduce_order(&mut self, order_id: OrderId, quantity: Quantity) -> BookResult<()> {
        let order = self
            .orders
            .get(order_id)
            .ok_or(BookError::UnknownOrder(order_id))?;

        if quantity.is_zero() || quantity > order.quantity() {
            return Err(BookError::InvalidReduction);
        }

        if quantity == order.quantity() {
            return self.delete_order(order_id);
        }

        let reduced = order.reduced(quantity)?;
        let side = order.side();

        let index = self
            .ladder(side)
            .index_of(order.price())
            .ok_or(BookError::InvalidPrice(order.price()))?;

        let mut level = self
            .ladder(side)
            .level_at(index)
            .ok_or(BookError::InvalidPriceLevel)?;

        level.reduce_quantity(quantity)?;

        self.orders.update(reduced)?;
        self.ladder_mut(side).set_level_at(index, level);

        Ok(())
    }

    fn delete_order(&mut self, order_id: OrderId) -> BookResult<()> {
        let order = self
            .orders
            .get(order_id)
            .ok_or(BookError::UnknownOrder(order_id))?;

        let side = order.side();

        let index = self
            .ladder(side)
            .index_of(order.price())
            .ok_or(BookError::InvalidPrice(order.price()))?;

        let mut level = self
            .ladder(side)
            .level_at(index)
            .ok_or(BookError::InvalidPriceLevel)?;

        level.remove_order(order.quantity())?;

        self.orders.remove(order_id)?;
        self.ladder_mut(side).set_level_at(index, level);

        Ok(())
    }

    fn replace_order(
        &mut self,
        order_id: OrderId,
        new_price: Price,
        new_quantity: Quantity,
        sequence: SequenceNumber,
    ) -> BookResult<()> {
        let previous = self
            .orders
            .get(order_id)
            .ok_or(BookError::UnknownOrder(order_id))?;

        let side = previous.side();

        let replacement = RestingOrder::new(order_id, side, new_price, new_quantity, sequence)?;

        self.validate_resting_order(replacement)?;

        let old_index = self
            .ladder(side)
            .index_of(previous.price())
            .ok_or(BookError::InvalidPrice(previous.price()))?;

        let new_index = self
            .ladder(side)
            .index_of(new_price)
            .ok_or(BookError::InvalidPrice(new_price))?;

        let mut old_level = self
            .ladder(side)
            .level_at(old_index)
            .ok_or(BookError::InvalidPriceLevel)?;

        old_level.remove_order(previous.quantity())?;

        if old_index == new_index {
            old_level.add_order(new_quantity)?;

            self.orders.update(replacement)?;

            self.ladder_mut(side).set_level_at(old_index, old_level);
        } else {
            let mut new_level = self
                .ladder(side)
                .level_at(new_index)
                .ok_or(BookError::InvalidPriceLevel)?;

            new_level.add_order(new_quantity)?;

            self.orders.update(replacement)?;

            let ladder = self.ladder_mut(side);

            ladder.set_level_at(old_index, old_level);
            ladder.set_level_at(new_index, new_level);
        }

        Ok(())
    }

    /// Marks the book stale without destroying its last
    /// known consistent state.
    pub fn require_recovery(&mut self) {
        self.state = BookState::RecoveryRequired;
    }

    /// Explicitly initializes an empty book for a new
    /// authoritative feed session.
    ///
    /// The caller must establish that the new session
    /// starts with an empty exchange book.
    pub fn reset_empty_session(&mut self, feed: FeedId) -> BookResult<()> {
        if feed == self.feed {
            return Err(BookError::InvalidSnapshot);
        }

        self.bids.clear();
        self.asks.clear();
        self.orders.clear();

        self.feed = feed;
        self.last_sequence = None;
        self.state = BookState::Live;

        Ok(())
    }

    pub(crate) fn set_snapshot_watermark(&mut self, sequence: Option<SequenceNumber>) {
        self.last_sequence = sequence;
    }

    /// Performs an expensive consistency audit.
    ///
    /// This is intended for diagnostics, tests, and
    /// recovery, not the steady-state processing path.
    pub fn validate_consistency(&self) -> BookResult<()> {
        let mut bid_levels = Vec::new();
        let mut ask_levels = Vec::new();

        bid_levels
            .try_reserve_exact(self.bids.capacity())
            .map_err(|_| BookError::AllocationFailed)?;

        ask_levels
            .try_reserve_exact(self.asks.capacity())
            .map_err(|_| BookError::AllocationFailed)?;

        bid_levels.resize(self.bids.capacity(), PriceLevel::EMPTY);
        ask_levels.resize(self.asks.capacity(), PriceLevel::EMPTY);

        for order in self.orders.iter() {
            self.validate_resting_order(*order)?;

            let index = self
                .ladder(order.side())
                .index_of(order.price())
                .ok_or(BookError::InconsistentBook)?;

            let levels = match order.side() {
                Side::Buy => &mut bid_levels,
                Side::Sell => &mut ask_levels,
            };

            levels[index].add_order(order.quantity())?;
        }

        for (ladder, expected) in [(&self.bids, &bid_levels), (&self.asks, &ask_levels)] {
            let mut active = 0;

            for (index, level) in expected.iter().enumerate() {
                if ladder.level_at(index) != Some(*level) {
                    return Err(BookError::InconsistentBook);
                }

                if !level.is_empty() {
                    active += 1;
                }
            }

            if ladder.active_levels() != active {
                return Err(BookError::InconsistentBook);
            }

            let best = match ladder.side() {
                Side::Buy => expected.iter().rposition(|level| !level.is_empty()),

                Side::Sell => expected.iter().position(|level| !level.is_empty()),
            };

            let expected_best = best.and_then(|index| ladder.price_at(index));

            if ladder.best_price() != expected_best {
                return Err(BookError::InconsistentBook);
            }

            let mut actual_indices = ladder.iter();

            let expected_iter: Box<dyn Iterator<Item = usize>> = match ladder.side() {
                Side::Buy => Box::new((0..expected.len()).rev()),

                Side::Sell => Box::new(0..expected.len()),
            };

            for index in expected_iter {
                if expected[index].is_empty() {
                    continue;
                }

                let price = ladder.price_at(index).ok_or(BookError::InconsistentBook)?;

                if actual_indices.next() != Some((price, expected[index])) {
                    return Err(BookError::InconsistentBook);
                }
            }

            if actual_indices.next().is_some() {
                return Err(BookError::InconsistentBook);
            }
        }

        Ok(())
    }
}
