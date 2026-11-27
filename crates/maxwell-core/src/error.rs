//! Unified domain error representation for Maxwell.
//!
//! Aggregates foundational errors from the instrument,
//! timestamp, and market event modules.
//!
//! Design guarantees:
//!
//! - Strongly typed error variants.
//! - No heap allocations during error construction.
//! - No dynamic dispatch during error propagation.
//! - Preservation of underlying error information.
//! - Standard library error interoperability.
//! - No external dependencies.
//!
//! Component-specific errors belong to their respective
//! crates, including maxwell-protocol, maxwell-sequencer,
//! maxwell-book, and maxwell-transport.

use core::fmt;

use crate::{EventError, InstrumentError, TimestampError};

// ============================================================
// Core result
// ============================================================

/// Result type for operations returning a unified core error.
pub type CoreResult<T> = Result<T, CoreError>;

// ============================================================
// Arithmetic errors
// ============================================================

/// Errors associated with checked numerical operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticError {
    Overflow,
    Underflow,
    DivisionByZero,
    InvalidTickSize,
    InvalidLotSize,
    IncompatibleScale,
    PrecisionLoss,
}

impl fmt::Display for ArithmeticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Overflow => "arithmetic overflow",
            Self::Underflow => "arithmetic underflow",
            Self::DivisionByZero => "division by zero",
            Self::InvalidTickSize => "tick size must be positive",
            Self::InvalidLotSize => "lot size must be positive",
            Self::IncompatibleScale => "incompatible numerical scales",
            Self::PrecisionLoss => "operation would lose numerical precision",
        };

        f.write_str(message)
    }
}

impl std::error::Error for ArithmeticError {}

// ============================================================
// Sequence errors
// ============================================================

/// Errors associated with sequence number operations.
///
/// Stateful sequencing failures and recovery policies belong
/// to maxwell-sequencer rather than maxwell-core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceError {
    Overflow,
    Underflow,
    Exhausted,
    InvalidRange,
    EmptyRange,
    IncompatibleStream,
    IncompatibleSession,
}

impl fmt::Display for SequenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Overflow => "sequence number overflow",
            Self::Underflow => "sequence number underflow",
            Self::Exhausted => "sequence number space exhausted",
            Self::InvalidRange => "invalid sequence range",
            Self::EmptyRange => "sequence range must not be empty",
            Self::IncompatibleStream => "sequence numbers belong to different streams",
            Self::IncompatibleSession => "sequence numbers belong to different sessions",
        };

        f.write_str(message)
    }
}

impl std::error::Error for SequenceError {}

// ============================================================
// Error category
// ============================================================

/// High-level classification of foundational errors.
///
/// Categories are intended for diagnostics, metrics, and
/// error reporting rather than recovery policy decisions.
///
/// Discriminants are internal values and must not be used
/// directly as external wire protocol values.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    Arithmetic = 1,
    Instrument = 2,
    Sequence = 3,
    Timestamp = 4,
    Event = 5,
}

impl ErrorCategory {
    /// Returns the stable textual category name.
    #[must_use]
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Arithmetic => "arithmetic",
            Self::Instrument => "instrument",
            Self::Sequence => "sequence",
            Self::Timestamp => "timestamp",
            Self::Event => "event",
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================
// Unified core error
// ============================================================

/// Unified error representation for foundational operations.
///
/// This enum preserves the original typed error without
/// requiring heap allocation, boxing, or string construction.
///
/// Individual modules should continue returning their own
/// specific error types where appropriate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreError {
    Arithmetic(ArithmeticError),
    Instrument(InstrumentError),
    Sequence(SequenceError),
    Timestamp(TimestampError),
    Event(EventError),
}

impl CoreError {
    // ========================================================
    // Classification
    // ========================================================

    /// Returns the high-level error category.
    #[must_use]
    #[inline]
    pub const fn category(self) -> ErrorCategory {
        match self {
            Self::Arithmetic(_) => ErrorCategory::Arithmetic,
            Self::Instrument(_) => ErrorCategory::Instrument,
            Self::Sequence(_) => ErrorCategory::Sequence,
            Self::Timestamp(_) => ErrorCategory::Timestamp,
            Self::Event(_) => ErrorCategory::Event,
        }
    }

    /// Returns whether this is an arithmetic error.
    #[must_use]
    #[inline]
    pub const fn is_arithmetic(self) -> bool {
        matches!(self, Self::Arithmetic(_))
    }

    /// Returns whether this is an instrument error.
    #[must_use]
    #[inline]
    pub const fn is_instrument(self) -> bool {
        matches!(self, Self::Instrument(_))
    }

    /// Returns whether this is a sequence error.
    #[must_use]
    #[inline]
    pub const fn is_sequence(self) -> bool {
        matches!(self, Self::Sequence(_))
    }

    /// Returns whether this is a timestamp error.
    #[must_use]
    #[inline]
    pub const fn is_timestamp(self) -> bool {
        matches!(self, Self::Timestamp(_))
    }

    /// Returns whether this is a market event error.
    #[must_use]
    #[inline]
    pub const fn is_event(self) -> bool {
        matches!(self, Self::Event(_))
    }

    // ========================================================
    // Typed error extraction
    // ========================================================

    /// Returns the underlying arithmetic error, if present.
    #[must_use]
    #[inline]
    pub const fn as_arithmetic(self) -> Option<ArithmeticError> {
        match self {
            Self::Arithmetic(error) => Some(error),
            _ => None,
        }
    }

    /// Returns the underlying instrument error, if present.
    #[must_use]
    #[inline]
    pub const fn as_instrument(self) -> Option<InstrumentError> {
        match self {
            Self::Instrument(error) => Some(error),
            _ => None,
        }
    }

    /// Returns the underlying sequence error, if present.
    #[must_use]
    #[inline]
    pub const fn as_sequence(self) -> Option<SequenceError> {
        match self {
            Self::Sequence(error) => Some(error),
            _ => None,
        }
    }

    /// Returns the underlying timestamp error, if present.
    #[must_use]
    #[inline]
    pub const fn as_timestamp(self) -> Option<TimestampError> {
        match self {
            Self::Timestamp(error) => Some(error),
            _ => None,
        }
    }

    /// Returns the underlying event error, if present.
    #[must_use]
    #[inline]
    pub const fn as_event(self) -> Option<EventError> {
        match self {
            Self::Event(error) => Some(error),
            _ => None,
        }
    }
}

// ============================================================
// Error formatting
// ============================================================

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arithmetic(error) => {
                write!(f, "arithmetic error: {error}")
            }
            Self::Instrument(error) => {
                write!(f, "instrument error: {error}")
            }
            Self::Sequence(error) => {
                write!(f, "sequence error: {error}")
            }
            Self::Timestamp(error) => {
                write!(f, "timestamp error: {error}")
            }
            Self::Event(error) => {
                write!(f, "event error: {error}")
            }
        }
    }
}

// ============================================================
// Standard library error interoperability
// ============================================================

impl std::error::Error for CoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Arithmetic(error) => Some(error),
            Self::Instrument(error) => Some(error),
            Self::Sequence(error) => Some(error),
            Self::Timestamp(error) => Some(error),
            Self::Event(error) => Some(error),
        }
    }
}

// ============================================================
// Error conversions
// ============================================================

/// Implements conversion from a specific foundational error
/// into the unified `CoreError` representation.
///
/// These conversions enable automatic error propagation
/// through Rust's question mark operator.
macro_rules! impl_error_conversion {
    ($error:ty, $variant:ident) => {
        impl From<$error> for CoreError {
            #[inline]
            fn from(error: $error) -> Self {
                Self::$variant(error)
            }
        }
    };
}

impl_error_conversion!(ArithmeticError, Arithmetic);
impl_error_conversion!(InstrumentError, Instrument);
impl_error_conversion!(SequenceError, Sequence);
impl_error_conversion!(TimestampError, Timestamp);
impl_error_conversion!(EventError, Event);

// ============================================================
// Compile-time layout guarantees
// ============================================================

const _: () = {
    assert!(size_of::<ErrorCategory>() == size_of::<u8>());

    assert!(!core::mem::needs_drop::<ArithmeticError>());
    assert!(!core::mem::needs_drop::<SequenceError>());
    assert!(!core::mem::needs_drop::<CoreError>());
};

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    use crate::{
        Instrument, InstrumentConfig, InstrumentId, OrderAdded, OrderId, Price, Quantity,
        SequenceNumber, SequenceRange, Side,
    };

    // ========================================================
    // Arithmetic error tests
    // ========================================================

    #[test]
    fn arithmetic_error_display() {
        let cases = [
            (ArithmeticError::Overflow, "arithmetic overflow"),
            (ArithmeticError::Underflow, "arithmetic underflow"),
            (ArithmeticError::DivisionByZero, "division by zero"),
            (
                ArithmeticError::InvalidTickSize,
                "tick size must be positive",
            ),
            (ArithmeticError::InvalidLotSize, "lot size must be positive"),
            (
                ArithmeticError::IncompatibleScale,
                "incompatible numerical scales",
            ),
            (
                ArithmeticError::PrecisionLoss,
                "operation would lose numerical precision",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn arithmetic_error_has_no_source() {
        assert!(ArithmeticError::Overflow.source().is_none());
    }

    // ========================================================
    // Sequence error tests
    // ========================================================

    #[test]
    fn sequence_error_display() {
        let cases = [
            (SequenceError::Overflow, "sequence number overflow"),
            (SequenceError::Underflow, "sequence number underflow"),
            (SequenceError::Exhausted, "sequence number space exhausted"),
            (SequenceError::InvalidRange, "invalid sequence range"),
            (
                SequenceError::EmptyRange,
                "sequence range must not be empty",
            ),
            (
                SequenceError::IncompatibleStream,
                "sequence numbers belong to different streams",
            ),
            (
                SequenceError::IncompatibleSession,
                "sequence numbers belong to different sessions",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn sequence_error_has_no_source() {
        assert!(SequenceError::Exhausted.source().is_none());
    }

    // ========================================================
    // Error category tests
    // ========================================================

    #[test]
    fn error_category_names() {
        let cases = [
            (ErrorCategory::Arithmetic, "arithmetic"),
            (ErrorCategory::Instrument, "instrument"),
            (ErrorCategory::Sequence, "sequence"),
            (ErrorCategory::Timestamp, "timestamp"),
            (ErrorCategory::Event, "event"),
        ];

        for (category, expected) in cases {
            assert_eq!(category.as_str(), expected);
            assert_eq!(category.to_string(), expected);
        }
    }

    #[test]
    fn core_error_categories() {
        let cases = [
            (
                CoreError::Arithmetic(ArithmeticError::Overflow),
                ErrorCategory::Arithmetic,
            ),
            (
                CoreError::Instrument(InstrumentError::InvalidTickSize),
                ErrorCategory::Instrument,
            ),
            (
                CoreError::Sequence(SequenceError::Exhausted),
                ErrorCategory::Sequence,
            ),
            (
                CoreError::Timestamp(TimestampError::Overflow),
                ErrorCategory::Timestamp,
            ),
            (
                CoreError::Event(EventError::ZeroOrderQuantity),
                ErrorCategory::Event,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.category(), expected);
        }
    }

    // ========================================================
    // Conversion tests
    // ========================================================

    #[test]
    fn arithmetic_error_conversion() {
        let error: CoreError = ArithmeticError::Overflow.into();

        assert_eq!(error, CoreError::Arithmetic(ArithmeticError::Overflow),);
    }

    #[test]
    fn instrument_error_conversion() {
        let error: CoreError = InstrumentError::InvalidTickSize.into();

        assert_eq!(
            error,
            CoreError::Instrument(InstrumentError::InvalidTickSize,),
        );
    }

    #[test]
    fn sequence_error_conversion() {
        let error: CoreError = SequenceError::Exhausted.into();

        assert_eq!(error, CoreError::Sequence(SequenceError::Exhausted),);
    }

    #[test]
    fn timestamp_error_conversion() {
        let error: CoreError = TimestampError::Overflow.into();

        assert_eq!(error, CoreError::Timestamp(TimestampError::Overflow),);
    }

    #[test]
    fn event_error_conversion() {
        let error: CoreError = EventError::ZeroOrderQuantity.into();

        assert_eq!(error, CoreError::Event(EventError::ZeroOrderQuantity),);
    }

    // ========================================================
    // Error classification tests
    // ========================================================

    #[test]
    fn arithmetic_classification() {
        let error = CoreError::Arithmetic(ArithmeticError::Overflow);

        assert!(error.is_arithmetic());
        assert!(!error.is_instrument());
        assert!(!error.is_sequence());
        assert!(!error.is_timestamp());
        assert!(!error.is_event());
    }

    #[test]
    fn instrument_classification() {
        let error = CoreError::Instrument(InstrumentError::InvalidTickSize);

        assert!(!error.is_arithmetic());
        assert!(error.is_instrument());
        assert!(!error.is_sequence());
        assert!(!error.is_timestamp());
        assert!(!error.is_event());
    }

    #[test]
    fn sequence_classification() {
        let error = CoreError::Sequence(SequenceError::Exhausted);

        assert!(!error.is_arithmetic());
        assert!(!error.is_instrument());
        assert!(error.is_sequence());
        assert!(!error.is_timestamp());
        assert!(!error.is_event());
    }

    #[test]
    fn timestamp_classification() {
        let error = CoreError::Timestamp(TimestampError::Overflow);

        assert!(!error.is_arithmetic());
        assert!(!error.is_instrument());
        assert!(!error.is_sequence());
        assert!(error.is_timestamp());
        assert!(!error.is_event());
    }

    #[test]
    fn event_classification() {
        let error = CoreError::Event(EventError::ZeroTradeQuantity);

        assert!(!error.is_arithmetic());
        assert!(!error.is_instrument());
        assert!(!error.is_sequence());
        assert!(!error.is_timestamp());
        assert!(error.is_event());
    }

    // ========================================================
    // Typed extraction tests
    // ========================================================

    #[test]
    fn extract_arithmetic_error() {
        let error = CoreError::Arithmetic(ArithmeticError::Underflow);

        assert_eq!(error.as_arithmetic(), Some(ArithmeticError::Underflow),);

        assert_eq!(error.as_instrument(), None);
        assert_eq!(error.as_sequence(), None);
        assert_eq!(error.as_timestamp(), None);
        assert_eq!(error.as_event(), None);
    }

    #[test]
    fn extract_instrument_error() {
        let error = CoreError::Instrument(InstrumentError::InvalidLotSize);

        assert_eq!(error.as_instrument(), Some(InstrumentError::InvalidLotSize),);

        assert_eq!(error.as_arithmetic(), None);
    }

    #[test]
    fn extract_sequence_error() {
        let error = CoreError::Sequence(SequenceError::InvalidRange);

        assert_eq!(error.as_sequence(), Some(SequenceError::InvalidRange),);

        assert_eq!(error.as_event(), None);
    }

    #[test]
    fn extract_timestamp_error() {
        let error = CoreError::Timestamp(TimestampError::BeforeUnixEpoch);

        assert_eq!(error.as_timestamp(), Some(TimestampError::BeforeUnixEpoch),);

        assert_eq!(error.as_sequence(), None);
    }

    #[test]
    fn extract_event_error() {
        let error = CoreError::Event(EventError::ZeroExecutionQuantity);

        assert_eq!(error.as_event(), Some(EventError::ZeroExecutionQuantity),);

        assert_eq!(error.as_timestamp(), None);
    }

    // ========================================================
    // Formatting tests
    // ========================================================

    #[test]
    fn core_error_display() {
        let cases = [
            (
                CoreError::Arithmetic(ArithmeticError::Overflow),
                "arithmetic error: arithmetic overflow",
            ),
            (
                CoreError::Instrument(InstrumentError::InvalidTickSize),
                "instrument error: tick size must be positive",
            ),
            (
                CoreError::Sequence(SequenceError::Exhausted),
                "sequence error: sequence number space exhausted",
            ),
            (
                CoreError::Timestamp(TimestampError::Overflow),
                "timestamp error: timestamp exceeds representable range",
            ),
            (
                CoreError::Event(EventError::ZeroTradeQuantity),
                "event error: trade quantity must be positive",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    // ========================================================
    // Error source tests
    // ========================================================

    #[test]
    fn arithmetic_error_source() {
        let error = CoreError::Arithmetic(ArithmeticError::Overflow);

        let source = error.source().unwrap();

        assert_eq!(
            source.downcast_ref::<ArithmeticError>(),
            Some(&ArithmeticError::Overflow),
        );
    }

    #[test]
    fn instrument_error_source() {
        let error = CoreError::Instrument(InstrumentError::InvalidTickSize);

        let source = error.source().unwrap();

        assert_eq!(
            source.downcast_ref::<InstrumentError>(),
            Some(&InstrumentError::InvalidTickSize),
        );
    }

    #[test]
    fn sequence_error_source() {
        let error = CoreError::Sequence(SequenceError::Exhausted);

        let source = error.source().unwrap();

        assert_eq!(
            source.downcast_ref::<SequenceError>(),
            Some(&SequenceError::Exhausted),
        );
    }

    #[test]
    fn timestamp_error_source() {
        let error = CoreError::Timestamp(TimestampError::Overflow);

        let source = error.source().unwrap();

        assert_eq!(
            source.downcast_ref::<TimestampError>(),
            Some(&TimestampError::Overflow),
        );
    }

    #[test]
    fn event_error_source() {
        let error = CoreError::Event(EventError::ZeroOrderQuantity);

        let source = error.source().unwrap();

        assert_eq!(
            source.downcast_ref::<EventError>(),
            Some(&EventError::ZeroOrderQuantity),
        );
    }

    // ========================================================
    // Error propagation tests
    // ========================================================

    fn checked_price_add(lhs: Price, rhs: Price) -> CoreResult<Price> {
        let result = lhs.checked_add(rhs).ok_or(ArithmeticError::Overflow)?;

        Ok(result)
    }

    fn checked_quantity_sub(lhs: Quantity, rhs: Quantity) -> CoreResult<Quantity> {
        let result = lhs.checked_sub(rhs).ok_or(ArithmeticError::Underflow)?;

        Ok(result)
    }

    fn advance_sequence(sequence: SequenceNumber) -> CoreResult<SequenceNumber> {
        let next = sequence.next().ok_or(SequenceError::Exhausted)?;

        Ok(next)
    }

    fn create_sequence_range(
        start: SequenceNumber,
        end: SequenceNumber,
    ) -> CoreResult<SequenceRange> {
        let range = SequenceRange::new(start, end).ok_or(SequenceError::InvalidRange)?;

        Ok(range)
    }

    fn validate_order(event: OrderAdded) -> CoreResult<()> {
        event.validate()?;

        Ok(())
    }

    fn create_instrument(config: InstrumentConfig) -> CoreResult<Instrument> {
        let instrument = Instrument::new(config)?;

        Ok(instrument)
    }

    #[test]
    fn successful_arithmetic_propagation() {
        assert_eq!(
            checked_price_add(Price::new(100), Price::new(50),),
            Ok(Price::new(150)),
        );
    }

    #[test]
    fn arithmetic_overflow_propagation() {
        assert_eq!(
            checked_price_add(Price::MAX, Price::new(1),),
            Err(CoreError::Arithmetic(ArithmeticError::Overflow,)),
        );
    }

    #[test]
    fn arithmetic_underflow_propagation() {
        assert_eq!(
            checked_quantity_sub(Quantity::new(100), Quantity::new(150),),
            Err(CoreError::Arithmetic(ArithmeticError::Underflow,)),
        );
    }

    #[test]
    fn successful_sequence_propagation() {
        assert_eq!(
            advance_sequence(SequenceNumber::new(100)),
            Ok(SequenceNumber::new(101)),
        );
    }

    #[test]
    fn sequence_exhaustion_propagation() {
        assert_eq!(
            advance_sequence(SequenceNumber::MAX),
            Err(CoreError::Sequence(SequenceError::Exhausted,)),
        );
    }

    #[test]
    fn invalid_sequence_range_propagation() {
        assert_eq!(
            create_sequence_range(SequenceNumber::new(200), SequenceNumber::new(100),),
            Err(CoreError::Sequence(SequenceError::InvalidRange,)),
        );
    }

    #[test]
    fn successful_event_propagation() {
        let event = OrderAdded {
            order_id: OrderId::new(1),
            price: Price::new(100),
            quantity: Quantity::new(50),
            side: Side::Buy,
        };

        assert_eq!(validate_order(event), Ok(()));
    }

    #[test]
    fn event_error_propagation() {
        let event = OrderAdded {
            order_id: OrderId::new(1),
            price: Price::new(100),
            quantity: Quantity::ZERO,
            side: Side::Buy,
        };

        assert_eq!(
            validate_order(event),
            Err(CoreError::Event(EventError::ZeroOrderQuantity,)),
        );
    }

    #[test]
    fn instrument_error_propagation() {
        let config = InstrumentConfig {
            id: InstrumentId::new(1),
            price_scale: 4,
            quantity_scale: 0,
            tick_size: Price::ZERO,
            lot_size: Quantity::ONE,
            min_price: Price::new(-100_000),
            max_price: Price::new(100_000),
            min_quantity: Quantity::ONE,
            max_quantity: Quantity::new(1_000_000),
        };

        assert_eq!(
            create_instrument(config),
            Err(CoreError::Instrument(InstrumentError::InvalidTickSize,)),
        );
    }

    // ========================================================
    // Miscellaneous tests
    // ========================================================

    #[test]
    fn error_equality() {
        let first = CoreError::Arithmetic(ArithmeticError::Overflow);

        let second = CoreError::Arithmetic(ArithmeticError::Overflow);

        let third = CoreError::Arithmetic(ArithmeticError::Underflow);

        assert_eq!(first, second);
        assert_ne!(first, third);
    }

    #[test]
    fn error_is_copyable() {
        let original = CoreError::Sequence(SequenceError::Exhausted);

        let copied = original;

        assert_eq!(original, copied);
    }

    #[test]
    fn error_requires_no_destructor() {
        assert!(!core::mem::needs_drop::<CoreError>());
    }

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<ArithmeticError>();
        assert_send_sync::<SequenceError>();
        assert_send_sync::<CoreError>();
    }

    #[test]
    fn const_error_classification() {
        const ERROR: CoreError = CoreError::Arithmetic(ArithmeticError::Overflow);

        const CATEGORY: ErrorCategory = ERROR.category();

        const INNER: Option<ArithmeticError> = ERROR.as_arithmetic();

        assert_eq!(CATEGORY, ErrorCategory::Arithmetic);
        assert_eq!(INNER, Some(ArithmeticError::Overflow));
    }
}
