//! Preallocated, bitmap-indexed price ladder.
//!
//! Each side owns a contiguous array of price levels.
//! Occupancy bits identify nonempty levels.
//!
//! Bids are traversed from highest to lowest price.
//! Asks are traversed from lowest to highest price.

use maxwell_core::{Instrument, Price, Side};

use crate::error::{BookError, BookResult};
use crate::level::PriceLevel;

const WORD_BITS: usize = 64;

#[derive(Debug)]
pub struct PriceLadder {
    instrument: Instrument,
    side: Side,
    levels: Box<[PriceLevel]>,
    occupancy: Box<[u64]>,
    active_levels: usize,
    best_index: Option<usize>,
}

impl PriceLadder {
    /// Creates an empty ladder covering the instrument's price band.
    ///
    /// # Errors
    ///
    /// Returns `PriceLevelCapacityExceeded` if the band is empty or
    /// needs more levels than `max_levels`, or `AllocationFailed` if storage
    /// cannot be reserved.
    pub fn new(instrument: Instrument, side: Side, max_levels: usize) -> BookResult<Self> {
        let count = instrument
            .price_level_count()
            .ok_or(BookError::PriceLevelCapacityExceeded)?;

        let count = usize::try_from(count).map_err(|_| BookError::PriceLevelCapacityExceeded)?;

        if count == 0 || count > max_levels {
            return Err(BookError::PriceLevelCapacityExceeded);
        }

        let word_count = count.div_ceil(WORD_BITS);

        let mut levels = Vec::new();

        levels
            .try_reserve_exact(count)
            .map_err(|_| BookError::AllocationFailed)?;

        levels.resize(count, PriceLevel::EMPTY);

        let mut occupancy = Vec::new();

        occupancy
            .try_reserve_exact(word_count)
            .map_err(|_| BookError::AllocationFailed)?;

        occupancy.resize(word_count, 0);

        Ok(Self {
            instrument,
            side,
            levels: levels.into_boxed_slice(),
            occupancy: occupancy.into_boxed_slice(),
            active_levels: 0,
            best_index: None,
        })
    }

    #[must_use]
    #[inline]
    pub const fn side(&self) -> Side {
        self.side
    }

    #[must_use]
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.levels.len()
    }

    #[must_use]
    #[inline]
    pub const fn active_levels(&self) -> usize {
        self.active_levels
    }

    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.active_levels == 0
    }

    #[must_use]
    #[inline]
    pub fn index_of(&self, price: Price) -> Option<usize> {
        let index = self.instrument.price_to_tick_index(price)?;
        let index = usize::try_from(index).ok()?;

        (index < self.levels.len()).then_some(index)
    }

    #[must_use]
    #[inline]
    pub fn price_at(&self, index: usize) -> Option<Price> {
        if index >= self.levels.len() {
            return None;
        }

        self.instrument.tick_index_to_price(index as u64)
    }

    #[must_use]
    #[inline]
    pub fn level(&self, price: Price) -> Option<PriceLevel> {
        let index = self.index_of(price)?;

        self.levels.get(index).copied()
    }

    #[must_use]
    #[inline]
    pub fn level_at(&self, index: usize) -> Option<PriceLevel> {
        self.levels.get(index).copied()
    }

    #[must_use]
    #[inline]
    pub fn best_price(&self) -> Option<Price> {
        self.price_at(self.best_index?)
    }

    #[must_use]
    #[inline]
    pub fn best_level(&self) -> Option<(Price, PriceLevel)> {
        let index = self.best_index?;

        Some((self.price_at(index)?, self.levels[index]))
    }

    /// Installs an already validated aggregate level.
    ///
    /// Restricted to the crate so callers cannot mutate
    /// aggregate depth independently of the order registry.
    ///
    /// # Errors
    ///
    /// Returns `InvalidPrice` if `price` is not a level of this
    /// ladder.
    #[inline]
    pub fn set_level(&mut self, price: Price, level: PriceLevel) -> BookResult<()> {
        let index = self.index_of(price).ok_or(BookError::InvalidPrice(price))?;

        self.set_level_at(index, level);

        Ok(())
    }

    #[inline]
    pub(crate) fn set_level_at(&mut self, index: usize, level: PriceLevel) {
        let was_empty = self.levels[index].is_empty();
        let now_empty = level.is_empty();

        self.levels[index] = level;

        match (was_empty, now_empty) {
            (true, false) => {
                self.set_occupied(index);
                self.active_levels += 1;
                self.update_best_on_insert(index);
            }

            (false, true) => {
                self.clear_occupied(index);
                self.active_levels -= 1;

                if self.best_index == Some(index) {
                    self.best_index = self.find_best();
                }
            }

            _ => {}
        }
    }

    #[inline]
    fn set_occupied(&mut self, index: usize) {
        let word = index / WORD_BITS;
        let bit = index % WORD_BITS;

        self.occupancy[word] |= 1_u64 << bit;
    }

    #[inline]
    fn clear_occupied(&mut self, index: usize) {
        let word = index / WORD_BITS;
        let bit = index % WORD_BITS;

        self.occupancy[word] &= !(1_u64 << bit);
    }

    #[inline]
    const fn update_best_on_insert(&mut self, index: usize) {
        let replace = match self.best_index {
            None => true,

            Some(current) => match self.side {
                Side::Buy => index > current,
                Side::Sell => index < current,
            },
        };

        if replace {
            self.best_index = Some(index);
        }
    }

    fn find_best(&self) -> Option<usize> {
        match self.side {
            Side::Buy => self.previous_occupied(self.capacity() - 1),
            Side::Sell => self.next_occupied(0),
        }
    }

    /// Finds the first occupied index at or above `start`.
    #[must_use]
    pub fn next_occupied(&self, start: usize) -> Option<usize> {
        if start >= self.capacity() {
            return None;
        }

        let first_word = start / WORD_BITS;
        let first_bit = start % WORD_BITS;

        for word_index in first_word..self.occupancy.len() {
            let mut word = self.occupancy[word_index];

            if word_index == first_word {
                word &= u64::MAX << first_bit;
            }

            if word != 0 {
                let bit = word.trailing_zeros() as usize;
                return Some(word_index * WORD_BITS + bit);
            }
        }

        None
    }

    /// Finds the first occupied index at or below `start`.
    #[must_use]
    pub fn previous_occupied(&self, start: usize) -> Option<usize> {
        if start >= self.capacity() {
            return None;
        }

        let first_word = start / WORD_BITS;
        let first_bit = start % WORD_BITS;

        for word_index in (0..=first_word).rev() {
            let mut word = self.occupancy[word_index];

            if word_index == first_word {
                word &= u64::MAX >> (63 - first_bit);
            }

            if word != 0 {
                let bit = 63 - word.leading_zeros() as usize;
                return Some(word_index * WORD_BITS + bit);
            }
        }

        None
    }

    /// Iterates occupied levels in best-to-worst price order.
    #[must_use]
    pub const fn iter(&self) -> LadderIter<'_> {
        LadderIter {
            ladder: self,
            next_index: self.best_index,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.levels.fill(PriceLevel::EMPTY);
        self.occupancy.fill(0);
        self.active_levels = 0;
        self.best_index = None;
    }
}

#[derive(Debug)]
pub struct LadderIter<'a> {
    ladder: &'a PriceLadder,
    next_index: Option<usize>,
}

impl<'a> IntoIterator for &'a PriceLadder {
    type Item = (Price, PriceLevel);
    type IntoIter = LadderIter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for LadderIter<'_> {
    type Item = (Price, PriceLevel);

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.next_index?;

        self.next_index = match self.ladder.side {
            Side::Buy => index
                .checked_sub(1)
                .and_then(|i| self.ladder.previous_occupied(i)),

            Side::Sell => index
                .checked_add(1)
                .and_then(|i| self.ladder.next_occupied(i)),
        };

        Some((self.ladder.price_at(index)?, self.ladder.levels[index]))
    }
}

impl core::iter::FusedIterator for LadderIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{InstrumentConfig, InstrumentId, Quantity};

    fn instrument() -> Instrument {
        Instrument::new(InstrumentConfig {
            id: InstrumentId::new(1),
            price_scale: 0,
            quantity_scale: 0,
            tick_size: Price::new(1),
            lot_size: Quantity::ONE,
            min_price: Price::new(0),
            max_price: Price::new(255),
            min_quantity: Quantity::ONE,
            max_quantity: Quantity::MAX,
        })
        .unwrap()
    }

    fn level() -> PriceLevel {
        PriceLevel::new(Quantity::new(100), 1).unwrap()
    }

    #[test]
    fn bid_priority() {
        let mut ladder = PriceLadder::new(instrument(), Side::Buy, 256).unwrap();

        ladder.set_level(Price::new(100), level()).unwrap();
        ladder.set_level(Price::new(200), level()).unwrap();

        assert_eq!(ladder.best_price(), Some(Price::new(200)));
    }

    #[test]
    fn ask_priority() {
        let mut ladder = PriceLadder::new(instrument(), Side::Sell, 256).unwrap();

        ladder.set_level(Price::new(100), level()).unwrap();
        ladder.set_level(Price::new(200), level()).unwrap();

        assert_eq!(ladder.best_price(), Some(Price::new(100)));
    }

    #[test]
    fn best_level_removal() {
        let mut ladder = PriceLadder::new(instrument(), Side::Buy, 256).unwrap();

        ladder.set_level(Price::new(100), level()).unwrap();
        ladder.set_level(Price::new(200), level()).unwrap();

        ladder
            .set_level(Price::new(200), PriceLevel::EMPTY)
            .unwrap();

        assert_eq!(ladder.best_price(), Some(Price::new(100)));
        assert_eq!(ladder.active_levels(), 1);
    }

    #[test]
    fn bitmap_word_boundaries() {
        let mut ladder = PriceLadder::new(instrument(), Side::Sell, 256).unwrap();

        for price in [0, 63, 64, 127, 128, 255] {
            ladder.set_level(Price::new(price), level()).unwrap();
        }

        let prices: Vec<_> = ladder.iter().map(|(price, _)| price.raw()).collect();

        assert_eq!(prices, vec![0, 63, 64, 127, 128, 255]);
    }

    #[test]
    fn reverse_bitmap_traversal() {
        let mut ladder = PriceLadder::new(instrument(), Side::Buy, 256).unwrap();

        for price in [0, 63, 64, 127, 128, 255] {
            ladder.set_level(Price::new(price), level()).unwrap();
        }

        let prices: Vec<_> = ladder.iter().map(|(price, _)| price.raw()).collect();

        assert_eq!(prices, vec![255, 128, 127, 64, 63, 0]);
    }

    #[test]
    fn capacity_enforced() {
        assert!(PriceLadder::new(instrument(), Side::Buy, 128).is_err());
    }

    #[test]
    fn clear_preserves_allocation() {
        let mut ladder = PriceLadder::new(instrument(), Side::Buy, 256).unwrap();

        ladder.set_level(Price::new(100), level()).unwrap();
        ladder.clear();

        assert!(ladder.is_empty());
        assert_eq!(ladder.best_price(), None);
        assert_eq!(ladder.capacity(), 256);
    }
}
