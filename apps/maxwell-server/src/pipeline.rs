//! Market data normalization, book mutation, and publication.
//!
//! The pipeline owns the reconstructed books and downstream
//! gateway. The gateway owns the publisher.
//!
//! Upstream sequencing is performed externally by the
//! recovery coordinator.
//!
//! Live publication rejection retains the original event
//! without applying its book mutation a second time.

use maxwell_book::{BookManager, BookUpdate, BookUpdateKind};
use maxwell_core::{EventEnvelope, InstrumentId, MarketEvent};
use maxwell_gateway::{Gateway, GatewayError};
use maxwell_publisher::{EventKind, PublishReport, PublisherError};

use crate::config::ServerConfig;
use crate::error::{ServerError, ServerResult};

// ============================================================
// Types
// ============================================================

/// Concrete gateway used by the server.
pub type MarketGateway = Gateway<InstrumentId, EventEnvelope>;

/// Result of a publication attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationOutcome {
    Published { sequence: u64 },
    Blocked,
    Idle,
}

/// An event whose book mutation has already been applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingPublication {
    event: EventEnvelope,
    kind: EventKind,
    timestamp_ns: u64,
}

// ============================================================
// Event normalization
// ============================================================

/// Converts a canonical event into an L3 book update.
///
/// Aggregate L2 updates and book-clearing events require
/// explicit integration semantics and are rejected for now.
///
/// Trade reports advance the instrument watermark without
/// independently reducing resting liquidity.
pub fn normalize_event(envelope: EventEnvelope) -> ServerResult<(BookUpdate, EventKind)> {
    envelope
        .event
        .validate()
        .map_err(|_| ServerError::InvalidState("invalid market event"))?;

    let (kind, publication_kind) = match envelope.event {
        MarketEvent::OrderAdded(event) => (
            BookUpdateKind::Add {
                order_id: event.order_id,
                side: event.side,
                price: event.price,
                quantity: event.quantity,
            },
            EventKind::BookUpdate,
        ),

        MarketEvent::OrderModified(event) => (
            BookUpdateKind::Replace {
                order_id: event.order_id,
                new_price: event.new_price,
                new_quantity: event.new_quantity,
            },
            EventKind::BookUpdate,
        ),

        MarketEvent::OrderCancelled(event) => (
            BookUpdateKind::Reduce {
                order_id: event.order_id,
                quantity: event.cancelled_quantity,
            },
            EventKind::BookUpdate,
        ),

        MarketEvent::OrderDeleted(event) => (
            BookUpdateKind::Delete {
                order_id: event.order_id,
            },
            EventKind::BookUpdate,
        ),

        MarketEvent::OrderExecuted(event) => (
            BookUpdateKind::Execute {
                order_id: event.order_id,
                quantity: event.executed_quantity,
            },
            EventKind::BookUpdate,
        ),

        MarketEvent::Trade(_) => (BookUpdateKind::Observe, EventKind::Trade),

        MarketEvent::TradingStatusChanged(_) => {
            (BookUpdateKind::Observe, EventKind::InstrumentStatus)
        }

        MarketEvent::PriceLevelUpdated(_) => {
            return Err(ServerError::InvalidState(
                "aggregate L2 updates are unsupported by the L3 pipeline",
            ));
        }

        MarketEvent::BookCleared(_) => {
            return Err(ServerError::InvalidState(
                "book clearing requires explicit reset semantics",
            ));
        }
    };

    let update = BookUpdate::new(
        envelope.header.feed(),
        envelope.header.instrument_id,
        envelope.sequence(),
        kind,
    );

    Ok((update, publication_kind))
}

// ============================================================
// Pipeline
// ============================================================

/// Single-owner market data processing pipeline.
///
/// Mutable book access is intentionally restricted to
/// application-internal initialization and recovery paths.
pub struct Pipeline {
    books: BookManager,
    gateway: MarketGateway,
    pending: Option<PendingPublication>,
    valid: bool,
}

impl Pipeline {
    /// Initializes the processing components.
    ///
    /// A newly constructed pipeline is not synchronized.
    pub fn new(config: &ServerConfig) -> ServerResult<Self> {
        config.validate()?;

        Ok(Self {
            books: BookManager::new(config.book)?,
            gateway: MarketGateway::new(config.gateway)?,
            pending: None,
            valid: false,
        })
    }

    #[must_use]
    pub const fn books(&self) -> &BookManager {
        &self.books
    }

    #[must_use]
    pub const fn gateway(&self) -> &MarketGateway {
        &self.gateway
    }

    pub(crate) fn gateway_mut(&mut self) -> &mut MarketGateway {
        &mut self.gateway
    }

    pub(crate) fn books_mut(&mut self) -> &mut BookManager {
        &mut self.books
    }

    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.valid
    }

    #[must_use]
    pub const fn has_pending_publication(&self) -> bool {
        self.pending.is_some()
    }

    #[must_use]
    pub fn publication_watermark(&self) -> u64 {
        self.gateway.publisher_watermark()
    }

    /// Marks an authoritative reconstructed baseline as valid.
    ///
    /// The caller must first establish upstream synchronization.
    pub(crate) fn mark_valid(&mut self) -> ServerResult<()> {
        if self.pending.is_some() {
            return Err(ServerError::InvariantViolation(
                "cannot establish a baseline with pending publication",
            ));
        }

        self.valid = true;

        Ok(())
    }

    /// Invalidates the published state and all subscribers.
    ///
    /// Existing book contents are retained for retransmission
    /// recovery, but must not be advertised as current.
    pub(crate) fn invalidate(&mut self) {
        self.valid = false;
        self.pending = None;

        self.gateway.require_resync_all();
    }

    /// Applies one sequenced event and retains its publication.
    ///
    /// Publication is performed separately so that successful
    /// book application can be committed by the recovery cursor
    /// before downstream delivery is attempted.
    pub(crate) fn stage_live(
        &mut self,
        event: EventEnvelope,
        timestamp_ns: u64,
    ) -> ServerResult<()> {
        if !self.valid {
            return Err(ServerError::InvalidState(
                "cannot stage an event on an invalid baseline",
            ));
        }

        if self.pending.is_some() {
            return Err(ServerError::InvalidState(
                "previous publication is still pending",
            ));
        }

        let (update, kind) = normalize_event(event)?;

        self.books.apply(update)?;

        self.pending = Some(PendingPublication {
            event,
            kind,
            timestamp_ns,
        });

        Ok(())
    }

    /// Applies a recovery event without publishing it.
    ///
    /// Subscribers must have been invalidated before this
    /// method is called.
    pub(crate) fn apply_recovery(&mut self, event: EventEnvelope) -> ServerResult<()> {
        if self.valid || self.pending.is_some() {
            return Err(ServerError::InvariantViolation(
                "recovery requires an invalidated publication baseline",
            ));
        }

        let (update, _) = normalize_event(event)?;

        self.books.apply(update)?;

        Ok(())
    }

    /// Attempts to publish an already-applied event.
    ///
    /// Backpressure retains the publication for another attempt.
    /// No book mutation occurs in this method.
    pub fn retry_publication(&mut self) -> ServerResult<PublicationOutcome> {
        let Some(pending) = self.pending else {
            return Ok(PublicationOutcome::Idle);
        };

        if !self.valid {
            return Err(ServerError::InvalidState(
                "cannot publish an invalid baseline",
            ));
        }

        let result = self.gateway.publish(
            Some(pending.event.instrument_id()),
            pending.kind,
            pending.timestamp_ns,
            pending.event,
        );

        match result {
            Ok(PublishReport { sequence, .. }) => {
                self.pending = None;

                Ok(PublicationOutcome::Published { sequence })
            }

            Err(GatewayError::Publisher(PublisherError::Backpressure { .. })) => {
                Ok(PublicationOutcome::Blocked)
            }

            Err(error) => {
                self.invalidate();

                Err(ServerError::Gateway(error))
            }
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
    fn publication_outcomes_are_distinct() {
        assert_ne!(PublicationOutcome::Blocked, PublicationOutcome::Idle,);

        assert_ne!(
            PublicationOutcome::Published { sequence: 1 },
            PublicationOutcome::Published { sequence: 2 },
        );
    }

    #[test]
    fn new_pipeline_has_no_valid_baseline() {
        let pipeline = Pipeline::new(&ServerConfig::default()).unwrap();

        assert!(!pipeline.is_valid());
        assert!(!pipeline.has_pending_publication());
    }

    #[test]
    fn empty_publication_retry_is_idle() {
        let mut pipeline = Pipeline::new(&ServerConfig::default()).unwrap();

        assert_eq!(
            pipeline.retry_publication().unwrap(),
            PublicationOutcome::Idle,
        );
    }

    #[test]
    fn baseline_can_be_invalidated() {
        let mut pipeline = Pipeline::new(&ServerConfig::default()).unwrap();

        pipeline.mark_valid().unwrap();

        assert!(pipeline.is_valid());

        pipeline.invalidate();

        assert!(!pipeline.is_valid());
        assert!(!pipeline.has_pending_publication());
    }
}
