//! Deterministic authoritative market event generation.

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, MarketEvent,
    OrderAdded, OrderCancelled, OrderDeleted, OrderExecuted,
    OrderId, OrderModified, Price, Quantity, Side, TradeId,
};

use crate::clock::VirtualClock;
use crate::config::{
    EventDistribution, MarketConfig, SimulatorConfig,
};
use crate::error::{SimulatorError, SimulatorResult};
use crate::market::{InstrumentMarket, Market};
use crate::rng::{MARKET_STREAM, SplitMix64};

// ============================================================
// Operation selection
// ============================================================

/// Supported authoritative market operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Add,
    Modify,
    Cancel,
    Delete,
    Execute,
}

impl Operation {
    /// Selects an operation using a validated distribution.
    fn sample(
        distribution: EventDistribution,
        rng: &mut SplitMix64,
    ) -> Self {
        let mut value = rng.basis_points();

        if value < distribution.add {
            return Self::Add;
        }

        value -= distribution.add;

        if value < distribution.modify {
            return Self::Modify;
        }

        value -= distribution.modify;

        if value < distribution.cancel {
            return Self::Cancel;
        }

        value -= distribution.cancel;

        if value < distribution.delete {
            return Self::Delete;
        }

        Self::Execute
    }
}

// ============================================================
// Market event generator
// ============================================================

/// Generates and applies authoritative market events.
///
/// The generator owns its market, random stream, virtual
/// clock, and unique identifier cursor.
#[derive(Debug)]
pub struct EventGenerator {
    config: SimulatorConfig,
    market: Market,
    rng: SplitMix64,
    clock: VirtualClock,
    next_order_id: u64,
    generated: u64,
}

impl EventGenerator {
    pub fn new(
        config: SimulatorConfig,
        feed: FeedId,
    ) -> SimulatorResult<Self> {
        config.validate()?;

        let market = Market::new(config.market, feed)?;

        let rng = SplitMix64::for_stream(
            config.run.seed,
            MARKET_STREAM,
        );

        let clock = VirtualClock::new(
            config.run.start_timestamp_ns,
            config.pacing.event_interval_ns,
        )?;

        Ok(Self {
            config,
            market,
            rng,
            clock,
            next_order_id: 1,
            generated: 0,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &SimulatorConfig {
        &self.config
    }

    #[must_use]
    pub const fn market(&self) -> &Market {
        &self.market
    }

    #[must_use]
    pub const fn clock(&self) -> &VirtualClock {
        &self.clock
    }

    #[must_use]
    pub const fn generated(&self) -> u64 {
        self.generated
    }

    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.config.run.event_count - self.generated
    }

    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.generated == self.config.run.event_count
    }

    // --------------------------------------------------------
    // Price and quantity generation
    // --------------------------------------------------------

    fn random_price(
        config: MarketConfig,
        side: Side,
        rng: &mut SplitMix64,
    ) -> SimulatorResult<Price> {
        let ticks = rng.inclusive(
            1,
            u64::from(config.max_price_distance_ticks),
        )?;

        let distance = config.tick_size * ticks as i64;

        let price = match side {
            Side::Buy => config.reference_price - distance,
            Side::Sell => config.reference_price + distance,
        };

        Ok(Price::new(price))
    }

    fn random_quantity(
        config: MarketConfig,
        rng: &mut SplitMix64,
    ) -> SimulatorResult<Quantity> {
        let lots = rng.inclusive(
            1,
            u64::from(config.max_order_quantity_lots),
        )?;

        Ok(Quantity::new(config.lot_size * lots))
    }

    fn reduction_quantity(
        config: MarketConfig,
        resting: Quantity,
        rng: &mut SplitMix64,
    ) -> SimulatorResult<Quantity> {
        let resting_lots =
            resting.raw() / config.lot_size;

        if resting_lots == 0 {
            return Err(SimulatorError::InvariantViolation(
                "resting order contains no complete lots",
            ));
        }

        let lots = rng.inclusive(1, resting_lots)?;

        Ok(Quantity::new(config.lot_size * lots))
    }

    // --------------------------------------------------------
    // Operation eligibility
    // --------------------------------------------------------

    fn eligible_operation(
        instrument: &InstrumentMarket,
        preferred: Operation,
    ) -> Operation {
        if instrument.is_empty() {
            return Operation::Add;
        }

        if instrument.is_full()
            && preferred == Operation::Add
        {
            // Free capacity rather than attempting an
            // invalid addition.
            return Operation::Delete;
        }

        preferred
    }

    // --------------------------------------------------------
    // Event construction
    // --------------------------------------------------------

    fn construct_event(
        config: MarketConfig,
        instrument: &InstrumentMarket,
        operation: Operation,
        sequence: u64,
        next_order_id: u64,
        rng: &mut SplitMix64,
    ) -> SimulatorResult<MarketEvent> {
        if operation == Operation::Add {
            let side = if rng.next_bool() {
                Side::Buy
            } else {
                Side::Sell
            };

            return Ok(MarketEvent::OrderAdded(OrderAdded {
                order_id: OrderId::new(next_order_id),
                side,
                price: Self::random_price(
                    config,
                    side,
                    rng,
                )?,
                quantity: Self::random_quantity(
                    config,
                    rng,
                )?,
            }));
        }

        let index = rng.index(instrument.len())?;

        let order = instrument.orders()[index];

        let event = match operation {
            Operation::Add => {
                return Err(
                    SimulatorError::InvariantViolation(
                        "addition reached existing-order path",
                    ),
                );
            }

            Operation::Modify => {
                MarketEvent::OrderModified(OrderModified {
                    order_id: order.order_id,
                    new_price: Self::random_price(
                        config,
                        order.side,
                        rng,
                    )?,
                    new_quantity: Self::random_quantity(
                        config,
                        rng,
                    )?,
                })
            }

            Operation::Cancel => {
                MarketEvent::OrderCancelled(OrderCancelled {
                    order_id: order.order_id,
                    cancelled_quantity:
                        Self::reduction_quantity(
                            config,
                            order.quantity,
                            rng,
                        )?,
                })
            }

            Operation::Delete => {
                MarketEvent::OrderDeleted(OrderDeleted {
                    order_id: order.order_id,
                })
            }

            Operation::Execute => {
                MarketEvent::OrderExecuted(OrderExecuted {
                    order_id: order.order_id,

                    // Each synthetic execution has a unique
                    // trade identifier within this feed.
                    trade_id: TradeId::new(sequence),

                    execution_price: order.price,

                    executed_quantity:
                        Self::reduction_quantity(
                            config,
                            order.quantity,
                            rng,
                        )?,
                })
            }
        };

        Ok(event)
    }

    // --------------------------------------------------------
    // Authoritative event generation
    // --------------------------------------------------------

    /// Generates and applies the next authoritative event.
    ///
    /// Returns None after the configured event count has
    /// been reached.
    ///
    /// The generator's cursors and random stream are
    /// committed only after successful market mutation.
    pub fn generate(
        &mut self,
    ) -> SimulatorResult<Option<EventEnvelope>> {
        if self.is_finished() {
            return Ok(None);
        }

        // Work on local copies so a failed operation
        // does not consume random values or timestamps.
        let mut rng = self.rng;
        let mut clock = self.clock;

        let instrument_index =
            rng.index(self.market.instrument_count())?;

        let instrument =
            &self.market.instruments()[instrument_index];

        let instrument_id = instrument.id();

        let preferred = Operation::sample(
            self.config.distribution,
            &mut rng,
        );

        let operation = Self::eligible_operation(
            instrument,
            preferred,
        );

        let sequence = self.market.next_expected();

        let event = Self::construct_event(
            self.config.market,
            instrument,
            operation,
            sequence.raw(),
            self.next_order_id,
            &mut rng,
        )?;

        let next_order_id =
            if operation == Operation::Add {
                self.next_order_id
                    .checked_add(1)
                    .ok_or(
                        SimulatorError::OrderIdExhausted,
                    )?
            } else {
                self.next_order_id
            };

        let generated = self.generated
            .checked_add(1)
            .ok_or(SimulatorError::ArithmeticOverflow(
                "generated event count",
            ))?;

        let envelope = EventEnvelope::new(
            EventHeader::new(
                EventId::new(
                    self.market.feed(),
                    sequence,
                ),
                instrument_id,
                Some(clock.exchange_timestamp()),
            ),
            event,
        );

        // Validate timestamp advancement before applying
        // the authoritative event.
        clock.advance()?;

        self.market.apply(envelope)?;

        self.rng = rng;
        self.clock = clock;
        self.next_order_id = next_order_id;
        self.generated = generated;

        Ok(Some(envelope))
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{
        EventKind, SequenceNumber, SessionId, StreamId,
    };

    fn feed() -> FeedId {
        FeedId::new(
            StreamId::new(1),
            SessionId::new(1),
        )
    }

    fn config(event_count: u64) -> SimulatorConfig {
        let mut config = SimulatorConfig::default();

        config.run.event_count = event_count;

        config
    }

    #[test]
    fn generator_starts_with_empty_market() {
        let generator =
            EventGenerator::new(config(100), feed()).unwrap();

        assert_eq!(generator.generated(), 0);
        assert_eq!(generator.remaining(), 100);
        assert_eq!(generator.market().total_orders(), 0);
        assert!(!generator.is_finished());
    }

    #[test]
    fn first_event_is_an_addition() {
        let mut generator =
            EventGenerator::new(config(100), feed()).unwrap();

        let event = generator.generate().unwrap().unwrap();

        assert_eq!(event.kind(), EventKind::OrderAdded);
        assert_eq!(event.sequence(), SequenceNumber::ONE);
        assert_eq!(generator.market().total_orders(), 1);
    }

    #[test]
    fn generation_stops_at_configured_event_count() {
        let mut generator =
            EventGenerator::new(config(10), feed()).unwrap();

        for _ in 0..10 {
            assert!(generator.generate().unwrap().is_some());
        }

        assert!(generator.generate().unwrap().is_none());
        assert!(generator.generate().unwrap().is_none());

        assert!(generator.is_finished());
        assert_eq!(generator.generated(), 10);
        assert_eq!(generator.remaining(), 0);
    }

    #[test]
    fn identical_seeds_generate_identical_events() {
        let mut first =
            EventGenerator::new(config(1_000), feed()).unwrap();

        let mut second =
            EventGenerator::new(config(1_000), feed()).unwrap();

        for _ in 0..1_000 {
            assert_eq!(
                first.generate().unwrap(),
                second.generate().unwrap(),
            );
        }
    }

    #[test]
    fn delivery_fault_settings_do_not_change_generation() {
        let first_config = config(1_000);
        let mut second_config = first_config;

        second_config.faults.drop_bps = 100;
        second_config.faults.duplicate_bps = 200;
        second_config.faults.reorder_bps = 300;

        let mut first =
            EventGenerator::new(first_config, feed()).unwrap();

        let mut second =
            EventGenerator::new(second_config, feed()).unwrap();

        for _ in 0..1_000 {
            assert_eq!(
                first.generate().unwrap(),
                second.generate().unwrap(),
            );
        }
    }

    #[test]
    fn generated_sequences_are_contiguous() {
        let mut generator =
            EventGenerator::new(config(1_000), feed()).unwrap();

        for expected in 1..=1_000 {
            let event =
                generator.generate().unwrap().unwrap();

            assert_eq!(
                event.sequence(),
                SequenceNumber::new(expected),
            );
        }

        assert_eq!(
            generator.market().next_expected(),
            SequenceNumber::new(1_001),
        );
    }

    #[test]
    fn exchange_timestamps_advance_deterministically() {
        let mut config = config(3);

        config.run.start_timestamp_ns = 100;
        config.pacing.event_interval_ns = 20;

        let mut generator =
            EventGenerator::new(config, feed()).unwrap();

        for expected in [100, 120, 140] {
            let event =
                generator.generate().unwrap().unwrap();

            assert_eq!(
                event.header.exchange_timestamp.unwrap().raw(),
                expected,
            );
        }

        assert_eq!(generator.clock().current_ns(), 160);
    }

    #[test]
    fn generated_market_remains_consistent() {
        let mut generator =
            EventGenerator::new(config(10_000), feed()).unwrap();

        while generator.generate().unwrap().is_some() {
            assert!(
                generator.market().total_orders()
                    <= generator.config()
                        .market
                        .max_total_orders()
                        .unwrap()
            );
        }

        generator.market().validate_consistency().unwrap();
    }

    #[test]
    fn generator_respects_small_instrument_capacity() {
        let mut config = config(1_000);

        config.market.instrument_count = 1;
        config.market.max_orders_per_instrument = 1;

        let mut generator =
            EventGenerator::new(config, feed()).unwrap();

        while generator.generate().unwrap().is_some() {
            assert!(generator.market().total_orders() <= 1);
        }

        generator.market().validate_consistency().unwrap();
    }

    #[test]
    fn generated_order_identifiers_are_unique() {
        let mut generator =
            EventGenerator::new(config(10_000), feed()).unwrap();

        let mut previous = 0;

        while let Some(event) = generator.generate().unwrap() {
            if let MarketEvent::OrderAdded(add) = event.event {
                assert!(add.order_id.raw() > previous);

                previous = add.order_id.raw();
            }
        }
    }

    #[test]
    fn generated_resting_prices_do_not_cross_reference() {
        let config = config(10_000);

        let mut generator =
            EventGenerator::new(config, feed()).unwrap();

        while generator.generate().unwrap().is_some() {}

        for instrument in generator.market().instruments() {
            for order in instrument.orders() {
                match order.side {
                    Side::Buy => {
                        assert!(
                            order.price.raw()
                                < config.market.reference_price
                        );
                    }

                    Side::Sell => {
                        assert!(
                            order.price.raw()
                                > config.market.reference_price
                        );
                    }
                }
            }
        }
    }
}
