//! Deterministic full-depth order book snapshots.
//!
//! Snapshots store individual resting orders. Aggregate
//! price levels are reconstructed during restoration.
//!
//! Snapshot creation and restoration are control-path
//! operations and may allocate memory.

use maxwell_core::{FeedId, Instrument, SequenceNumber};

use crate::book::{BookState, OrderBook};
use crate::config::BookConfig;
use crate::error::{BookError, BookResult};
use crate::order::RestingOrder;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSnapshot {
    pub instrument: Instrument,
    pub feed: FeedId,
    pub last_sequence: Option<SequenceNumber>,
    pub orders: Vec<RestingOrder>,
}

impl BookSnapshot {
    /// Captures a deterministic snapshot of a live book.
    pub fn capture(book: &OrderBook) -> BookResult<Self> {
        if book.state() != BookState::Live {
            return Err(BookError::RecoveryRequired);
        }

        let mut orders = Vec::new();

        orders
            .try_reserve_exact(book.order_count())
            .map_err(|_| BookError::AllocationFailed)?;

        orders.extend(book.orders().iter().copied());

        // Canonical ordering independent of HashMap
        // iteration order and randomized hash seeds.
        orders.sort_unstable_by_key(|order| order.order_id());

        Ok(Self {
            instrument: *book.instrument(),
            feed: book.feed(),
            last_sequence: book.last_sequence(),
            orders,
        })
    }

    /// Reconstructs a complete book in isolated storage.
    ///
    /// The original live book is not modified if snapshot
    /// restoration fails.
    pub fn restore(&self, config: BookConfig) -> BookResult<OrderBook> {
        config.validate()?;

        if self.orders.len() > config.max_orders {
            return Err(BookError::OrderCapacityExceeded);
        }

        if self.last_sequence.is_none() && !self.orders.is_empty() {
            return Err(BookError::InvalidSnapshot);
        }

        let mut book = OrderBook::new(self.instrument, self.feed, config)?;

        for order in &self.orders {
            if let Some(watermark) = self.last_sequence {
                if order.priority_sequence() > watermark {
                    return Err(BookError::InvalidSnapshot);
                }
            }

            book.add_order(*order)?;
        }

        book.set_snapshot_watermark(self.last_sequence);
        book.validate_consistency()?;

        Ok(book)
    }

    /// Replaces a book only after successful reconstruction.
    ///
    /// The caller must coordinate this operation with the
    /// stream sequencer and recovery subsystem.
    pub fn install(&self, book: &mut OrderBook) -> BookResult<()> {
        if self.instrument != *book.instrument() || self.feed != book.feed() {
            return Err(BookError::InvalidSnapshot);
        }

        if self.last_sequence < book.last_sequence() {
            return Err(BookError::InvalidSnapshot);
        }

        let restored = self.restore(book.config())?;

        *book = restored;

        Ok(())
    }
}
