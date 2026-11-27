#![forbid(unsafe_code)]

//! Foundational domain types and primitives for Maxwell.
//!
//! This crate provides the common vocabulary shared across Maxwell's
//! market data processing, sequencing, order book reconstruction,
//! recovery, and distribution components.
//!
//! Design principles:
//!
//! - Strongly typed domain primitives.
//! - Deterministic state representation.
//! - Fixed-point numerical arithmetic.
//! - Explicit overflow and underflow handling.
//! - Minimal memory overhead.
//! - No external dependencies.
//! - No unsafe code.
//! - No networking or transport-specific logic.
//! - No dependency on external wire representations.

// ============================================================
// Module declarations
// ============================================================

pub mod error;
pub mod event;
pub mod instrument;
pub mod price;
pub mod quantity;
pub mod sequence;
pub mod timestamp;
pub mod types;

// ============================================================
// Domain identifiers
// ============================================================

pub use types::{InstrumentId, OrderId, ParticipantId, Side, StreamId, TradeId};

// ============================================================
// Numerical primitives
// ============================================================

pub use price::Price;
pub use quantity::Quantity;

// ============================================================
// Instrument primitives
// ============================================================

pub use instrument::{Instrument, InstrumentConfig, InstrumentError, MAX_DECIMAL_SCALE};

// ============================================================
// Sequence primitives
// ============================================================

pub use sequence::{SequenceNumber, SequenceRange, SequenceRangeIter, SequenceRelation};

// ============================================================
// Timestamp primitives
// ============================================================

pub use timestamp::{
    ExchangeTimestamp, MonotonicClock, MonotonicTimestamp, NANOS_PER_MICROSECOND,
    NANOS_PER_MILLISECOND, NANOS_PER_SECOND, NanoDuration, ReceiveTimestamp, TimestampError,
};

// ============================================================
// Market data events
// ============================================================

pub use event::{
    BookClearScope, BookCleared, EventEnvelope, EventError, EventHeader, EventId, EventKind,
    EventSource, FeedId, IngressMetadata, MarketEvent, OrderAdded, OrderCancelled, OrderDeleted,
    OrderExecuted, OrderModified, PriceLevelUpdated, ReceivedEvent, SessionId, Trade,
    TradingStatus, TradingStatusChanged,
};

// ============================================================
// Unified error representation
// ============================================================

pub use error::{ArithmeticError, CoreError, CoreResult, ErrorCategory, SequenceError};
