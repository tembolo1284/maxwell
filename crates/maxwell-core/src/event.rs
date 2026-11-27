//! Canonical market data events for Maxwell.
//!
//! Defines the internal event representation shared by
//! protocol decoders, sequencers, order book reconstruction,
//! recovery components, publishers, and replay applications.
//!
//! These structures are domain representations rather than
//! external wire protocol structures.
//!
//! Event semantics:
//!
//! - Order additions introduce resting liquidity.
//! - Order modifications replace resting price and quantity.
//! - Order cancellations remove a specified quantity.
//! - Order deletions remove all remaining liquidity.
//! - Order executions reduce resting liquidity.
//! - Trade events report transactions without independently
//!   modifying the order book.
//! - Price-level updates represent absolute aggregate depth.
//!
//! L2 and L3 updates must not be applied independently to
//! the same book unless explicitly coordinated.
//!
//! Stateful validation belongs to maxwell-book and
//! maxwell-sequencer.

use core::fmt;

use crate::{
    ExchangeTimestamp, InstrumentId, MonotonicTimestamp, OrderId, Price, Quantity,
    ReceiveTimestamp, SequenceNumber, Side, StreamId, TradeId,
};

// ============================================================
// Session identifier
// ============================================================

/// Identifies an exchange-defined market data session.
///
/// Sequence numbers may restart between sessions.
/// Session identifiers must distinguish separate sequence
/// epochs within the same stream.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(u64);

impl SessionId {
    #[must_use]
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl From<SessionId> for u64 {
    #[inline]
    fn from(value: SessionId) -> Self {
        value.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

// ============================================================
// Feed identity
// ============================================================

/// Identifies a logical market data stream and session.
///
/// Sequence numbers are meaningful only within this scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeedId {
    pub stream_id: StreamId,
    pub session_id: SessionId,
}

impl FeedId {
    #[must_use]
    #[inline]
    pub const fn new(stream_id: StreamId, session_id: SessionId) -> Self {
        Self {
            stream_id,
            session_id,
        }
    }
}

impl fmt::Display for FeedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.stream_id, self.session_id)
    }
}

// ============================================================
// Event identity
// ============================================================

/// Identifies a canonical sequenced market data event.
///
/// A sequence number identifies one event within a feed.
///
/// Protocols that sequence packets rather than individual
/// events must normalize their ordering semantics before
/// constructing this identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventId {
    pub feed: FeedId,
    pub sequence: SequenceNumber,
}

impl EventId {
    #[must_use]
    #[inline]
    pub const fn new(feed: FeedId, sequence: SequenceNumber) -> Self {
        Self { feed, sequence }
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.feed, self.sequence)
    }
}

// ============================================================
// Event errors
// ============================================================

/// Errors associated with stateless market event validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventError {
    ZeroOrderQuantity,
    ZeroCancellationQuantity,
    ZeroExecutionQuantity,
    ZeroTradeQuantity,
    InvalidLevelOrderCount,
}

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ZeroOrderQuantity => "resting order quantity must be positive",
            Self::ZeroCancellationQuantity => "cancellation quantity must be positive",
            Self::ZeroExecutionQuantity => "execution quantity must be positive",
            Self::ZeroTradeQuantity => "trade quantity must be positive",
            Self::InvalidLevelOrderCount => "order count is inconsistent with level quantity",
        };

        f.write_str(message)
    }
}

impl std::error::Error for EventError {}

// ============================================================
// Event kind
// ============================================================

/// Identifies the semantic category of a market data event.
///
/// Discriminants are internal values and must not be used
/// directly as wire protocol message identifiers.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    OrderAdded = 1,
    OrderModified = 2,
    OrderCancelled = 3,
    OrderDeleted = 4,
    OrderExecuted = 5,
    Trade = 6,
    PriceLevelUpdated = 7,
    BookCleared = 8,
    TradingStatusChanged = 9,
}

impl EventKind {
    #[must_use]
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OrderAdded => "ORDER_ADDED",
            Self::OrderModified => "ORDER_MODIFIED",
            Self::OrderCancelled => "ORDER_CANCELLED",
            Self::OrderDeleted => "ORDER_DELETED",
            Self::OrderExecuted => "ORDER_EXECUTED",
            Self::Trade => "TRADE",
            Self::PriceLevelUpdated => "PRICE_LEVEL_UPDATED",
            Self::BookCleared => "BOOK_CLEARED",
            Self::TradingStatusChanged => "TRADING_STATUS_CHANGED",
        }
    }

    /// Returns whether this event directly changes book state.
    #[must_use]
    #[inline]
    pub const fn affects_book(self) -> bool {
        matches!(
            self,
            Self::OrderAdded
                | Self::OrderModified
                | Self::OrderCancelled
                | Self::OrderDeleted
                | Self::OrderExecuted
                | Self::PriceLevelUpdated
                | Self::BookCleared
        )
    }

    /// Returns whether this is an order-level event.
    #[must_use]
    #[inline]
    pub const fn is_order_event(self) -> bool {
        matches!(
            self,
            Self::OrderAdded
                | Self::OrderModified
                | Self::OrderCancelled
                | Self::OrderDeleted
                | Self::OrderExecuted
        )
    }

    /// Returns whether this event reports a transaction.
    #[must_use]
    #[inline]
    pub const fn is_trade_event(self) -> bool {
        matches!(self, Self::Trade | Self::OrderExecuted)
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================
// Trading status
// ============================================================

/// Trading status of an individual financial instrument.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TradingStatus {
    Unknown = 0,
    PreOpen = 1,
    Open = 2,
    Halted = 3,
    Auction = 4,
    Closed = 5,
}

impl TradingStatus {
    #[must_use]
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::PreOpen => "PRE_OPEN",
            Self::Open => "OPEN",
            Self::Halted => "HALTED",
            Self::Auction => "AUCTION",
            Self::Closed => "CLOSED",
        }
    }

    /// Returns whether the instrument is in its continuous
    /// trading state.
    #[must_use]
    #[inline]
    pub const fn is_continuous_trading(self) -> bool {
        matches!(self, Self::Open)
    }
}

impl fmt::Display for TradingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================
// Book clear scope
// ============================================================

/// Determines which portion of an instrument book is cleared.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookClearScope {
    BothSides = 0,
    BuySide = 1,
    SellSide = 2,
}

impl BookClearScope {
    #[must_use]
    #[inline]
    pub const fn includes(self, side: Side) -> bool {
        match self {
            Self::BothSides => true,
            Self::BuySide => side.is_buy(),
            Self::SellSide => side.is_sell(),
        }
    }
}

// ============================================================
// Order added
// ============================================================

/// Introduces a new resting order into the order book.
///
/// Quantity represents the initial visible resting quantity,
/// not necessarily the original submitted order quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderAdded {
    pub order_id: OrderId,
    pub price: Price,
    pub quantity: Quantity,
    pub side: Side,
}

impl OrderAdded {
    /// Validates stateless event invariants.
    ///
    /// # Errors
    ///
    /// Returns an error if the resting quantity is zero.
    #[inline]
    pub const fn validate(self) -> Result<(), EventError> {
        if self.quantity.is_zero() {
            return Err(EventError::ZeroOrderQuantity);
        }

        Ok(())
    }
}

// ============================================================
// Order modified
// ============================================================

/// Replaces the price and remaining quantity of a resting order.
///
/// The new quantity is absolute, not a signed quantity delta.
///
/// The order identifier and side remain unchanged.
/// Queue-priority semantics depend on exchange rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderModified {
    pub order_id: OrderId,
    pub new_price: Price,
    pub new_quantity: Quantity,
}

impl OrderModified {
    /// Validates stateless event invariants.
    ///
    /// # Errors
    ///
    /// Returns an error if the replacement quantity is zero.
    /// Full removal must use an appropriate removal event.
    #[inline]
    pub const fn validate(self) -> Result<(), EventError> {
        if self.new_quantity.is_zero() {
            return Err(EventError::ZeroOrderQuantity);
        }

        Ok(())
    }
}

// ============================================================
// Order cancelled
// ============================================================

/// Removes a specified quantity from a resting order.
///
/// Cancelled quantity is a decrement, not the resulting
/// remaining quantity.
///
/// Cancellation may remove all remaining liquidity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderCancelled {
    pub order_id: OrderId,
    pub cancelled_quantity: Quantity,
}

impl OrderCancelled {
    /// Validates stateless event invariants.
    ///
    /// # Errors
    ///
    /// Returns an error if the cancelled quantity is zero.
    #[inline]
    pub const fn validate(self) -> Result<(), EventError> {
        if self.cancelled_quantity.is_zero() {
            return Err(EventError::ZeroCancellationQuantity);
        }

        Ok(())
    }
}

// ============================================================
// Order deleted
// ============================================================

/// Removes all remaining liquidity associated with an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderDeleted {
    pub order_id: OrderId,
}

// ============================================================
// Order executed
// ============================================================

/// Reports an execution against a resting order.
///
/// Executed quantity is a decrement to resting liquidity.
///
/// Execution price may differ from the resting order price
/// depending on exchange rules.
///
/// Multiple execution events may reference the same trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderExecuted {
    pub order_id: OrderId,
    pub trade_id: TradeId,
    pub execution_price: Price,
    pub executed_quantity: Quantity,
}

impl OrderExecuted {
    /// Validates stateless event invariants.
    ///
    /// # Errors
    ///
    /// Returns an error if the executed quantity is zero.
    #[inline]
    pub const fn validate(self) -> Result<(), EventError> {
        if self.executed_quantity.is_zero() {
            return Err(EventError::ZeroExecutionQuantity);
        }

        Ok(())
    }
}

// ============================================================
// Trade
// ============================================================

/// Reports an executed transaction.
///
/// Trade reports do not independently decrement resting
/// liquidity. Order execution events or absolute level
/// updates are responsible for book state changes.
///
/// Aggressor side may be unavailable in the source feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trade {
    pub trade_id: TradeId,
    pub price: Price,
    pub quantity: Quantity,
    pub aggressor_side: Option<Side>,
}

impl Trade {
    /// Validates stateless event invariants.
    ///
    /// # Errors
    ///
    /// Returns an error if the trade quantity is zero.
    #[inline]
    pub const fn validate(self) -> Result<(), EventError> {
        if self.quantity.is_zero() {
            return Err(EventError::ZeroTradeQuantity);
        }

        Ok(())
    }
}

// ============================================================
// Price level updated
// ============================================================

/// Represents an absolute aggregate quantity at a price level.
///
/// Quantity zero removes the price level.
///
/// Order count is optional because some feeds do not
/// publish the number of orders at each level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceLevelUpdated {
    pub price: Price,
    pub quantity: Quantity,
    pub order_count: Option<u32>,
    pub side: Side,
}

impl PriceLevelUpdated {
    /// Returns whether this update removes the price level.
    #[must_use]
    #[inline]
    pub const fn is_removal(self) -> bool {
        self.quantity.is_zero()
    }

    /// Validates stateless event invariants.
    ///
    /// # Errors
    ///
    /// Returns an error if a supplied order count contradicts
    /// the presence or absence of aggregate liquidity.
    #[inline]
    pub const fn validate(self) -> Result<(), EventError> {
        match self.order_count {
            Some(count) if self.quantity.is_zero() != (count == 0) => {
                Err(EventError::InvalidLevelOrderCount)
            }
            _ => Ok(()),
        }
    }
}

// ============================================================
// Book cleared
// ============================================================

/// Removes all resting liquidity within the specified scope.
///
/// A book clear does not reset feed sequencing or establish
/// a new session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookCleared {
    pub scope: BookClearScope,
}

// ============================================================
// Trading status changed
// ============================================================

/// Reports a change in an instrument's trading status.
///
/// A status change does not implicitly clear the order book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradingStatusChanged {
    pub status: TradingStatus,
}

// ============================================================
// Market event payload
// ============================================================

/// Canonical market data event payload.
///
/// This enum is allocation-free and contains no borrowed
/// references or dynamically sized fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketEvent {
    OrderAdded(OrderAdded),
    OrderModified(OrderModified),
    OrderCancelled(OrderCancelled),
    OrderDeleted(OrderDeleted),
    OrderExecuted(OrderExecuted),
    Trade(Trade),
    PriceLevelUpdated(PriceLevelUpdated),
    BookCleared(BookCleared),
    TradingStatusChanged(TradingStatusChanged),
}

impl MarketEvent {
    // ========================================================
    // Event classification
    // ========================================================

    #[must_use]
    #[inline]
    pub const fn kind(self) -> EventKind {
        match self {
            Self::OrderAdded(_) => EventKind::OrderAdded,
            Self::OrderModified(_) => EventKind::OrderModified,
            Self::OrderCancelled(_) => EventKind::OrderCancelled,
            Self::OrderDeleted(_) => EventKind::OrderDeleted,
            Self::OrderExecuted(_) => EventKind::OrderExecuted,
            Self::Trade(_) => EventKind::Trade,
            Self::PriceLevelUpdated(_) => EventKind::PriceLevelUpdated,
            Self::BookCleared(_) => EventKind::BookCleared,
            Self::TradingStatusChanged(_) => EventKind::TradingStatusChanged,
        }
    }

    #[must_use]
    #[inline]
    pub const fn affects_book(self) -> bool {
        self.kind().affects_book()
    }

    #[must_use]
    #[inline]
    pub const fn is_order_event(self) -> bool {
        self.kind().is_order_event()
    }

    #[must_use]
    #[inline]
    pub const fn is_trade_event(self) -> bool {
        self.kind().is_trade_event()
    }

    // ========================================================
    // Identifier extraction
    // ========================================================

    /// Returns the order identifier, if present.
    #[must_use]
    #[inline]
    pub const fn order_id(self) -> Option<OrderId> {
        match self {
            Self::OrderAdded(event) => Some(event.order_id),
            Self::OrderModified(event) => Some(event.order_id),
            Self::OrderCancelled(event) => Some(event.order_id),
            Self::OrderDeleted(event) => Some(event.order_id),
            Self::OrderExecuted(event) => Some(event.order_id),
            _ => None,
        }
    }

    /// Returns the trade identifier, if present.
    #[must_use]
    #[inline]
    pub const fn trade_id(self) -> Option<TradeId> {
        match self {
            Self::OrderExecuted(event) => Some(event.trade_id),
            Self::Trade(event) => Some(event.trade_id),
            _ => None,
        }
    }

    // ========================================================
    // Validation
    // ========================================================

    /// Validates stateless event invariants.
    ///
    /// This does not validate sequence continuity, order
    /// existence, available liquidity, instrument-specific
    /// price limits, or exchange-specific trading rules.
    ///
    /// # Errors
    ///
    /// Returns the first violated event invariant.
    #[inline]
    pub const fn validate(self) -> Result<(), EventError> {
        match self {
            Self::OrderAdded(event) => event.validate(),
            Self::OrderModified(event) => event.validate(),
            Self::OrderCancelled(event) => event.validate(),
            Self::OrderExecuted(event) => event.validate(),
            Self::Trade(event) => event.validate(),
            Self::PriceLevelUpdated(event) => event.validate(),

            Self::OrderDeleted(_) | Self::BookCleared(_) | Self::TradingStatusChanged(_) => Ok(()),
        }
    }
}

// ============================================================
// Event conversions
// ============================================================

macro_rules! impl_event_conversion {
    ($payload:ty, $variant:ident) => {
        impl From<$payload> for MarketEvent {
            #[inline]
            fn from(value: $payload) -> Self {
                Self::$variant(value)
            }
        }
    };
}

impl_event_conversion!(OrderAdded, OrderAdded);
impl_event_conversion!(OrderModified, OrderModified);
impl_event_conversion!(OrderCancelled, OrderCancelled);
impl_event_conversion!(OrderDeleted, OrderDeleted);
impl_event_conversion!(OrderExecuted, OrderExecuted);
impl_event_conversion!(Trade, Trade);
impl_event_conversion!(PriceLevelUpdated, PriceLevelUpdated);
impl_event_conversion!(BookCleared, BookCleared);
impl_event_conversion!(TradingStatusChanged, TradingStatusChanged);

// ============================================================
// Event header
// ============================================================

/// Authoritative metadata associated with a market event.
///
/// Local receipt timestamps are intentionally excluded so
/// the canonical event remains deterministic across replay
/// and independent consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventHeader {
    pub id: EventId,
    pub instrument_id: InstrumentId,
    pub exchange_timestamp: Option<ExchangeTimestamp>,
}

impl EventHeader {
    #[must_use]
    #[inline]
    pub const fn new(
        id: EventId,
        instrument_id: InstrumentId,
        exchange_timestamp: Option<ExchangeTimestamp>,
    ) -> Self {
        Self {
            id,
            instrument_id,
            exchange_timestamp,
        }
    }

    #[must_use]
    #[inline]
    pub const fn sequence(self) -> SequenceNumber {
        self.id.sequence
    }

    #[must_use]
    #[inline]
    pub const fn feed(self) -> FeedId {
        self.id.feed
    }

    #[must_use]
    #[inline]
    pub const fn stream_id(self) -> StreamId {
        self.id.feed.stream_id
    }

    #[must_use]
    #[inline]
    pub const fn session_id(self) -> SessionId {
        self.id.feed.session_id
    }
}

// ============================================================
// Event envelope
// ============================================================

/// A canonical market event with authoritative metadata.
///
/// This representation is intended for deterministic replay,
/// sequencing, book reconstruction, and event distribution.
///
/// Construction does not implicitly validate the event.
/// Protocol decoders and other ingestion boundaries must
/// perform appropriate validation before accepting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventEnvelope {
    pub header: EventHeader,
    pub event: MarketEvent,
}

impl EventEnvelope {
    #[must_use]
    #[inline]
    pub const fn new(header: EventHeader, event: MarketEvent) -> Self {
        Self { header, event }
    }

    /// Constructs an envelope after stateless validation.
    ///
    /// # Errors
    ///
    /// Returns an error if the event violates an invariant.
    #[inline]
    pub const fn try_new(header: EventHeader, event: MarketEvent) -> Result<Self, EventError> {
        match event.validate() {
            Ok(()) => Ok(Self { header, event }),
            Err(error) => Err(error),
        }
    }

    #[must_use]
    #[inline]
    pub const fn id(self) -> EventId {
        self.header.id
    }

    #[must_use]
    #[inline]
    pub const fn kind(self) -> EventKind {
        self.event.kind()
    }

    #[must_use]
    #[inline]
    pub const fn instrument_id(self) -> InstrumentId {
        self.header.instrument_id
    }

    #[must_use]
    #[inline]
    pub const fn sequence(self) -> SequenceNumber {
        self.header.sequence()
    }

    #[must_use]
    #[inline]
    pub const fn feed(self) -> FeedId {
        self.header.feed()
    }

    #[must_use]
    #[inline]
    pub const fn exchange_timestamp(self) -> Option<ExchangeTimestamp> {
        self.header.exchange_timestamp
    }

    #[must_use]
    #[inline]
    pub const fn affects_book(self) -> bool {
        self.event.affects_book()
    }

    /// Validates stateless event invariants.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload is invalid.
    #[inline]
    pub const fn validate(self) -> Result<(), EventError> {
        self.event.validate()
    }
}

// ============================================================
// Ingress metadata
// ============================================================

/// Identifies how a market event entered Maxwell.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventSource {
    Live = 0,
    Retransmission = 1,
    Replay = 2,
    Synthetic = 3,
}

/// Local, nonauthoritative event receipt metadata.
///
/// Monotonic timestamps must not be persisted as globally
/// meaningful timestamps or compared across processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngressMetadata {
    pub receive_timestamp: Option<ReceiveTimestamp>,
    pub monotonic_timestamp: Option<MonotonicTimestamp>,
    pub source: EventSource,
}

impl IngressMetadata {
    #[must_use]
    #[inline]
    pub const fn new(
        receive_timestamp: Option<ReceiveTimestamp>,
        monotonic_timestamp: Option<MonotonicTimestamp>,
        source: EventSource,
    ) -> Self {
        Self {
            receive_timestamp,
            monotonic_timestamp,
            source,
        }
    }

    #[must_use]
    #[inline]
    pub const fn is_live(self) -> bool {
        matches!(self.source, EventSource::Live)
    }

    #[must_use]
    #[inline]
    pub const fn is_retransmission(self) -> bool {
        matches!(self.source, EventSource::Retransmission)
    }

    #[must_use]
    #[inline]
    pub const fn is_replay(self) -> bool {
        matches!(self.source, EventSource::Replay)
    }
}

// ============================================================
// Received event
// ============================================================

/// A canonical market event accompanied by local receipt
/// metadata.
///
/// Different consumers may observe different ingress
/// metadata for the same authoritative event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceivedEvent {
    pub envelope: EventEnvelope,
    pub ingress: IngressMetadata,
}

impl ReceivedEvent {
    #[must_use]
    #[inline]
    pub const fn new(envelope: EventEnvelope, ingress: IngressMetadata) -> Self {
        Self { envelope, ingress }
    }

    #[must_use]
    #[inline]
    pub const fn id(self) -> EventId {
        self.envelope.id()
    }

    #[must_use]
    #[inline]
    pub const fn kind(self) -> EventKind {
        self.envelope.kind()
    }

    #[must_use]
    #[inline]
    pub const fn sequence(self) -> SequenceNumber {
        self.envelope.sequence()
    }

    #[must_use]
    #[inline]
    pub const fn instrument_id(self) -> InstrumentId {
        self.envelope.instrument_id()
    }

    /// Returns the observed signed wall-clock difference
    /// between local receipt and exchange publication.
    ///
    /// This is not necessarily true network latency.
    #[must_use]
    #[inline]
    pub fn observed_exchange_offset(self) -> Option<i128> {
        let received = self.ingress.receive_timestamp?;
        let exchange = self.envelope.exchange_timestamp()?;

        Some(received.observed_exchange_offset(exchange))
    }

    /// Returns the canonical event without ingress metadata.
    #[must_use]
    #[inline]
    pub const fn into_envelope(self) -> EventEnvelope {
        self.envelope
    }
}

// ============================================================
// Compile-time layout guarantees
// ============================================================

const _: () = {
    assert!(size_of::<SessionId>() == size_of::<u64>());
    assert!(align_of::<SessionId>() == align_of::<u64>());

    assert!(size_of::<EventKind>() == size_of::<u8>());
    assert!(size_of::<TradingStatus>() == size_of::<u8>());
    assert!(size_of::<BookClearScope>() == size_of::<u8>());
    assert!(size_of::<EventSource>() == size_of::<u8>());

    assert!(!core::mem::needs_drop::<MarketEvent>());
    assert!(!core::mem::needs_drop::<EventEnvelope>());
    assert!(!core::mem::needs_drop::<ReceivedEvent>());
};

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    const fn feed() -> FeedId {
        FeedId::new(StreamId::new(1), SessionId::new(100))
    }

    const fn header() -> EventHeader {
        EventHeader::new(
            EventId::new(feed(), SequenceNumber::new(500)),
            InstrumentId::new(42),
            Some(ExchangeTimestamp::new(1_000_000)),
        )
    }

    const fn order_added() -> OrderAdded {
        OrderAdded {
            order_id: OrderId::new(1_001),
            price: Price::new(10_000),
            quantity: Quantity::new(500),
            side: Side::Buy,
        }
    }

    const fn envelope() -> EventEnvelope {
        EventEnvelope::new(header(), MarketEvent::OrderAdded(order_added()))
    }

    // ========================================================
    // Identity tests
    // ========================================================

    #[test]
    fn session_id_roundtrip() {
        let session = SessionId::new(100);

        assert_eq!(session.raw(), 100);
        assert_eq!(u64::from(session), 100);
    }

    #[test]
    fn session_id_display() {
        assert_eq!(SessionId::new(100).to_string(), "100");
    }

    #[test]
    fn feed_identity() {
        assert_eq!(feed().stream_id, StreamId::new(1));
        assert_eq!(feed().session_id, SessionId::new(100));
    }

    #[test]
    fn different_sessions_are_distinct() {
        let first = FeedId::new(StreamId::new(1), SessionId::new(100));

        let second = FeedId::new(StreamId::new(1), SessionId::new(101));

        assert_ne!(first, second);
    }

    #[test]
    fn different_streams_are_distinct() {
        let first = FeedId::new(StreamId::new(1), SessionId::new(100));

        let second = FeedId::new(StreamId::new(2), SessionId::new(100));

        assert_ne!(first, second);
    }

    #[test]
    fn event_identity() {
        let id = EventId::new(feed(), SequenceNumber::new(500));

        assert_eq!(id.feed, feed());
        assert_eq!(id.sequence, SequenceNumber::new(500));
    }

    #[test]
    fn event_identity_display() {
        assert_eq!(
            EventId::new(feed(), SequenceNumber::new(500),).to_string(),
            "1:100:500",
        );
    }

    #[test]
    fn equal_sequences_in_different_sessions() {
        let first = EventId::new(
            FeedId::new(StreamId::new(1), SessionId::new(100)),
            SequenceNumber::new(500),
        );

        let second = EventId::new(
            FeedId::new(StreamId::new(1), SessionId::new(101)),
            SequenceNumber::new(500),
        );

        assert_ne!(first, second);
    }

    // ========================================================
    // Order addition tests
    // ========================================================

    #[test]
    fn valid_order_addition() {
        assert_eq!(order_added().validate(), Ok(()));
    }

    #[test]
    fn zero_order_quantity() {
        let mut event = order_added();
        event.quantity = Quantity::ZERO;

        assert_eq!(event.validate(), Err(EventError::ZeroOrderQuantity),);
    }

    #[test]
    fn negative_order_price() {
        let mut event = order_added();
        event.price = Price::new(-100);

        assert_eq!(event.validate(), Ok(()));
    }

    // ========================================================
    // Order modification tests
    // ========================================================

    #[test]
    fn valid_order_modification() {
        let event = OrderModified {
            order_id: OrderId::new(1_001),
            new_price: Price::new(10_005),
            new_quantity: Quantity::new(400),
        };

        assert_eq!(event.validate(), Ok(()));
    }

    #[test]
    fn zero_modified_quantity() {
        let event = OrderModified {
            order_id: OrderId::new(1_001),
            new_price: Price::new(10_005),
            new_quantity: Quantity::ZERO,
        };

        assert_eq!(event.validate(), Err(EventError::ZeroOrderQuantity),);
    }

    // ========================================================
    // Cancellation tests
    // ========================================================

    #[test]
    fn valid_cancellation() {
        let event = OrderCancelled {
            order_id: OrderId::new(1_001),
            cancelled_quantity: Quantity::new(100),
        };

        assert_eq!(event.validate(), Ok(()));
    }

    #[test]
    fn zero_cancellation_quantity() {
        let event = OrderCancelled {
            order_id: OrderId::new(1_001),
            cancelled_quantity: Quantity::ZERO,
        };

        assert_eq!(event.validate(), Err(EventError::ZeroCancellationQuantity),);
    }

    // ========================================================
    // Execution tests
    // ========================================================

    #[test]
    fn valid_execution() {
        let event = OrderExecuted {
            order_id: OrderId::new(1_001),
            trade_id: TradeId::new(2_001),
            execution_price: Price::new(10_000),
            executed_quantity: Quantity::new(100),
        };

        assert_eq!(event.validate(), Ok(()));
    }

    #[test]
    fn zero_execution_quantity() {
        let event = OrderExecuted {
            order_id: OrderId::new(1_001),
            trade_id: TradeId::new(2_001),
            execution_price: Price::new(10_000),
            executed_quantity: Quantity::ZERO,
        };

        assert_eq!(event.validate(), Err(EventError::ZeroExecutionQuantity),);
    }

    // ========================================================
    // Trade tests
    // ========================================================

    #[test]
    fn valid_trade() {
        let event = Trade {
            trade_id: TradeId::new(2_001),
            price: Price::new(10_000),
            quantity: Quantity::new(100),
            aggressor_side: Some(Side::Sell),
        };

        assert_eq!(event.validate(), Ok(()));
    }

    #[test]
    fn trade_without_aggressor_side() {
        let event = Trade {
            trade_id: TradeId::new(2_001),
            price: Price::new(10_000),
            quantity: Quantity::new(100),
            aggressor_side: None,
        };

        assert_eq!(event.validate(), Ok(()));
    }

    #[test]
    fn zero_trade_quantity() {
        let event = Trade {
            trade_id: TradeId::new(2_001),
            price: Price::new(10_000),
            quantity: Quantity::ZERO,
            aggressor_side: None,
        };

        assert_eq!(event.validate(), Err(EventError::ZeroTradeQuantity),);
    }

    // ========================================================
    // Price level tests
    // ========================================================

    #[test]
    fn valid_price_level_update() {
        let event = PriceLevelUpdated {
            price: Price::new(10_000),
            quantity: Quantity::new(500),
            order_count: Some(5),
            side: Side::Buy,
        };

        assert_eq!(event.validate(), Ok(()));
        assert!(!event.is_removal());
    }

    #[test]
    fn price_level_without_order_count() {
        let event = PriceLevelUpdated {
            price: Price::new(10_000),
            quantity: Quantity::new(500),
            order_count: None,
            side: Side::Buy,
        };

        assert_eq!(event.validate(), Ok(()));
    }

    #[test]
    fn price_level_removal() {
        let event = PriceLevelUpdated {
            price: Price::new(10_000),
            quantity: Quantity::ZERO,
            order_count: Some(0),
            side: Side::Buy,
        };

        assert_eq!(event.validate(), Ok(()));
        assert!(event.is_removal());
    }

    #[test]
    fn price_level_removal_without_order_count() {
        let event = PriceLevelUpdated {
            price: Price::new(10_000),
            quantity: Quantity::ZERO,
            order_count: None,
            side: Side::Buy,
        };

        assert_eq!(event.validate(), Ok(()));
        assert!(event.is_removal());
    }

    #[test]
    fn nonzero_order_count_with_zero_quantity() {
        let event = PriceLevelUpdated {
            price: Price::new(10_000),
            quantity: Quantity::ZERO,
            order_count: Some(5),
            side: Side::Buy,
        };

        assert_eq!(event.validate(), Err(EventError::InvalidLevelOrderCount),);
    }

    #[test]
    fn zero_order_count_with_nonzero_quantity() {
        let event = PriceLevelUpdated {
            price: Price::new(10_000),
            quantity: Quantity::new(500),
            order_count: Some(0),
            side: Side::Buy,
        };

        assert_eq!(event.validate(), Err(EventError::InvalidLevelOrderCount),);
    }

    // ========================================================
    // Book clear tests
    // ========================================================

    #[test]
    fn clear_both_sides() {
        assert!(BookClearScope::BothSides.includes(Side::Buy));
        assert!(BookClearScope::BothSides.includes(Side::Sell));
    }

    #[test]
    fn clear_buy_side() {
        assert!(BookClearScope::BuySide.includes(Side::Buy));
        assert!(!BookClearScope::BuySide.includes(Side::Sell));
    }

    #[test]
    fn clear_sell_side() {
        assert!(BookClearScope::SellSide.includes(Side::Sell));
        assert!(!BookClearScope::SellSide.includes(Side::Buy));
    }

    // ========================================================
    // Trading status tests
    // ========================================================

    #[test]
    fn continuous_trading_status() {
        assert!(TradingStatus::Open.is_continuous_trading());

        for status in [
            TradingStatus::Unknown,
            TradingStatus::PreOpen,
            TradingStatus::Halted,
            TradingStatus::Auction,
            TradingStatus::Closed,
        ] {
            assert!(!status.is_continuous_trading());
        }
    }

    #[test]
    fn trading_status_display() {
        assert_eq!(TradingStatus::Open.to_string(), "OPEN");
        assert_eq!(TradingStatus::Halted.to_string(), "HALTED");
    }

    // ========================================================
    // Event classification tests
    // ========================================================

    #[test]
    fn order_event_classification() {
        let event = MarketEvent::OrderAdded(order_added());

        assert_eq!(event.kind(), EventKind::OrderAdded);
        assert!(event.is_order_event());
        assert!(event.affects_book());
        assert!(!event.is_trade_event());
    }

    #[test]
    fn execution_event_classification() {
        let event = MarketEvent::OrderExecuted(OrderExecuted {
            order_id: OrderId::new(1),
            trade_id: TradeId::new(2),
            execution_price: Price::new(100),
            executed_quantity: Quantity::new(10),
        });

        assert!(event.is_order_event());
        assert!(event.is_trade_event());
        assert!(event.affects_book());
    }

    #[test]
    fn trade_does_not_modify_book() {
        let event = MarketEvent::Trade(Trade {
            trade_id: TradeId::new(1),
            price: Price::new(100),
            quantity: Quantity::new(10),
            aggressor_side: None,
        });

        assert!(event.is_trade_event());
        assert!(!event.is_order_event());
        assert!(!event.affects_book());
    }

    #[test]
    fn status_does_not_modify_book() {
        let event = MarketEvent::TradingStatusChanged(TradingStatusChanged {
            status: TradingStatus::Halted,
        });

        assert!(!event.affects_book());
        assert!(!event.is_order_event());
        assert!(!event.is_trade_event());
    }

    #[test]
    fn level_update_classification() {
        assert!(EventKind::PriceLevelUpdated.affects_book());
        assert!(!EventKind::PriceLevelUpdated.is_order_event());
    }

    #[test]
    fn book_clear_classification() {
        assert!(EventKind::BookCleared.affects_book());
        assert!(!EventKind::BookCleared.is_order_event());
    }

    #[test]
    fn event_kind_display() {
        assert_eq!(EventKind::OrderAdded.to_string(), "ORDER_ADDED",);
    }

    // ========================================================
    // Identifier extraction tests
    // ========================================================

    #[test]
    fn extract_order_id() {
        let event = MarketEvent::OrderAdded(order_added());

        assert_eq!(event.order_id(), Some(OrderId::new(1_001)),);

        assert_eq!(event.trade_id(), None);
    }

    #[test]
    fn extract_execution_identifiers() {
        let event = MarketEvent::OrderExecuted(OrderExecuted {
            order_id: OrderId::new(1_001),
            trade_id: TradeId::new(2_001),
            execution_price: Price::new(100),
            executed_quantity: Quantity::new(10),
        });

        assert_eq!(event.order_id(), Some(OrderId::new(1_001)),);

        assert_eq!(event.trade_id(), Some(TradeId::new(2_001)),);
    }

    #[test]
    fn extract_trade_id() {
        let event = MarketEvent::Trade(Trade {
            trade_id: TradeId::new(2_001),
            price: Price::new(100),
            quantity: Quantity::new(10),
            aggressor_side: None,
        });

        assert_eq!(event.order_id(), None);

        assert_eq!(event.trade_id(), Some(TradeId::new(2_001)),);
    }

    // ========================================================
    // Event conversion tests
    // ========================================================

    #[test]
    fn order_added_conversion() {
        let event: MarketEvent = order_added().into();

        assert_eq!(event, MarketEvent::OrderAdded(order_added()),);
    }

    #[test]
    fn order_deleted_conversion() {
        let payload = OrderDeleted {
            order_id: OrderId::new(1_001),
        };

        let event: MarketEvent = payload.into();

        assert_eq!(event, MarketEvent::OrderDeleted(payload));
    }

    // ========================================================
    // Envelope tests
    // ========================================================

    #[test]
    fn event_header_accessors() {
        let header = header();

        assert_eq!(header.feed(), feed());
        assert_eq!(header.stream_id(), StreamId::new(1));
        assert_eq!(header.session_id(), SessionId::new(100));
        assert_eq!(header.sequence(), SequenceNumber::new(500));
    }

    #[test]
    fn event_envelope_accessors() {
        let event = envelope();

        assert_eq!(event.kind(), EventKind::OrderAdded);
        assert_eq!(event.instrument_id(), InstrumentId::new(42));
        assert_eq!(event.sequence(), SequenceNumber::new(500));
        assert_eq!(event.feed(), feed());

        assert_eq!(
            event.exchange_timestamp(),
            Some(ExchangeTimestamp::new(1_000_000)),
        );

        assert!(event.affects_book());
    }

    #[test]
    fn validated_envelope_construction() {
        let result = EventEnvelope::try_new(header(), MarketEvent::OrderAdded(order_added()));

        assert_eq!(result, Ok(envelope()));
    }

    #[test]
    fn invalid_envelope_construction() {
        let mut payload = order_added();
        payload.quantity = Quantity::ZERO;

        let result = EventEnvelope::try_new(header(), MarketEvent::OrderAdded(payload));

        assert_eq!(result, Err(EventError::ZeroOrderQuantity),);
    }

    #[test]
    fn envelope_without_exchange_timestamp() {
        let mut header = header();
        header.exchange_timestamp = None;

        let event = EventEnvelope::new(header, MarketEvent::OrderAdded(order_added()));

        assert_eq!(event.exchange_timestamp(), None);
        assert_eq!(event.validate(), Ok(()));
    }

    // ========================================================
    // Ingress metadata tests
    // ========================================================

    #[test]
    fn live_ingress_metadata() {
        let metadata = IngressMetadata::new(
            Some(ReceiveTimestamp::new(1_001_000)),
            Some(MonotonicTimestamp::new(500)),
            EventSource::Live,
        );

        assert!(metadata.is_live());
        assert!(!metadata.is_retransmission());
        assert!(!metadata.is_replay());
    }

    #[test]
    fn retransmission_metadata() {
        let metadata = IngressMetadata::new(None, None, EventSource::Retransmission);

        assert!(metadata.is_retransmission());
        assert!(!metadata.is_live());
    }

    #[test]
    fn replay_metadata() {
        let metadata = IngressMetadata::new(None, None, EventSource::Replay);

        assert!(metadata.is_replay());
        assert!(!metadata.is_live());
    }

    #[test]
    fn received_event_accessors() {
        let event = ReceivedEvent::new(
            envelope(),
            IngressMetadata::new(None, None, EventSource::Live),
        );

        assert_eq!(event.id(), envelope().id());
        assert_eq!(event.kind(), EventKind::OrderAdded);
        assert_eq!(event.sequence(), SequenceNumber::new(500));
        assert_eq!(event.instrument_id(), InstrumentId::new(42));
    }

    #[test]
    fn received_event_exchange_offset() {
        let event = ReceivedEvent::new(
            envelope(),
            IngressMetadata::new(
                Some(ReceiveTimestamp::new(1_001_000)),
                Some(MonotonicTimestamp::new(500)),
                EventSource::Live,
            ),
        );

        assert_eq!(event.observed_exchange_offset(), Some(1_000),);
    }

    #[test]
    fn received_event_negative_exchange_offset() {
        let event = ReceivedEvent::new(
            envelope(),
            IngressMetadata::new(
                Some(ReceiveTimestamp::new(999_000)),
                None,
                EventSource::Live,
            ),
        );

        assert_eq!(event.observed_exchange_offset(), Some(-1_000),);
    }

    #[test]
    fn missing_receive_timestamp() {
        let event = ReceivedEvent::new(
            envelope(),
            IngressMetadata::new(None, None, EventSource::Replay),
        );

        assert_eq!(event.observed_exchange_offset(), None);
    }

    #[test]
    fn missing_exchange_timestamp() {
        let mut envelope = envelope();
        envelope.header.exchange_timestamp = None;

        let event = ReceivedEvent::new(
            envelope,
            IngressMetadata::new(
                Some(ReceiveTimestamp::new(1_001_000)),
                None,
                EventSource::Live,
            ),
        );

        assert_eq!(event.observed_exchange_offset(), None);
    }

    // ========================================================
    // Deterministic replay tests
    // ========================================================

    #[test]
    fn ingress_metadata_does_not_change_canonical_event() {
        let live = ReceivedEvent::new(
            envelope(),
            IngressMetadata::new(
                Some(ReceiveTimestamp::new(1_001_000)),
                Some(MonotonicTimestamp::new(500)),
                EventSource::Live,
            ),
        );

        let replay = ReceivedEvent::new(
            envelope(),
            IngressMetadata::new(None, None, EventSource::Replay),
        );

        assert_ne!(live, replay);

        assert_eq!(live.into_envelope(), replay.into_envelope(),);

        assert_eq!(live.id(), replay.id());
    }

    #[test]
    fn event_is_copyable() {
        let original = envelope();
        let copied = original;

        assert_eq!(original, copied);
    }

    #[test]
    fn event_types_require_no_destructor() {
        assert!(!core::mem::needs_drop::<MarketEvent>());
        assert!(!core::mem::needs_drop::<EventEnvelope>());
        assert!(!core::mem::needs_drop::<ReceivedEvent>());
    }

    #[test]
    fn error_display() {
        assert_eq!(
            EventError::ZeroTradeQuantity.to_string(),
            "trade quantity must be positive",
        );
    }

    #[test]
    fn const_event_construction() {
        const EVENT: EventEnvelope = envelope();
        const VALIDATION: Result<(), EventError> = EVENT.validate();

        assert_eq!(VALIDATION, Ok(()));
    }
}
