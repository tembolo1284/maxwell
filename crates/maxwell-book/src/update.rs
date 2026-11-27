
//! Normalized order book updates.
//!
//! A protocol adapter translates authoritative market
//! events into these mutations.
//!
//! Trade reports must not reduce resting quantities if
//! separate execution events already perform that mutation.

use maxwell_core::{
    FeedId,
    InstrumentId,
    OrderId,
    Price,
    Quantity,
    SequenceNumber,
    Side,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookUpdateKind {
    /// Adds a newly resting order.
    Add {
        order_id: OrderId,
        side: Side,
        price: Price,
        quantity: Quantity,
    },

    /// Reduces remaining quantity by a positive delta.
    ///
    /// A reduction equal to the remaining quantity
    /// removes the order.
    Reduce {
        order_id: OrderId,
        quantity: Quantity,
    },

    /// Removes the entire remaining order.
    Delete {
        order_id: OrderId,
    },

    /// Execution against an identified resting order.
    ///
    /// Quantity represents the executed delta, not
    /// the new remaining quantity.
    Execute {
        order_id: OrderId,
        quantity: Quantity,
    },

    /// Replaces an existing order and establishes new
    /// priority at the current sequence.
    ///
    /// Only use this when the exchange's replacement
    /// semantics explicitly reset order priority.
    Replace {
        order_id: OrderId,
        new_price: Price,
        new_quantity: Quantity,
    },

    /// Advances the instrument watermark without
    /// changing resting order state.
    Observe,
}

impl BookUpdateKind {
    #[must_use]
    #[inline]
    pub const fn affects_book(self) -> bool {
        !matches!(self, Self::Observe)
    }

    #[must_use]
    #[inline]
    pub const fn order_id(self) -> Option<OrderId> {
        match self {
            Self::Add { order_id, .. }
            | Self::Reduce { order_id, .. }
            | Self::Delete { order_id }
            | Self::Execute { order_id, .. }
            | Self::Replace { order_id, .. } => Some(order_id),

            Self::Observe => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookUpdate {
    pub feed: FeedId,
    pub instrument_id: InstrumentId,
    pub sequence: SequenceNumber,
    pub kind: BookUpdateKind,
}

impl BookUpdate {
    #[must_use]
    #[inline]
    pub const fn new(
        feed: FeedId,
        instrument_id: InstrumentId,
        sequence: SequenceNumber,
        kind: BookUpdateKind,
    ) -> Self {
        Self {
            feed,
            instrument_id,
            sequence,
            kind,
        }
    }
}

