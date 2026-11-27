//! Level 1 market data: best bid and offer.
//!
//! Midpoint and spread calculations use widened integer
//! arithmetic to avoid overflow and preserve precision.

use maxwell_core::{FeedId, InstrumentId, Price, Quantity, SequenceNumber};

use crate::book::{BookState, OrderBook};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    pub price: Price,
    pub quantity: Quantity,
    pub order_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level1 {
    pub instrument_id: InstrumentId,
    pub feed: FeedId,
    pub sequence: Option<SequenceNumber>,
    pub state: BookState,
    pub bid: Option<Quote>,
    pub ask: Option<Quote>,
}

impl Level1 {
    #[must_use]
    #[inline]
    pub fn from_book(book: &OrderBook) -> Self {
        let bid = book.best_bid().map(|(price, level)| Quote {
            price,
            quantity: level.quantity(),
            order_count: level.order_count(),
        });

        let ask = book.best_ask().map(|(price, level)| Quote {
            price,
            quantity: level.quantity(),
            order_count: level.order_count(),
        });

        Self {
            instrument_id: book.instrument_id(),
            feed: book.feed(),
            sequence: book.last_sequence(),
            state: book.state(),
            bid,
            ask,
        }
    }

    #[must_use]
    #[inline]
    pub const fn is_live(self) -> bool {
        matches!(self.state, BookState::Live)
    }

    #[must_use]
    #[inline]
    pub const fn is_two_sided(self) -> bool {
        self.bid.is_some() && self.ask.is_some()
    }

    /// Signed spread in raw price units.
    ///
    /// Negative values indicate a crossed book.
    #[must_use]
    #[inline]
    pub const fn spread(self) -> Option<i128> {
        match (self.bid, self.ask) {
            (Some(bid), Some(ask)) => Some(ask.price.raw() as i128 - bid.price.raw() as i128),

            _ => None,
        }
    }

    /// Exact midpoint numerator with an implicit denominator
    /// of two.
    ///
    /// Preserves half-unit midpoints without floating point.
    #[must_use]
    #[inline]
    pub const fn midpoint_numerator(self) -> Option<i128> {
        match (self.bid, self.ask) {
            (Some(bid), Some(ask)) => Some(bid.price.raw() as i128 + ask.price.raw() as i128),

            _ => None,
        }
    }

    /// Midpoint rounded toward negative infinity.
    #[must_use]
    #[inline]
    pub fn midpoint_floor(self) -> Option<Price> {
        let numerator = self.midpoint_numerator()?;
        let raw = numerator.div_euclid(2);

        i64::try_from(raw).ok().map(Price::new)
    }

    #[must_use]
    #[inline]
    pub const fn is_crossed(self) -> bool {
        matches!(self.spread(), Some(value) if value < 0)
    }

    #[must_use]
    #[inline]
    pub const fn is_locked(self) -> bool {
        matches!(self.spread(), Some(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{SessionId, StreamId};

    fn quote(price: i64) -> Quote {
        Quote {
            price: Price::new(price),
            quantity: Quantity::new(100),
            order_count: 1,
        }
    }

    fn bbo(bid: Option<Quote>, ask: Option<Quote>) -> Level1 {
        Level1 {
            instrument_id: InstrumentId::new(1),
            feed: FeedId::new(StreamId::new(1), SessionId::new(1)),
            sequence: Some(SequenceNumber::ONE),
            state: BookState::Live,
            bid,
            ask,
        }
    }

    #[test]
    fn normal_spread() {
        let value = bbo(Some(quote(100)), Some(quote(105)));

        assert_eq!(value.spread(), Some(5));
        assert_eq!(value.midpoint_numerator(), Some(205));
        assert_eq!(value.midpoint_floor(), Some(Price::new(102)));
    }

    #[test]
    fn crossed_book() {
        let value = bbo(Some(quote(105)), Some(quote(100)));

        assert!(value.is_crossed());
        assert_eq!(value.spread(), Some(-5));
    }

    #[test]
    fn locked_book() {
        let value = bbo(Some(quote(100)), Some(quote(100)));

        assert!(value.is_locked());
    }

    #[test]
    fn one_sided_book() {
        let value = bbo(Some(quote(100)), None);

        assert!(!value.is_two_sided());
        assert_eq!(value.spread(), None);
        assert_eq!(value.midpoint_floor(), None);
    }

    #[test]
    fn negative_midpoint_rounding() {
        let value = bbo(Some(quote(-5)), Some(quote(-2)));

        assert_eq!(value.midpoint_numerator(), Some(-7));
        assert_eq!(value.midpoint_floor(), Some(Price::new(-4)));
    }

    #[test]
    fn full_price_range_spread() {
        let value = bbo(Some(quote(i64::MIN)), Some(quote(i64::MAX)));

        assert_eq!(value.spread(), Some(i128::from(u64::MAX)));
    }
}
