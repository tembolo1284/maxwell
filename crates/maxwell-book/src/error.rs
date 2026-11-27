//! Strongly typed order book errors.

use core::fmt;

use maxwell_core::{InstrumentError, InstrumentId, OrderId, Price, SequenceNumber};

pub type BookResult<T> = Result<T, BookError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookError {
    InvalidPriceLevelCapacity(usize),
    InvalidOrderCapacity(usize),
    InvalidInstrumentCapacity(usize),

    AllocationFailed,
    PriceLevelCapacityExceeded,
    OrderCapacityExceeded,
    InstrumentCapacityExceeded,

    UnknownInstrument(InstrumentId),
    InstrumentAlreadyRegistered(InstrumentId),

    InstrumentMismatch {
        expected: InstrumentId,
        actual: InstrumentId,
    },

    UnknownOrder(OrderId),
    DuplicateOrder(OrderId),

    InvalidPrice(Price),
    ZeroQuantity,
    QuantityOverflow,
    QuantityUnderflow,
    OrderCountOverflow,
    OrderCountUnderflow,

    InvalidPriceLevel,
    InvalidReduction,
    InvalidReplacement,
    InvalidSnapshot,

    NonIncreasingSequence {
        previous: SequenceNumber,
        received: SequenceNumber,
    },

    RecoveryRequired,
    InconsistentBook,
    UnsupportedEvent,

    Instrument(InstrumentError),
}

impl fmt::Display for BookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPriceLevelCapacity(value) => {
                write!(f, "invalid price level capacity: {value}")
            }

            Self::InvalidOrderCapacity(value) => {
                write!(f, "invalid order capacity: {value}")
            }

            Self::InvalidInstrumentCapacity(value) => {
                write!(f, "invalid instrument capacity: {value}")
            }

            Self::AllocationFailed => f.write_str("order book allocation failed"),

            Self::PriceLevelCapacityExceeded => f.write_str("price level capacity exceeded"),

            Self::OrderCapacityExceeded => f.write_str("order capacity exceeded"),

            Self::InstrumentCapacityExceeded => f.write_str("instrument capacity exceeded"),

            Self::UnknownInstrument(id) => {
                write!(f, "unknown instrument: {id}")
            }

            Self::InstrumentAlreadyRegistered(id) => {
                write!(f, "instrument already registered: {id}")
            }

            Self::InstrumentMismatch { expected, actual } => {
                write!(
                    f,
                    "instrument mismatch: expected {expected}, \
                     received {actual}"
                )
            }

            Self::UnknownOrder(id) => {
                write!(f, "unknown order: {id}")
            }

            Self::DuplicateOrder(id) => {
                write!(f, "duplicate order: {id}")
            }

            Self::InvalidPrice(price) => {
                write!(f, "invalid price: {price}")
            }

            Self::ZeroQuantity => f.write_str("quantity must be nonzero"),

            Self::QuantityOverflow => f.write_str("quantity overflow"),

            Self::QuantityUnderflow => f.write_str("quantity underflow"),

            Self::OrderCountOverflow => f.write_str("order count overflow"),

            Self::OrderCountUnderflow => f.write_str("order count underflow"),

            Self::InvalidPriceLevel => f.write_str("invalid price level"),

            Self::InvalidReduction => f.write_str("invalid order reduction"),

            Self::InvalidReplacement => f.write_str("invalid order replacement"),

            Self::InvalidSnapshot => f.write_str("invalid book snapshot"),

            Self::NonIncreasingSequence { previous, received } => {
                write!(
                    f,
                    "non-increasing sequence: previous {previous}, \
                     received {received}"
                )
            }

            Self::RecoveryRequired => f.write_str("order book recovery required"),

            Self::InconsistentBook => f.write_str("inconsistent order book state"),

            Self::UnsupportedEvent => f.write_str("unsupported market event"),

            Self::Instrument(error) => {
                write!(f, "instrument validation failed: {error}")
            }
        }
    }
}

impl std::error::Error for BookError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Instrument(error) => Some(error),
            _ => None,
        }
    }
}

impl From<InstrumentError> for BookError {
    #[inline]
    fn from(error: InstrumentError) -> Self {
        Self::Instrument(error)
    }
}

const _: () = {
    assert!(!core::mem::needs_drop::<BookError>());
};
