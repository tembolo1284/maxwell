//! Level 2 market data: aggregated price depth.
//!
//! The primary API writes into caller-provided storage,
//! avoiding allocation during market data publication.

use maxwell_core::{Price, Quantity, Side};

use crate::book::OrderBook;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthLevel {
    pub price: Price,
    pub quantity: Quantity,
    pub order_count: u32,
}

impl DepthLevel {
    pub const EMPTY: Self = Self {
        price: Price::ZERO,
        quantity: Quantity::ZERO,
        order_count: 0,
    };
}

impl Default for DepthLevel {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Level2<'a> {
    book: &'a OrderBook,
}

impl<'a> Level2<'a> {
    #[must_use]
    #[inline]
    pub const fn new(book: &'a OrderBook) -> Self {
        Self { book }
    }

    #[must_use]
    #[inline]
    pub const fn book(&self) -> &'a OrderBook {
        self.book
    }

    /// Iterates aggregated levels in best-to-worst order.
    pub fn iter(&self, side: Side) -> impl Iterator<Item = DepthLevel> + '_ {
        self.book
            .ladder(side)
            .iter()
            .map(|(price, level)| DepthLevel {
                price,
                quantity: level.quantity(),
                order_count: level.order_count(),
            })
    }

    /// Writes as many levels as fit in the output slice.
    ///
    /// Returns the number of populated entries.
    ///
    /// Only output[..count] is valid after this operation.
    pub fn write_depth(&self, side: Side, output: &mut [DepthLevel]) -> usize {
        let mut count = 0;

        for (destination, level) in output.iter_mut().zip(self.iter(side)) {
            *destination = level;
            count += 1;
        }

        count
    }

    /// Writes both sides into independent output buffers.
    pub fn write_both(&self, bids: &mut [DepthLevel], asks: &mut [DepthLevel]) -> (usize, usize) {
        (
            self.write_depth(Side::Buy, bids),
            self.write_depth(Side::Sell, asks),
        )
    }

    /// Total quantity across the first N occupied levels.
    ///
    /// Uses u128 to avoid overflow when aggregating
    /// multiple u64 price level quantities.
    #[must_use]
    pub fn cumulative_quantity(&self, side: Side, depth: usize) -> u128 {
        self.iter(side)
            .take(depth)
            .map(|level| u128::from(level.quantity.raw()))
            .sum()
    }

    #[must_use]
    #[inline]
    pub fn active_levels(&self, side: Side) -> usize {
        self.book.ladder(side).active_levels()
    }
}
