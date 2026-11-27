//! Deterministic authoritative market snapshots.
//!
//! A snapshot represents the complete simulated feed
//! through an inclusive upstream sequence watermark.
//!
//! Snapshot capture is a control-path operation and
//! may allocate bounded temporary storage.

use maxwell_core::{
    FeedId, Instrument, InstrumentId, SequenceNumber,
};

use crate::error::{SimulatorError, SimulatorResult};
use crate::market::{Market, SimulatedOrder};

// ============================================================
// Instrument snapshot
// ============================================================

/// Authoritative state of one instrument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentSnapshot {
    instrument: Instrument,
    last_sequence: Option<SequenceNumber>,
    orders: Vec<SimulatedOrder>,
}

impl InstrumentSnapshot {
    #[must_use]
    pub const fn instrument(&self) -> &Instrument {
        &self.instrument
    }

    #[must_use]
    pub const fn instrument_id(&self) -> InstrumentId {
        self.instrument.id()
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
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }
}

// ============================================================
// Feed-wide snapshot
// ============================================================

/// Complete authoritative simulated feed state.
///
/// The watermark is independent of downstream publisher
/// and connection-local sequence numbers.
///
/// A watermark of zero represents an empty feed before
/// its first authoritative event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketSnapshot {
    feed: FeedId,
    watermark: SequenceNumber,
    instruments: Vec<InstrumentSnapshot>,
    order_count: usize,
}

impl MarketSnapshot {
    /// Captures the complete authoritative market.
    ///
    /// Each instrument and resting order consumes one
    /// snapshot entry.
    ///
    /// Capacity is checked before allocating storage.
    pub fn capture(
        market: &Market,
        max_entries: usize,
    ) -> SimulatorResult<Self> {
        if max_entries == 0 {
            return Err(SimulatorError::InvalidArgument(
                "snapshot capacity must be positive",
            ));
        }

        let order_count = market.total_orders();

        let required = market
            .instrument_count()
            .checked_add(order_count)
            .ok_or(SimulatorError::ArithmeticOverflow(
                "snapshot entry count",
            ))?;

        if required > max_entries {
            return Err(
                SimulatorError::SnapshotCapacityExceeded {
                    limit: max_entries,
                    required,
                },
            );
        }

        let watermark = market
            .last_sequence()
            .unwrap_or(SequenceNumber::new(0));

        let mut instruments = Vec::new();

        instruments
            .try_reserve_exact(market.instrument_count())
            .map_err(|_| SimulatorError::AllocationFailed)?;

        for source in market.instruments() {
            let mut orders = Vec::new();

            orders
                .try_reserve_exact(source.len())
                .map_err(|_| {
                    SimulatorError::AllocationFailed
                })?;

            orders.extend_from_slice(source.orders());

            orders.sort_unstable_by_key(|order| {
                order.order_id.raw()
            });

            instruments.push(InstrumentSnapshot {
                instrument: *source.instrument(),
                last_sequence: source.last_sequence(),
                orders,
            });
        }

        instruments.sort_unstable_by_key(|instrument| {
            instrument.instrument_id().raw()
        });

        Ok(Self {
            feed: market.feed(),
            watermark,
            instruments,
            order_count,
        })
    }

    // --------------------------------------------------------
    // Snapshot metadata
    // --------------------------------------------------------

    #[must_use]
    pub const fn feed(&self) -> FeedId {
        self.feed
    }

    #[must_use]
    pub const fn watermark(&self) -> SequenceNumber {
        self.watermark
    }

    /// Returns the first event not covered by the snapshot.
    pub fn next_expected(
        &self,
    ) -> SimulatorResult<SequenceNumber> {
        self.watermark
            .raw()
            .checked_add(1)
            .map(SequenceNumber::new)
            .ok_or(SimulatorError::SequenceExhausted)
    }

    #[must_use]
    pub fn covers(&self, sequence: SequenceNumber) -> bool {
        sequence <= self.watermark
    }

    #[must_use]
    pub fn instrument_count(&self) -> usize {
        self.instruments.len()
    }

    #[must_use]
    pub const fn order_count(&self) -> usize {
        self.order_count
    }

    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.instrument_count() + self.order_count
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.order_count == 0
    }

    // --------------------------------------------------------
    // Instrument lookup
    // --------------------------------------------------------

    #[must_use]
    pub fn instruments(&self) -> &[InstrumentSnapshot] {
        &self.instruments
    }

    #[must_use]
    pub fn instrument(
        &self,
        instrument_id: InstrumentId,
    ) -> Option<&InstrumentSnapshot> {
        let position = self.instruments
            .binary_search_by_key(
                &instrument_id.raw(),
                |instrument| {
                    instrument.instrument_id().raw()
                },
            )
            .ok()?;

        self.instruments.get(position)
    }

    // --------------------------------------------------------
    // Recovery validation
    // --------------------------------------------------------

    /// Checks whether this snapshot can satisfy a recovery
    /// request for the specified feed and minimum watermark.
    pub fn validate_for(
        &self,
        feed: FeedId,
        minimum_watermark: SequenceNumber,
    ) -> SimulatorResult<()> {
        if self.feed != feed {
            return Err(
                SimulatorError::InvalidMarketOperation(
                    "snapshot feed does not match request",
                ),
            );
        }

        if self.watermark < minimum_watermark {
            return Err(
                SimulatorError::InvalidMarketOperation(
                    "snapshot watermark is too old",
                ),
            );
        }

        self.next_expected()?;

        Ok(())
    }

    // --------------------------------------------------------
    // Diagnostics
    // --------------------------------------------------------

    /// Performs full snapshot consistency validation.
    ///
    /// Intended for recovery boundaries, tests, and
    /// diagnostics rather than per-event processing.
    pub fn validate_consistency(&self) -> SimulatorResult<()> {
        let mut previous_instrument = None;
        let mut counted_orders = 0usize;

        for snapshot in &self.instruments {
            let id = snapshot.instrument_id().raw();

            if previous_instrument
                .is_some_and(|previous| previous >= id)
            {
                return Err(
                    SimulatorError::InvariantViolation(
                        "snapshot instruments are not unique and ordered",
                    ),
                );
            }

            previous_instrument = Some(id);

            if snapshot.last_sequence
                .is_some_and(|sequence| {
                    sequence > self.watermark
                })
            {
                return Err(
                    SimulatorError::InvariantViolation(
                        "instrument sequence exceeds snapshot watermark",
                    ),
                );
            }

            if snapshot.last_sequence.is_none()
                && !snapshot.orders.is_empty()
            {
                return Err(
                    SimulatorError::InvariantViolation(
                        "uninitialized instrument contains orders",
                    ),
                );
            }

            let mut previous_order = None;

            for order in &snapshot.orders {
                let id = order.order_id.raw();

                if previous_order
                    .is_some_and(|previous| previous >= id)
                {
                    return Err(
                        SimulatorError::InvariantViolation(
                            "snapshot orders are not unique and ordered",
                        ),
                    );
                }

                previous_order = Some(id);

                snapshot.instrument.validate_limit_order(
                    order.price,
                    order.quantity,
                )?;

                counted_orders = counted_orders
                    .checked_add(1)
                    .ok_or(
                        SimulatorError::ArithmeticOverflow(
                            "snapshot order count",
                        ),
                    )?;
            }
        }

        if counted_orders != self.order_count {
            return Err(
                SimulatorError::InvariantViolation(
                    "snapshot order count is inconsistent",
                ),
            );
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

    use maxwell_core::{SessionId, StreamId};

    use crate::config::SimulatorConfig;
    use crate::generator::EventGenerator;

    fn feed() -> FeedId {
        FeedId::new(
            StreamId::new(1),
            SessionId::new(1),
        )
    }

    fn generator() -> EventGenerator {
        EventGenerator::new(
            SimulatorConfig::default(),
            feed(),
        )
        .unwrap()
    }

    #[test]
    fn empty_snapshot_preserves_registered_instruments() {
        let generator = generator();

        let snapshot = MarketSnapshot::capture(
            generator.market(),
            100,
        )
        .unwrap();

        assert_eq!(snapshot.feed(), feed());
        assert_eq!(snapshot.watermark().raw(), 0);
        assert_eq!(snapshot.next_expected().unwrap().raw(), 1);
        assert_eq!(snapshot.instrument_count(), 10);
        assert_eq!(snapshot.entry_count(), 10);
        assert!(snapshot.is_empty());

        snapshot.validate_consistency().unwrap();
    }

    #[test]
    fn snapshot_captures_authoritative_watermark() {
        let mut generator = generator();

        for _ in 0..100 {
            generator.generate().unwrap().unwrap();
        }

        let snapshot = MarketSnapshot::capture(
            generator.market(),
            65_536,
        )
        .unwrap();

        assert_eq!(snapshot.watermark().raw(), 100);
        assert_eq!(
            snapshot.next_expected().unwrap().raw(),
            101,
        );

        assert_eq!(
            snapshot.order_count(),
            generator.market().total_orders(),
        );

        snapshot.validate_consistency().unwrap();
    }

    #[test]
    fn snapshot_is_independent_of_future_market_mutations() {
        let mut generator = generator();

        generator.generate().unwrap().unwrap();

        let snapshot = MarketSnapshot::capture(
            generator.market(),
            65_536,
        )
        .unwrap();

        for _ in 0..100 {
            generator.generate().unwrap().unwrap();
        }

        assert_eq!(snapshot.watermark().raw(), 1);
        assert_eq!(snapshot.order_count(), 1);
    }

    #[test]
    fn repeated_capture_is_deterministic() {
        let mut generator = generator();

        for _ in 0..1_000 {
            generator.generate().unwrap().unwrap();
        }

        let first = MarketSnapshot::capture(
            generator.market(),
            65_536,
        )
        .unwrap();

        let second = MarketSnapshot::capture(
            generator.market(),
            65_536,
        )
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn snapshot_capacity_is_enforced() {
        let generator = generator();

        assert!(matches!(
            MarketSnapshot::capture(generator.market(), 9),
            Err(
                SimulatorError::SnapshotCapacityExceeded {
                    limit: 9,
                    required: 10,
                }
            ),
        ));
    }

    #[test]
    fn snapshot_supports_exact_capacity() {
        let generator = generator();

        let snapshot = MarketSnapshot::capture(
            generator.market(),
            10,
        )
        .unwrap();

        assert_eq!(snapshot.entry_count(), 10);
    }

    #[test]
    fn instrument_lookup_is_supported() {
        let generator = generator();

        let snapshot = MarketSnapshot::capture(
            generator.market(),
            100,
        )
        .unwrap();

        assert!(
            snapshot.instrument(InstrumentId::new(1)).is_some()
        );

        assert!(
            snapshot.instrument(InstrumentId::new(99)).is_none()
        );
    }

    #[test]
    fn snapshot_rejects_foreign_feed() {
        let generator = generator();

        let snapshot = MarketSnapshot::capture(
            generator.market(),
            100,
        )
        .unwrap();

        let foreign = FeedId::new(
            StreamId::new(2),
            SessionId::new(1),
        );

        assert!(
            snapshot.validate_for(
                foreign,
                SequenceNumber::new(0),
            )
            .is_err()
        );
    }

    #[test]
    fn snapshot_rejects_insufficient_watermark() {
        let generator = generator();

        let snapshot = MarketSnapshot::capture(
            generator.market(),
            100,
        )
        .unwrap();

        assert!(
            snapshot.validate_for(
                feed(),
                SequenceNumber::new(1),
            )
            .is_err()
        );
    }

    #[test]
    fn snapshot_accepts_exact_watermark() {
        let generator = generator();

        let snapshot = MarketSnapshot::capture(
            generator.market(),
            100,
        )
        .unwrap();

        snapshot.validate_for(
            feed(),
            SequenceNumber::new(0),
        )
        .unwrap();
    }
}
