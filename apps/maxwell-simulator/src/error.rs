//! Unified simulator error handling.
//!
//! Errors distinguish invalid configuration, invalid market
//! operations, exhausted resources, unavailable recovery
//! history, and failures in external components.

use std::fmt;
use std::io;

use maxwell_core::{CoreError, InstrumentError, InstrumentId, OrderId, SequenceNumber};
use maxwell_protocol::ProtocolError;
use maxwell_transport::TransportError;

// ============================================================
// Result alias
// ============================================================

/// Result type used throughout the simulator.
pub type SimulatorResult<T> = Result<T, SimulatorError>;

// ============================================================
// Simulator errors
// ============================================================

/// Errors produced by the Maxwell simulator.
#[derive(Debug)]
pub enum SimulatorError {
    // --------------------------------------------------------
    // Configuration and lifecycle
    // --------------------------------------------------------
    /// Invalid simulator configuration.
    InvalidConfig(&'static str),

    /// Operation is not permitted in the current state.
    InvalidState(&'static str),

    /// Invalid argument supplied to an operation.
    InvalidArgument(&'static str),

    /// Unknown predefined simulation scenario.
    UnknownScenario(String),

    // --------------------------------------------------------
    // Resource management
    // --------------------------------------------------------
    /// Memory reservation failed.
    AllocationFailed,

    /// A bounded simulator resource has reached capacity.
    CapacityExceeded {
        resource: &'static str,
        capacity: usize,
        required: usize,
    },

    /// Outbound delivery cannot currently accept more events.
    ///
    /// The caller must retain any unconsumed input and retry
    /// after sufficient capacity becomes available.
    DeliveryBackpressure { required: usize, available: usize },

    // --------------------------------------------------------
    // Market state
    // --------------------------------------------------------
    /// Instrument is not registered in the simulated market.
    UnknownInstrument(InstrumentId),

    /// Instrument is already registered.
    DuplicateInstrument(InstrumentId),

    /// Order does not exist in the specified instrument.
    UnknownOrder {
        instrument_id: InstrumentId,
        order_id: OrderId,
    },

    /// Order identifier already exists.
    DuplicateOrder {
        instrument_id: InstrumentId,
        order_id: OrderId,
    },

    /// Market operation violates an instrument or order rule.
    InvalidMarketOperation(&'static str),

    /// No valid market event can currently be generated.
    NoEligibleOperation,

    // --------------------------------------------------------
    // Sequencing and arithmetic
    // --------------------------------------------------------
    /// No additional upstream sequence numbers are available.
    SequenceExhausted,

    /// An event does not have the expected sequence number.
    UnexpectedSequence {
        expected: SequenceNumber,
        actual: SequenceNumber,
    },

    /// No additional unique order identifiers are available.
    OrderIdExhausted,

    /// Virtual timestamp cannot be advanced safely.
    TimestampOverflow,

    /// Checked arithmetic operation failed.
    ArithmeticOverflow(&'static str),

    // --------------------------------------------------------
    // Recovery history
    // --------------------------------------------------------
    /// Requested recovery range is invalid.
    InvalidHistoryRange {
        start: SequenceNumber,
        end: SequenceNumber,
    },

    /// Requested event range is not fully retained.
    HistoryUnavailable {
        start: SequenceNumber,
        end: SequenceNumber,
    },

    /// Snapshot exceeds the configured entry capacity.
    SnapshotCapacityExceeded { limit: usize, required: usize },

    // --------------------------------------------------------
    // Internal invariants
    // --------------------------------------------------------
    /// An internal simulator invariant was violated.
    ///
    /// This represents a programming or state consistency
    /// failure rather than ordinary delivery backpressure.
    InvariantViolation(&'static str),

    // --------------------------------------------------------
    // Component errors
    // --------------------------------------------------------
    /// Maxwell core domain error.
    Core(CoreError),

    /// Instrument construction or validation error.
    Instrument(InstrumentError),

    /// Maxwell protocol encoding or decoding error.
    Protocol(ProtocolError),

    /// Maxwell transport error.
    Transport(TransportError),

    /// Standard input/output error.
    Io(io::Error),
}

// ============================================================
// Display
// ============================================================

impl fmt::Display for SimulatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => {
                write!(f, "invalid simulator configuration: {message}")
            }

            Self::InvalidState(message) => {
                write!(f, "invalid simulator state: {message}")
            }

            Self::InvalidArgument(message) => {
                write!(f, "invalid simulator argument: {message}")
            }

            Self::UnknownScenario(name) => {
                write!(f, "unknown simulation scenario: {name}")
            }

            Self::AllocationFailed => f.write_str("simulator memory allocation failed"),

            Self::CapacityExceeded {
                resource,
                capacity,
                required,
            } => {
                write!(
                    f,
                    "{resource} capacity exceeded: \
                     capacity={capacity}, required={required}"
                )
            }

            Self::DeliveryBackpressure {
                required,
                available,
            } => {
                write!(
                    f,
                    "delivery backpressure: \
                     required={required}, available={available}"
                )
            }

            Self::UnknownInstrument(instrument_id) => {
                write!(f, "unknown instrument: {instrument_id:?}")
            }

            Self::DuplicateInstrument(instrument_id) => {
                write!(f, "duplicate instrument: {instrument_id:?}")
            }

            Self::UnknownOrder {
                instrument_id,
                order_id,
            } => {
                write!(
                    f,
                    "unknown order: \
                     instrument={instrument_id:?}, \
                     order={order_id:?}"
                )
            }

            Self::DuplicateOrder {
                instrument_id,
                order_id,
            } => {
                write!(
                    f,
                    "duplicate order: \
                     instrument={instrument_id:?}, \
                     order={order_id:?}"
                )
            }

            Self::InvalidMarketOperation(message) => {
                write!(f, "invalid market operation: {message}")
            }

            Self::NoEligibleOperation => f.write_str("no eligible market operation"),

            Self::SequenceExhausted => f.write_str("upstream sequence numbers exhausted"),

            Self::UnexpectedSequence { expected, actual } => {
                write!(
                    f,
                    "unexpected sequence: \
                     expected={}, actual={}",
                    expected.raw(),
                    actual.raw(),
                )
            }

            Self::OrderIdExhausted => f.write_str("order identifiers exhausted"),

            Self::TimestampOverflow => f.write_str("simulation timestamp overflow"),

            Self::ArithmeticOverflow(operation) => {
                write!(f, "arithmetic overflow: {operation}")
            }

            Self::InvalidHistoryRange { start, end } => {
                write!(f, "invalid history range: {}..={}", start.raw(), end.raw(),)
            }

            Self::HistoryUnavailable { start, end } => {
                write!(
                    f,
                    "authoritative history unavailable: {}..={}",
                    start.raw(),
                    end.raw(),
                )
            }

            Self::SnapshotCapacityExceeded { limit, required } => {
                write!(
                    f,
                    "snapshot capacity exceeded: \
                     limit={limit}, required={required}"
                )
            }

            Self::InvariantViolation(message) => {
                write!(f, "simulator invariant violated: {message}")
            }

            Self::Core(error) => {
                write!(f, "core error: {error}")
            }

            Self::Instrument(error) => {
                write!(f, "instrument error: {error}")
            }

            Self::Protocol(error) => {
                write!(f, "protocol error: {error}")
            }

            Self::Transport(error) => {
                write!(f, "transport error: {error}")
            }

            Self::Io(error) => {
                write!(f, "I/O error: {error}")
            }
        }
    }
}

// ============================================================
// Standard error implementation
// ============================================================

impl std::error::Error for SimulatorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Core(error) => Some(error),

            Self::Instrument(error) => Some(error),

            Self::Protocol(error) => Some(error),

            Self::Transport(error) => Some(error),

            Self::Io(error) => Some(error),

            _ => None,
        }
    }
}

// ============================================================
// Error classification
// ============================================================

impl SimulatorError {
    /// Returns whether the error represents temporary
    /// outbound delivery backpressure.
    #[must_use]
    pub const fn is_backpressure(&self) -> bool {
        matches!(self, Self::DeliveryBackpressure { .. })
    }

    /// Returns whether the requested recovery history
    /// is unavailable.
    #[must_use]
    pub const fn is_history_unavailable(&self) -> bool {
        matches!(self, Self::HistoryUnavailable { .. })
    }

    /// Returns whether the error represents an internal
    /// state consistency failure.
    #[must_use]
    pub const fn is_invariant_violation(&self) -> bool {
        matches!(self, Self::InvariantViolation(_))
    }

    /// Returns whether the error represents resource
    /// capacity exhaustion.
    #[must_use]
    pub const fn is_capacity_exceeded(&self) -> bool {
        matches!(
            self,
            Self::CapacityExceeded { .. } | Self::SnapshotCapacityExceeded { .. }
        )
    }
}

// ============================================================
// Component conversions
// ============================================================

macro_rules! impl_simulator_error_from {
    ($source:ty, $variant:ident) => {
        impl From<$source> for SimulatorError {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}

impl_simulator_error_from!(CoreError, Core);
impl_simulator_error_from!(InstrumentError, Instrument);
impl_simulator_error_from!(ProtocolError, Protocol);
impl_simulator_error_from!(TransportError, Transport);
impl_simulator_error_from!(io::Error, Io);

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn invalid_configuration_has_context() {
        let error = SimulatorError::InvalidConfig("instrument count must be positive");

        assert_eq!(
            error.to_string(),
            "invalid simulator configuration: \
             instrument count must be positive",
        );
    }

    #[test]
    fn invalid_state_has_context() {
        let error = SimulatorError::InvalidState("simulation is stopped");

        assert_eq!(
            error.to_string(),
            "invalid simulator state: simulation is stopped",
        );
    }

    #[test]
    fn unknown_scenario_preserves_name() {
        let error = SimulatorError::UnknownScenario(String::from("unknown"));

        assert_eq!(error.to_string(), "unknown simulation scenario: unknown",);
    }

    #[test]
    fn capacity_error_reports_required_capacity() {
        let error = SimulatorError::CapacityExceeded {
            resource: "history",
            capacity: 100,
            required: 101,
        };

        assert_eq!(
            error.to_string(),
            "history capacity exceeded: \
             capacity=100, required=101",
        );

        assert!(error.is_capacity_exceeded());
        assert!(!error.is_backpressure());
    }

    #[test]
    fn delivery_backpressure_is_distinguishable() {
        let error = SimulatorError::DeliveryBackpressure {
            required: 8,
            available: 4,
        };

        assert_eq!(
            error.to_string(),
            "delivery backpressure: \
             required=8, available=4",
        );

        assert!(error.is_backpressure());
        assert!(!error.is_capacity_exceeded());
        assert!(!error.is_invariant_violation());
    }

    #[test]
    fn unknown_order_preserves_identifiers() {
        let error = SimulatorError::UnknownOrder {
            instrument_id: InstrumentId::new(1),
            order_id: OrderId::new(100),
        };

        match error {
            SimulatorError::UnknownOrder {
                instrument_id,
                order_id,
            } => {
                assert_eq!(instrument_id, InstrumentId::new(1),);

                assert_eq!(order_id, OrderId::new(100),);
            }

            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn unexpected_sequence_reports_both_values() {
        let error = SimulatorError::UnexpectedSequence {
            expected: SequenceNumber::new(10),
            actual: SequenceNumber::new(12),
        };

        assert_eq!(
            error.to_string(),
            "unexpected sequence: expected=10, actual=12",
        );
    }

    #[test]
    fn sequence_exhaustion_has_clear_message() {
        assert_eq!(
            SimulatorError::SequenceExhausted.to_string(),
            "upstream sequence numbers exhausted",
        );
    }

    #[test]
    fn invalid_history_range_reports_bounds() {
        let error = SimulatorError::InvalidHistoryRange {
            start: SequenceNumber::new(20),
            end: SequenceNumber::new(10),
        };

        assert_eq!(error.to_string(), "invalid history range: 20..=10",);

        assert!(!error.is_history_unavailable());
    }

    #[test]
    fn unavailable_history_is_distinguishable() {
        let error = SimulatorError::HistoryUnavailable {
            start: SequenceNumber::new(10),
            end: SequenceNumber::new(20),
        };

        assert_eq!(
            error.to_string(),
            "authoritative history unavailable: 10..=20",
        );

        assert!(error.is_history_unavailable());
        assert!(!error.is_backpressure());
    }

    #[test]
    fn snapshot_capacity_error_is_distinguishable() {
        let error = SimulatorError::SnapshotCapacityExceeded {
            limit: 100,
            required: 150,
        };

        assert_eq!(
            error.to_string(),
            "snapshot capacity exceeded: \
             limit=100, required=150",
        );

        assert!(error.is_capacity_exceeded());
    }

    #[test]
    fn invariant_violation_is_distinguishable() {
        let error = SimulatorError::InvariantViolation("authoritative market sequence mismatch");

        assert!(error.is_invariant_violation());
        assert!(!error.is_backpressure());
        assert!(!error.is_history_unavailable());
    }

    #[test]
    fn arithmetic_overflow_preserves_operation() {
        let error = SimulatorError::ArithmeticOverflow("order quantity multiplication");

        assert_eq!(
            error.to_string(),
            "arithmetic overflow: \
             order quantity multiplication",
        );
    }

    #[test]
    fn application_errors_have_no_underlying_source() {
        let errors = [
            SimulatorError::AllocationFailed,
            SimulatorError::SequenceExhausted,
            SimulatorError::OrderIdExhausted,
            SimulatorError::TimestampOverflow,
            SimulatorError::NoEligibleOperation,
            SimulatorError::InvalidConfig("invalid"),
            SimulatorError::InvalidState("invalid"),
            SimulatorError::InvariantViolation("invalid"),
        ];

        for error in errors {
            assert!(error.source().is_none());
        }
    }

    #[test]
    fn io_error_conversion_preserves_error_kind() {
        let original = io::Error::new(io::ErrorKind::BrokenPipe, "connection closed");

        let error = SimulatorError::from(original);

        match error {
            SimulatorError::Io(source) => {
                assert_eq!(source.kind(), io::ErrorKind::BrokenPipe,);
            }

            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn io_error_exposes_underlying_source() {
        let error = SimulatorError::from(io::Error::other("delivery failed"));

        let source = error.source().unwrap();

        assert_eq!(source.to_string(), "delivery failed",);
    }

    #[test]
    fn component_error_conversions_are_available() {
        fn assert_conversion<T>()
        where
            SimulatorError: From<T>,
        {
        }

        assert_conversion::<CoreError>();
        assert_conversion::<InstrumentError>();
        assert_conversion::<ProtocolError>();
        assert_conversion::<TransportError>();
        assert_conversion::<io::Error>();
    }

    #[test]
    fn simulator_error_is_send_sync_and_static() {
        fn assert_error<T>()
        where
            T: Error + Send + Sync + 'static,
        {
        }

        assert_error::<SimulatorError>();
    }

    #[test]
    fn simulator_result_supports_error_propagation() {
        fn operation() -> SimulatorResult<()> {
            Err(io::Error::other("test failure"))?;

            Ok(())
        }

        assert!(matches!(operation(), Err(SimulatorError::Io(_)),));
    }
}
