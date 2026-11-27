//! Simulator configuration and resource limits.
//!
//! Configuration is divided into independent concerns:
//!
//! - Market generation and instrument capacity.
//! - Relative event generation probabilities.
//! - Delivery fault probabilities.
//! - History and delivery buffer capacity.
//! - Virtual clock and real-time pacing.
//!
//! All probabilities are represented in basis points.

use crate::error::{SimulatorError, SimulatorResult};

// ============================================================
// Constants
// ============================================================

/// Basis points representing a probability of 100%.
pub const BASIS_POINTS: u32 = 10_000;

/// Default deterministic random seed.
pub const DEFAULT_SEED: u64 = 42;

/// Default number of simulated instruments.
pub const DEFAULT_INSTRUMENT_COUNT: usize = 10;

/// Default maximum resting orders per instrument.
pub const DEFAULT_MAX_ORDERS_PER_INSTRUMENT: usize = 1_024;

/// Default number of generated authoritative events.
pub const DEFAULT_EVENT_COUNT: u64 = 100_000;

/// Default retained authoritative event history.
pub const DEFAULT_HISTORY_CAPACITY: usize = 65_536;

/// Default maximum pending delivery messages.
pub const DEFAULT_DELIVERY_CAPACITY: usize = 4_096;

/// Default maximum number of snapshot entries.
pub const DEFAULT_SNAPSHOT_CAPACITY: usize = 65_536;

/// Default virtual timestamp increment.
pub const DEFAULT_EVENT_INTERVAL_NS: u64 = 20_000;

/// Default target real-time event generation rate.
pub const DEFAULT_EVENTS_PER_SECOND: u64 = 50_000;

/// Default maximum reordering window.
pub const DEFAULT_REORDER_WINDOW: usize = 8;

/// Maximum supported instrument count.
pub const MAX_INSTRUMENT_COUNT: usize = 65_536;

/// Number of nanoseconds in one second.
pub const NANOS_PER_SECOND: u64 = 1_000_000_000;

// ============================================================
// Market configuration
// ============================================================

/// Configuration governing the authoritative simulated market.
///
/// Prices and quantities are expressed as raw fixed-point
/// integer values. Instrument-specific scales will be
/// established by the market implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketConfig {
    /// Number of independently simulated instruments.
    pub instrument_count: usize,

    /// Maximum resting orders in each instrument.
    pub max_orders_per_instrument: usize,

    /// Initial reference price.
    pub reference_price: i64,

    /// Minimum allowed price increment.
    pub tick_size: i64,

    /// Minimum allowed quantity increment.
    pub lot_size: u64,

    /// Maximum number of ticks away from the reference price.
    pub max_price_distance_ticks: u32,

    /// Maximum generated order size in lots.
    pub max_order_quantity_lots: u32,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            instrument_count: DEFAULT_INSTRUMENT_COUNT,
            max_orders_per_instrument: DEFAULT_MAX_ORDERS_PER_INSTRUMENT,
            reference_price: 1_000_000,
            tick_size: 100,
            lot_size: 1,
            max_price_distance_ticks: 100,
            max_order_quantity_lots: 100,
        }
    }
}

impl MarketConfig {
    /// Validates market generation parameters.
    pub fn validate(&self) -> SimulatorResult<()> {
        if self.instrument_count == 0 || self.instrument_count > MAX_INSTRUMENT_COUNT {
            return Err(SimulatorError::InvalidConfig(
                "instrument count is outside the supported range",
            ));
        }

        if self.max_orders_per_instrument == 0 {
            return Err(SimulatorError::InvalidConfig(
                "maximum orders per instrument must be positive",
            ));
        }

        if self.reference_price <= 0 {
            return Err(SimulatorError::InvalidConfig(
                "reference price must be positive",
            ));
        }

        if self.tick_size <= 0 {
            return Err(SimulatorError::InvalidConfig("tick size must be positive"));
        }

        if self.lot_size == 0 {
            return Err(SimulatorError::InvalidConfig("lot size must be positive"));
        }

        if self.reference_price % self.tick_size != 0 {
            return Err(SimulatorError::InvalidConfig(
                "reference price must be aligned to tick size",
            ));
        }

        if self.max_price_distance_ticks == 0 {
            return Err(SimulatorError::InvalidConfig(
                "maximum price distance must be positive",
            ));
        }

        if self.max_order_quantity_lots == 0 {
            return Err(SimulatorError::InvalidConfig(
                "maximum order quantity must be positive",
            ));
        }

        let distance = self
            .tick_size
            .checked_mul(i64::from(self.max_price_distance_ticks))
            .ok_or(SimulatorError::InvalidConfig(
                "configured price distance overflows",
            ))?;

        let minimum_price =
            self.reference_price
                .checked_sub(distance)
                .ok_or(SimulatorError::InvalidConfig(
                    "minimum generated price overflows",
                ))?;

        if minimum_price <= 0 {
            return Err(SimulatorError::InvalidConfig(
                "minimum generated price must be positive",
            ));
        }

        self.reference_price
            .checked_add(distance)
            .ok_or(SimulatorError::InvalidConfig(
                "maximum generated price overflows",
            ))?;

        self.lot_size
            .checked_mul(u64::from(self.max_order_quantity_lots))
            .ok_or(SimulatorError::InvalidConfig(
                "maximum generated quantity overflows",
            ))?;

        self.max_total_orders()?;

        Ok(())
    }

    /// Returns the maximum total resting order capacity.
    pub fn max_total_orders(&self) -> SimulatorResult<usize> {
        self.instrument_count
            .checked_mul(self.max_orders_per_instrument)
            .ok_or(SimulatorError::InvalidConfig(
                "total resting order capacity overflows",
            ))
    }
}

// ============================================================
// Event distribution
// ============================================================

/// Relative probabilities for authoritative market events.
///
/// The weights must sum to exactly 10,000 basis points.
///
/// These weights describe preferred event selection.
/// The generator may select an alternative valid event
/// when the preferred operation is impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDistribution {
    pub add: u32,
    pub modify: u32,
    pub cancel: u32,
    pub delete: u32,
    pub execute: u32,
}

impl Default for EventDistribution {
    fn default() -> Self {
        Self {
            add: 4_000,
            modify: 2_000,
            cancel: 1_500,
            delete: 1_000,
            execute: 1_500,
        }
    }
}

impl EventDistribution {
    /// Returns the total configured event weight.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.add as u64
            + self.modify as u64
            + self.cancel as u64
            + self.delete as u64
            + self.execute as u64
    }

    /// Validates the event probability distribution.
    pub fn validate(&self) -> SimulatorResult<()> {
        if self.total() != u64::from(BASIS_POINTS) {
            return Err(SimulatorError::InvalidConfig(
                "event weights must sum to 10000 basis points",
            ));
        }

        if self.add == 0 {
            return Err(SimulatorError::InvalidConfig(
                "order addition weight must be positive",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Fault configuration
// ============================================================

/// Probabilities governing faulty event delivery.
///
/// Faults are applied after authoritative event generation
/// and history recording.
///
/// Drop, duplicate, and reorder are mutually exclusive
/// primary delivery decisions for an individual event.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FaultConfig {
    /// Probability of dropping an event.
    pub drop_bps: u32,

    /// Probability of delivering an event twice.
    pub duplicate_bps: u32,

    /// Probability of temporarily withholding an event.
    pub reorder_bps: u32,

    /// Maximum number of events retained for reordering.
    ///
    /// Zero selects the default window when reordering
    /// is enabled.
    pub reorder_window: usize,
}

impl FaultConfig {
    /// Returns a fault-free delivery configuration.
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            drop_bps: 0,
            duplicate_bps: 0,
            reorder_bps: 0,
            reorder_window: 0,
        }
    }

    /// Returns whether any delivery fault is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.drop_bps != 0 || self.duplicate_bps != 0 || self.reorder_bps != 0
    }

    /// Returns the combined primary fault probability.
    #[must_use]
    pub const fn total_bps(&self) -> u64 {
        self.drop_bps as u64 + self.duplicate_bps as u64 + self.reorder_bps as u64
    }

    /// Returns the effective reordering window.
    #[must_use]
    pub const fn effective_reorder_window(&self) -> usize {
        if self.reorder_bps == 0 {
            0
        } else if self.reorder_window == 0 {
            DEFAULT_REORDER_WINDOW
        } else {
            self.reorder_window
        }
    }

    /// Validates delivery fault probabilities.
    pub fn validate(&self) -> SimulatorResult<()> {
        if self.total_bps() > u64::from(BASIS_POINTS) {
            return Err(SimulatorError::InvalidConfig(
                "combined fault probability exceeds 10000 basis points",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Resource configuration
// ============================================================

/// Bounded simulator resource capacities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceConfig {
    /// Maximum authoritative events retained for recovery.
    pub history_capacity: usize,

    /// Maximum pending outbound delivery messages.
    pub delivery_capacity: usize,

    /// Maximum entries in one authoritative snapshot.
    pub snapshot_capacity: usize,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            history_capacity: DEFAULT_HISTORY_CAPACITY,
            delivery_capacity: DEFAULT_DELIVERY_CAPACITY,
            snapshot_capacity: DEFAULT_SNAPSHOT_CAPACITY,
        }
    }
}

impl ResourceConfig {
    /// Validates resource capacities.
    pub fn validate(&self) -> SimulatorResult<()> {
        if self.history_capacity == 0 {
            return Err(SimulatorError::InvalidConfig(
                "history capacity must be positive",
            ));
        }

        if self.delivery_capacity == 0 {
            return Err(SimulatorError::InvalidConfig(
                "delivery capacity must be positive",
            ));
        }

        if self.snapshot_capacity == 0 {
            return Err(SimulatorError::InvalidConfig(
                "snapshot capacity must be positive",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Pacing configuration
// ============================================================

/// Event generation pacing strategy.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PacingMode {
    /// Advance virtual time without sleeping.
    #[default]
    Virtual,

    /// Generate events as quickly as possible.
    Unbounded,

    /// Pace event generation against a monotonic clock.
    RealTime,
}

/// Configuration governing simulated timestamps and pacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacingConfig {
    pub mode: PacingMode,

    /// Virtual timestamp increment per authoritative event.
    pub event_interval_ns: u64,

    /// Target generation rate when real-time pacing is enabled.
    pub events_per_second: u64,
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self {
            mode: PacingMode::Virtual,
            event_interval_ns: DEFAULT_EVENT_INTERVAL_NS,
            events_per_second: DEFAULT_EVENTS_PER_SECOND,
        }
    }
}

impl PacingConfig {
    /// Validates timestamp and rate parameters.
    pub fn validate(&self) -> SimulatorResult<()> {
        if self.event_interval_ns == 0 {
            return Err(SimulatorError::InvalidConfig(
                "event timestamp interval must be positive",
            ));
        }

        if self.events_per_second == 0 || self.events_per_second > NANOS_PER_SECOND {
            return Err(SimulatorError::InvalidConfig(
                "event rate is outside the supported range",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Simulation run configuration
// ============================================================

/// Configuration governing one simulation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunConfig {
    /// Seed used to initialize deterministic RNG streams.
    pub seed: u64,

    /// Number of authoritative events to generate.
    ///
    /// Delivery faults do not change this event count.
    pub event_count: u64,

    /// Initial virtual timestamp.
    pub start_timestamp_ns: u64,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            seed: DEFAULT_SEED,
            event_count: DEFAULT_EVENT_COUNT,
            start_timestamp_ns: 0,
        }
    }
}

impl RunConfig {
    /// Validates simulation run parameters.
    pub fn validate(&self) -> SimulatorResult<()> {
        if self.event_count == 0 {
            return Err(SimulatorError::InvalidConfig(
                "simulation event count must be positive",
            ));
        }

        if self.event_count == u64::MAX {
            return Err(SimulatorError::InvalidConfig(
                "event count leaves no representable next sequence",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Complete simulator configuration
// ============================================================

/// Complete simulator configuration.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SimulatorConfig {
    pub market: MarketConfig,
    pub distribution: EventDistribution,
    pub faults: FaultConfig,
    pub resources: ResourceConfig,
    pub pacing: PacingConfig,
    pub run: RunConfig,
}

impl SimulatorConfig {
    /// Validates individual components and cross-component
    /// resource constraints.
    pub fn validate(&self) -> SimulatorResult<()> {
        self.market.validate()?;
        self.distribution.validate()?;
        self.faults.validate()?;
        self.resources.validate()?;
        self.pacing.validate()?;
        self.run.validate()?;

        // One metadata record per instrument, plus all
        // potentially resting orders.
        let maximum_snapshot_entries = self
            .market
            .max_total_orders()?
            .checked_add(self.market.instrument_count)
            .ok_or(SimulatorError::InvalidConfig(
                "maximum snapshot entry count overflows",
            ))?;

        if maximum_snapshot_entries > self.resources.snapshot_capacity {
            return Err(SimulatorError::InvalidConfig(
                "snapshot capacity cannot hold the maximum market state",
            ));
        }

        // Reserve sufficient outbound capacity for a
        // complete reorder-window release and a duplicate.
        let minimum_delivery_capacity = self
            .faults
            .effective_reorder_window()
            .checked_add(if self.faults.duplicate_bps != 0 { 2 } else { 1 })
            .ok_or(SimulatorError::InvalidConfig(
                "minimum delivery capacity overflows",
            ))?;

        if self.resources.delivery_capacity < minimum_delivery_capacity {
            return Err(SimulatorError::InvalidConfig(
                "delivery capacity is insufficient for configured faults",
            ));
        }

        // Conservatively ensure the entire run can be
        // represented using monotonically increasing
        // virtual timestamps.
        let timestamp_span = self
            .run
            .event_count
            .checked_mul(self.pacing.event_interval_ns)
            .ok_or(SimulatorError::InvalidConfig(
                "simulation timestamp span overflows",
            ))?;

        self.run
            .start_timestamp_ns
            .checked_add(timestamp_span)
            .ok_or(SimulatorError::InvalidConfig(
                "final simulation timestamp overflows",
            ))?;

        Ok(())
    }

    /// Returns the maximum possible number of snapshot entries.
    pub fn max_snapshot_entries(&self) -> SimulatorResult<usize> {
        self.market
            .max_total_orders()?
            .checked_add(self.market.instrument_count)
            .ok_or(SimulatorError::InvalidConfig(
                "maximum snapshot entry count overflows",
            ))
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_is_valid() {
        assert!(SimulatorConfig::default().validate().is_ok());
    }

    #[test]
    fn default_distribution_sums_to_basis_points() {
        assert_eq!(
            EventDistribution::default().total(),
            u64::from(BASIS_POINTS),
        );
    }

    #[test]
    fn default_fault_configuration_is_disabled() {
        let config = FaultConfig::default();

        assert_eq!(config, FaultConfig::disabled());
        assert!(!config.is_enabled());
        assert_eq!(config.total_bps(), 0);
        assert_eq!(config.effective_reorder_window(), 0);
    }

    #[test]
    fn default_market_capacity_is_bounded() {
        let config = SimulatorConfig::default();

        assert_eq!(
            config.market.max_total_orders().unwrap(),
            DEFAULT_INSTRUMENT_COUNT * DEFAULT_MAX_ORDERS_PER_INSTRUMENT,
        );

        assert_eq!(
            config.max_snapshot_entries().unwrap(),
            DEFAULT_INSTRUMENT_COUNT * (DEFAULT_MAX_ORDERS_PER_INSTRUMENT + 1),
        );
    }

    #[test]
    fn zero_instruments_are_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.instrument_count = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn excessive_instrument_count_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.instrument_count = MAX_INSTRUMENT_COUNT + 1;

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_order_capacity_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.max_orders_per_instrument = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_tick_size_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.tick_size = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn unaligned_reference_price_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.reference_price += 1;

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_lot_size_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.lot_size = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn nonpositive_minimum_price_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.reference_price = 100;

        assert!(config.validate().is_err());
    }

    #[test]
    fn price_distance_overflow_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.reference_price = i64::MAX;
        config.market.tick_size = i64::MAX;

        assert!(config.validate().is_err());
    }

    #[test]
    fn quantity_overflow_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.lot_size = u64::MAX;
        config.market.max_order_quantity_lots = 2;

        assert!(config.validate().is_err());
    }

    #[test]
    fn total_order_capacity_overflow_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.market.instrument_count = 2;
        config.market.max_orders_per_instrument = usize::MAX;

        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_event_distribution_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.distribution.add += 1;

        assert!(config.validate().is_err());
    }

    #[test]
    fn distribution_without_additions_is_rejected() {
        let config = EventDistribution {
            add: 0,
            modify: BASIS_POINTS,
            cancel: 0,
            delete: 0,
            execute: 0,
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn combined_fault_probability_is_bounded() {
        let config = FaultConfig {
            drop_bps: 5_000,
            duplicate_bps: 5_000,
            reorder_bps: 1,
            reorder_window: 8,
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn full_fault_probability_is_permitted() {
        let config = FaultConfig {
            drop_bps: BASIS_POINTS,
            ..FaultConfig::default()
        };

        assert!(config.validate().is_ok());
        assert!(config.is_enabled());
    }

    #[test]
    fn enabled_reordering_uses_default_window() {
        let config = FaultConfig {
            reorder_bps: 100,
            ..FaultConfig::default()
        };

        assert_eq!(config.effective_reorder_window(), DEFAULT_REORDER_WINDOW,);
    }

    #[test]
    fn explicit_reordering_window_is_preserved() {
        let config = FaultConfig {
            reorder_bps: 100,
            reorder_window: 32,
            ..FaultConfig::default()
        };

        assert_eq!(config.effective_reorder_window(), 32);
    }

    #[test]
    fn zero_history_capacity_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.resources.history_capacity = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn insufficient_snapshot_capacity_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.resources.snapshot_capacity = config.max_snapshot_entries().unwrap() - 1;

        assert!(config.validate().is_err());
    }

    #[test]
    fn exact_snapshot_capacity_is_permitted() {
        let mut config = SimulatorConfig::default();

        config.resources.snapshot_capacity = config.max_snapshot_entries().unwrap();

        assert!(config.validate().is_ok());
    }

    #[test]
    fn duplicate_delivery_requires_two_slots() {
        let mut config = SimulatorConfig::default();

        config.faults.duplicate_bps = 100;
        config.resources.delivery_capacity = 1;

        assert!(config.validate().is_err());

        config.resources.delivery_capacity = 2;

        assert!(config.validate().is_ok());
    }

    #[test]
    fn insufficient_reorder_delivery_capacity_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.faults.reorder_bps = 100;
        config.faults.reorder_window = 32;
        config.resources.delivery_capacity = 16;

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_event_interval_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.pacing.event_interval_ns = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_event_rate_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.pacing.events_per_second = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn excessive_event_rate_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.pacing.events_per_second = NANOS_PER_SECOND + 1;

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_event_count_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.run.event_count = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn timestamp_span_overflow_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.run.event_count = u64::MAX / 2;
        config.pacing.event_interval_ns = 10;

        assert!(config.validate().is_err());
    }

    #[test]
    fn final_timestamp_overflow_is_rejected() {
        let mut config = SimulatorConfig::default();

        config.run.start_timestamp_ns = u64::MAX - 1;

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_random_seed_is_permitted() {
        let mut config = SimulatorConfig::default();

        config.run.seed = 0;

        assert!(config.validate().is_ok());
    }

    #[test]
    fn mixed_fault_configuration_is_valid() {
        let config = SimulatorConfig {
            faults: FaultConfig {
                drop_bps: 100,
                duplicate_bps: 200,
                reorder_bps: 300,
                reorder_window: 16,
            },
            ..SimulatorConfig::default()
        };

        assert!(config.validate().is_ok());
    }
}
