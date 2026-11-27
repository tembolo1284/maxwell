//! Named deterministic simulation scenarios.
//!
//! Scenarios provide reproducible workload configurations
//! for correctness testing, recovery testing, capacity
//! testing, and throughput benchmarking.

use std::fmt;
use std::str::FromStr;

use crate::config::{
    EventDistribution, FaultConfig, PacingMode,
    SimulatorConfig,
};
use crate::error::{SimulatorError, SimulatorResult};

// ============================================================
// Scenario classification
// ============================================================

/// Predefined simulator workload.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scenario {
    #[default]
    Clean,
    Smoke,
    HighThroughput,
    PacketLoss,
    Duplicates,
    OutOfOrder,
    MixedFaults,
    RecoveryStress,
    OrderChurn,
    CapacityStress,
}

impl Scenario {
    /// All supported predefined scenarios.
    pub const ALL: [Self; 10] = [
        Self::Clean,
        Self::Smoke,
        Self::HighThroughput,
        Self::PacketLoss,
        Self::Duplicates,
        Self::OutOfOrder,
        Self::MixedFaults,
        Self::RecoveryStress,
        Self::OrderChurn,
        Self::CapacityStress,
    ];

    /// Canonical command-line scenario name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Smoke => "smoke",
            Self::HighThroughput => "high-throughput",
            Self::PacketLoss => "packet-loss",
            Self::Duplicates => "duplicates",
            Self::OutOfOrder => "out-of-order",
            Self::MixedFaults => "mixed-faults",
            Self::RecoveryStress => "recovery-stress",
            Self::OrderChurn => "order-churn",
            Self::CapacityStress => "capacity-stress",
        }
    }

    /// Human-readable scenario description.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Clean => {
                "Baseline ordered market data workload"
            }

            Self::Smoke => {
                "Small deterministic correctness workload"
            }

            Self::HighThroughput => {
                "Large unpaced throughput workload"
            }

            Self::PacketLoss => {
                "Random event loss and gap recovery"
            }

            Self::Duplicates => {
                "Duplicate event detection and suppression"
            }

            Self::OutOfOrder => {
                "Bounded event reordering and sequencing"
            }

            Self::MixedFaults => {
                "Combined loss, duplication, and reordering"
            }

            Self::RecoveryStress => {
                "High-frequency recovery fault workload"
            }

            Self::OrderChurn => {
                "Frequent order insertion and removal"
            }

            Self::CapacityStress => {
                "Small authoritative instrument capacity"
            }
        }
    }

    /// Returns whether the predefined scenario introduces
    /// delivery faults.
    #[must_use]
    pub const fn has_faults(self) -> bool {
        matches!(
            self,
            Self::PacketLoss
                | Self::Duplicates
                | Self::OutOfOrder
                | Self::MixedFaults
                | Self::RecoveryStress
        )
    }

    // --------------------------------------------------------
    // Scenario configuration
    // --------------------------------------------------------

    /// Constructs and validates the predefined configuration.
    pub fn config(self) -> SimulatorResult<SimulatorConfig> {
        self.config_with_seed(
            SimulatorConfig::default().run.seed,
        )
    }

    /// Constructs a scenario using an explicit random seed.
    pub fn config_with_seed(
        self,
        seed: u64,
    ) -> SimulatorResult<SimulatorConfig> {
        let mut config = SimulatorConfig::default();

        config.run.seed = seed;
        config.faults = FaultConfig::disabled();

        match self {
            Self::Clean => {
                config.run.event_count = 100_000;
            }

            Self::Smoke => {
                config.run.event_count = 1_000;
                config.market.instrument_count = 2;
                config.market.max_orders_per_instrument = 64;
            }

            Self::HighThroughput => {
                config.run.event_count = 1_000_000;
                config.pacing.mode = PacingMode::Unbounded;
            }

            Self::PacketLoss => {
                config.run.event_count = 100_000;

                config.faults = FaultConfig {
                    drop_bps: 100,
                    ..FaultConfig::default()
                };
            }

            Self::Duplicates => {
                config.run.event_count = 100_000;

                config.faults = FaultConfig {
                    duplicate_bps: 500,
                    ..FaultConfig::default()
                };
            }

            Self::OutOfOrder => {
                config.run.event_count = 100_000;

                config.faults = FaultConfig {
                    reorder_bps: 1_000,
                    reorder_window: 8,
                    ..FaultConfig::default()
                };
            }

            Self::MixedFaults => {
                config.run.event_count = 100_000;

                config.faults = FaultConfig {
                    drop_bps: 100,
                    duplicate_bps: 200,
                    reorder_bps: 500,
                    reorder_window: 8,
                };
            }

            Self::RecoveryStress => {
                config.run.event_count = 250_000;

                config.faults = FaultConfig {
                    drop_bps: 500,
                    duplicate_bps: 500,
                    reorder_bps: 2_000,
                    reorder_window: 32,
                };
            }

            Self::OrderChurn => {
                config.run.event_count = 250_000;

                config.distribution = EventDistribution {
                    add: 4_500,
                    modify: 500,
                    cancel: 1_000,
                    delete: 3_000,
                    execute: 1_000,
                };
            }

            Self::CapacityStress => {
                config.run.event_count = 100_000;

                config.market.instrument_count = 2;
                config.market.max_orders_per_instrument = 8;

                config.distribution = EventDistribution {
                    add: 8_000,
                    modify: 500,
                    cancel: 500,
                    delete: 500,
                    execute: 500,
                };
            }
        }

        config.validate()?;

        Ok(config)
    }
}

// ============================================================
// Display and parsing
// ============================================================

impl fmt::Display for Scenario {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Scenario {
    type Err = SimulatorError;

    fn from_str(value: &str) -> SimulatorResult<Self> {
        match value {
            "clean" => Ok(Self::Clean),
            "smoke" => Ok(Self::Smoke),
            "high-throughput" => Ok(Self::HighThroughput),
            "packet-loss" => Ok(Self::PacketLoss),
            "duplicates" => Ok(Self::Duplicates),
            "out-of-order" => Ok(Self::OutOfOrder),
            "mixed-faults" => Ok(Self::MixedFaults),
            "recovery-stress" => Ok(Self::RecoveryStress),
            "order-churn" => Ok(Self::OrderChurn),
            "capacity-stress" => Ok(Self::CapacityStress),

            _ => Err(SimulatorError::UnknownScenario(
                value.to_owned(),
            )),
        }
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scenario_is_clean() {
        assert_eq!(Scenario::default(), Scenario::Clean);
    }

    #[test]
    fn every_scenario_has_valid_configuration() {
        for scenario in Scenario::ALL {
            let config = scenario.config().unwrap();

            config.validate().unwrap();
        }
    }

    #[test]
    fn every_scenario_round_trips_through_string() {
        for scenario in Scenario::ALL {
            let parsed = scenario
                .to_string()
                .parse::<Scenario>()
                .unwrap();

            assert_eq!(parsed, scenario);
        }
    }

    #[test]
    fn every_scenario_has_a_description() {
        for scenario in Scenario::ALL {
            assert!(!scenario.description().is_empty());
        }
    }

    #[test]
    fn unknown_scenario_is_rejected() {
        assert!(matches!(
            "invalid".parse::<Scenario>(),
            Err(SimulatorError::UnknownScenario(_)),
        ));
    }

    #[test]
    fn smoke_scenario_is_small() {
        let config = Scenario::Smoke.config().unwrap();

        assert_eq!(config.run.event_count, 1_000);
        assert_eq!(config.market.instrument_count, 2);

        assert_eq!(
            config.market.max_orders_per_instrument,
            64,
        );
    }

    #[test]
    fn clean_scenario_disables_faults() {
        let config = Scenario::Clean.config().unwrap();

        assert!(!config.faults.is_enabled());
    }

    #[test]
    fn throughput_scenario_is_unpaced() {
        let config =
            Scenario::HighThroughput.config().unwrap();

        assert_eq!(config.run.event_count, 1_000_000);

        assert_eq!(
            config.pacing.mode,
            PacingMode::Unbounded,
        );
    }

    #[test]
    fn packet_loss_scenario_only_drops_events() {
        let config = Scenario::PacketLoss.config().unwrap();

        assert_eq!(config.faults.drop_bps, 100);
        assert_eq!(config.faults.duplicate_bps, 0);
        assert_eq!(config.faults.reorder_bps, 0);
    }

    #[test]
    fn duplicate_scenario_only_duplicates_events() {
        let config = Scenario::Duplicates.config().unwrap();

        assert_eq!(config.faults.drop_bps, 0);
        assert_eq!(config.faults.duplicate_bps, 500);
        assert_eq!(config.faults.reorder_bps, 0);
    }

    #[test]
    fn reorder_scenario_has_bounded_window() {
        let config = Scenario::OutOfOrder.config().unwrap();

        assert_eq!(config.faults.reorder_bps, 1_000);
        assert_eq!(config.faults.reorder_window, 8);
    }

    #[test]
    fn mixed_scenario_enables_all_fault_types() {
        let config = Scenario::MixedFaults.config().unwrap();

        assert!(config.faults.drop_bps > 0);
        assert!(config.faults.duplicate_bps > 0);
        assert!(config.faults.reorder_bps > 0);
    }

    #[test]
    fn fault_classification_matches_configuration() {
        for scenario in Scenario::ALL {
            let config = scenario.config().unwrap();

            assert_eq!(
                scenario.has_faults(),
                config.faults.is_enabled(),
            );
        }
    }

    #[test]
    fn custom_seed_is_preserved() {
        let config = Scenario::MixedFaults
            .config_with_seed(123_456)
            .unwrap();

        assert_eq!(config.run.seed, 123_456);
    }

    #[test]
    fn capacity_stress_has_small_order_capacity() {
        let config =
            Scenario::CapacityStress.config().unwrap();

        assert_eq!(config.market.instrument_count, 2);

        assert_eq!(
            config.market.max_orders_per_instrument,
            8,
        );

        assert_eq!(config.distribution.add, 8_000);
    }
}
