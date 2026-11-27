//! Instrument metadata and instrument-specific validation.
//!
//! Defines numerical scales, tick sizes, lot sizes, and
//! permissible price and quantity ranges.
//!
//! Instrument metadata is immutable after construction.
//! All numerical values are expressed in atomic units.

use core::fmt;

use crate::{InstrumentId, Price, Quantity};

// ============================================================
// Constants
// ============================================================

/// Maximum supported decimal scale.
pub const MAX_DECIMAL_SCALE: u8 = 18;

// ============================================================
// Instrument errors
// ============================================================

/// Errors associated with instrument configuration and validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentError {
    InvalidPriceScale,
    InvalidQuantityScale,
    InvalidTickSize,
    InvalidLotSize,
    InvalidPriceRange,
    InvalidQuantityRange,
    PriceOutOfRange,
    PriceNotTickAligned,
    QuantityOutOfRange,
    QuantityNotLotAligned,
    ZeroQuantity,
}

impl fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidPriceScale => "invalid price decimal scale",
            Self::InvalidQuantityScale => "invalid quantity decimal scale",
            Self::InvalidTickSize => "tick size must be positive",
            Self::InvalidLotSize => "lot size must be positive",
            Self::InvalidPriceRange => "invalid instrument price range",
            Self::InvalidQuantityRange => "invalid instrument quantity range",
            Self::PriceOutOfRange => "price is outside instrument limits",
            Self::PriceNotTickAligned => "price is not aligned to tick size",
            Self::QuantityOutOfRange => "quantity is outside instrument limits",
            Self::QuantityNotLotAligned => "quantity is not aligned to lot size",
            Self::ZeroQuantity => "order quantity must be positive",
        };

        f.write_str(message)
    }
}

impl std::error::Error for InstrumentError {}

// ============================================================
// Instrument configuration
// ============================================================

/// Unvalidated instrument configuration.
///
/// Configuration is validated when constructing an Instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstrumentConfig {
    pub id: InstrumentId,
    pub price_scale: u8,
    pub quantity_scale: u8,
    pub tick_size: Price,
    pub lot_size: Quantity,
    pub min_price: Price,
    pub max_price: Price,
    pub min_quantity: Quantity,
    pub max_quantity: Quantity,
}

// ============================================================
// Instrument
// ============================================================

/// Validated instrument metadata.
///
/// Price and quantity scales remain constant for the lifetime
/// of this object.
///
/// Tick and lot validation assumes constant increments on
/// zero-origin numerical grids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instrument {
    id: InstrumentId,
    price_scale: u8,
    quantity_scale: u8,
    tick_size: Price,
    lot_size: Quantity,
    min_price: Price,
    max_price: Price,
    min_quantity: Quantity,
    max_quantity: Quantity,
}

impl Instrument {
    // ========================================================
    // Construction
    // ========================================================

    /// Constructs validated instrument metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if any configuration invariant is
    /// violated.
    pub const fn new(config: InstrumentConfig) -> Result<Self, InstrumentError> {
        if config.price_scale > MAX_DECIMAL_SCALE {
            return Err(InstrumentError::InvalidPriceScale);
        }

        if config.quantity_scale > MAX_DECIMAL_SCALE {
            return Err(InstrumentError::InvalidQuantityScale);
        }

        if !config.tick_size.is_positive() {
            return Err(InstrumentError::InvalidTickSize);
        }

        if config.lot_size.is_zero() {
            return Err(InstrumentError::InvalidLotSize);
        }

        if config.min_price.raw() > config.max_price.raw() {
            return Err(InstrumentError::InvalidPriceRange);
        }

        if !config.min_price.is_tick_aligned(config.tick_size.raw())
            || !config.max_price.is_tick_aligned(config.tick_size.raw())
        {
            return Err(InstrumentError::InvalidPriceRange);
        }

        if config.min_quantity.is_zero() || config.min_quantity.raw() > config.max_quantity.raw() {
            return Err(InstrumentError::InvalidQuantityRange);
        }

        if !config.min_quantity.is_lot_aligned(config.lot_size.raw())
            || !config.max_quantity.is_lot_aligned(config.lot_size.raw())
        {
            return Err(InstrumentError::InvalidQuantityRange);
        }

        Ok(Self {
            id: config.id,
            price_scale: config.price_scale,
            quantity_scale: config.quantity_scale,
            tick_size: config.tick_size,
            lot_size: config.lot_size,
            min_price: config.min_price,
            max_price: config.max_price,
            min_quantity: config.min_quantity,
            max_quantity: config.max_quantity,
        })
    }

    // ========================================================
    // Accessors
    // ========================================================

    #[must_use]
    #[inline]
    pub const fn id(&self) -> InstrumentId {
        self.id
    }

    #[must_use]
    #[inline]
    pub const fn price_scale(&self) -> u8 {
        self.price_scale
    }

    #[must_use]
    #[inline]
    pub const fn quantity_scale(&self) -> u8 {
        self.quantity_scale
    }

    #[must_use]
    #[inline]
    pub const fn tick_size(&self) -> Price {
        self.tick_size
    }

    #[must_use]
    #[inline]
    pub const fn lot_size(&self) -> Quantity {
        self.lot_size
    }

    #[must_use]
    #[inline]
    pub const fn min_price(&self) -> Price {
        self.min_price
    }

    #[must_use]
    #[inline]
    pub const fn max_price(&self) -> Price {
        self.max_price
    }

    #[must_use]
    #[inline]
    pub const fn min_quantity(&self) -> Quantity {
        self.min_quantity
    }

    #[must_use]
    #[inline]
    pub const fn max_quantity(&self) -> Quantity {
        self.max_quantity
    }

    // ========================================================
    // Numerical scaling
    // ========================================================

    /// Returns the integer price scaling factor.
    #[must_use]
    #[inline]
    pub const fn price_scale_factor(&self) -> u64 {
        10_u64.pow(self.price_scale as u32)
    }

    /// Returns the integer quantity scaling factor.
    #[must_use]
    #[inline]
    pub const fn quantity_scale_factor(&self) -> u64 {
        10_u64.pow(self.quantity_scale as u32)
    }

    // ========================================================
    // Price validation
    // ========================================================

    /// Returns whether the price is within instrument bounds.
    #[must_use]
    #[inline]
    pub const fn is_price_in_range(&self, price: Price) -> bool {
        price.raw() >= self.min_price.raw() && price.raw() <= self.max_price.raw()
    }

    /// Returns whether the price is aligned to the tick grid.
    #[must_use]
    #[inline]
    pub const fn is_price_tick_aligned(&self, price: Price) -> bool {
        price.is_tick_aligned(self.tick_size.raw())
    }

    /// Validates a price against instrument constraints.
    ///
    /// # Errors
    ///
    /// Returns an error if the price is outside the permitted
    /// range or is not aligned to the tick grid.
    #[inline]
    pub const fn validate_price(&self, price: Price) -> Result<(), InstrumentError> {
        if !self.is_price_in_range(price) {
            return Err(InstrumentError::PriceOutOfRange);
        }

        if !self.is_price_tick_aligned(price) {
            return Err(InstrumentError::PriceNotTickAligned);
        }

        Ok(())
    }

    // ========================================================
    // Quantity validation
    // ========================================================

    /// Returns whether an order quantity is within bounds.
    ///
    /// These bounds do not apply to aggregate book depth or
    /// necessarily to partial executions.
    #[must_use]
    #[inline]
    pub const fn is_quantity_in_range(&self, quantity: Quantity) -> bool {
        quantity.raw() >= self.min_quantity.raw() && quantity.raw() <= self.max_quantity.raw()
    }

    /// Returns whether the quantity is aligned to the lot grid.
    #[must_use]
    #[inline]
    pub const fn is_quantity_lot_aligned(&self, quantity: Quantity) -> bool {
        quantity.is_lot_aligned(self.lot_size.raw())
    }

    /// Validates an original order quantity.
    ///
    /// This method must not be applied indiscriminately to
    /// remaining quantities, executions, or aggregate depth.
    ///
    /// # Errors
    ///
    /// Returns an error for zero quantities, quantities outside
    /// order limits, or quantities not aligned to the lot grid.
    #[inline]
    pub const fn validate_order_quantity(&self, quantity: Quantity) -> Result<(), InstrumentError> {
        if quantity.is_zero() {
            return Err(InstrumentError::ZeroQuantity);
        }

        if !self.is_quantity_in_range(quantity) {
            return Err(InstrumentError::QuantityOutOfRange);
        }

        if !self.is_quantity_lot_aligned(quantity) {
            return Err(InstrumentError::QuantityNotLotAligned);
        }

        Ok(())
    }

    // ========================================================
    // Combined validation
    // ========================================================

    /// Validates the price and quantity of a limit order.
    ///
    /// # Errors
    ///
    /// Returns the first violated price or quantity constraint.
    #[inline]
    pub const fn validate_limit_order(
        &self,
        price: Price,
        quantity: Quantity,
    ) -> Result<(), InstrumentError> {
        match self.validate_price(price) {
            Ok(()) => self.validate_order_quantity(quantity),
            Err(error) => Err(error),
        }
    }

    // ========================================================
    // Price ladder indexing
    // ========================================================

    /// Returns the zero-based tick index relative to min_price.
    ///
    /// Returns None for invalid or misaligned prices.
    ///
    /// Unsigned distance arithmetic supports the complete
    /// signed 64-bit price range without subtraction overflow.
    #[must_use]
    #[inline]
    pub const fn price_to_tick_index(&self, price: Price) -> Option<u64> {
        if !self.is_price_in_range(price) || !self.is_price_tick_aligned(price) {
            return None;
        }

        let distance = price.abs_diff(self.min_price);
        let tick = self.tick_size.raw().unsigned_abs();

        Some(distance / tick)
    }

    /// Converts a zero-based tick index into a price.
    ///
    /// Returns None if the index exceeds the configured range.
    #[must_use]
    #[inline]
    pub fn tick_index_to_price(&self, index: u64) -> Option<Price> {
        let max_index = self.price_to_tick_index(self.max_price)?;

        if index > max_index {
            return None;
        }

        let offset = i128::from(index) * i128::from(self.tick_size.raw());

        let raw = i128::from(self.min_price.raw()) + offset;

        i64::try_from(raw).ok().map(Price::new)
    }

    /// Returns the number of representable price levels.
    ///
    /// Returns None if the number of levels exceeds u64::MAX.
    ///
    /// This is a mathematical count, not a guarantee that a
    /// dense ladder of this size can be allocated.
    #[must_use]
    #[inline]
    pub const fn price_level_count(&self) -> Option<u64> {
        let distance = self.max_price.abs_diff(self.min_price);
        let tick = self.tick_size.raw().unsigned_abs();

        (distance / tick).checked_add(1)
    }
}

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    const fn valid_config() -> InstrumentConfig {
        InstrumentConfig {
            id: InstrumentId::new(1),
            price_scale: 4,
            quantity_scale: 0,
            tick_size: Price::new(5),
            lot_size: Quantity::new(10),
            min_price: Price::new(-100_000),
            max_price: Price::new(100_000),
            min_quantity: Quantity::new(10),
            max_quantity: Quantity::new(1_000_000),
        }
    }

    fn instrument() -> Instrument {
        Instrument::new(valid_config()).unwrap()
    }

    #[test]
    fn valid_construction() {
        let instrument = instrument();

        assert_eq!(instrument.id(), InstrumentId::new(1));
        assert_eq!(instrument.price_scale(), 4);
        assert_eq!(instrument.quantity_scale(), 0);
        assert_eq!(instrument.tick_size(), Price::new(5));
        assert_eq!(instrument.lot_size(), Quantity::new(10));
    }

    #[test]
    fn invalid_price_scale() {
        let mut config = valid_config();
        config.price_scale = MAX_DECIMAL_SCALE + 1;

        assert_eq!(
            Instrument::new(config),
            Err(InstrumentError::InvalidPriceScale),
        );
    }

    #[test]
    fn invalid_quantity_scale() {
        let mut config = valid_config();
        config.quantity_scale = MAX_DECIMAL_SCALE + 1;

        assert_eq!(
            Instrument::new(config),
            Err(InstrumentError::InvalidQuantityScale),
        );
    }

    #[test]
    fn maximum_decimal_scale() {
        let mut config = valid_config();
        config.price_scale = MAX_DECIMAL_SCALE;
        config.quantity_scale = MAX_DECIMAL_SCALE;

        let instrument = Instrument::new(config).unwrap();

        assert_eq!(instrument.price_scale_factor(), 1_000_000_000_000_000_000,);

        assert_eq!(
            instrument.quantity_scale_factor(),
            1_000_000_000_000_000_000,
        );
    }

    #[test]
    fn invalid_tick_size() {
        for tick in [0, -1, i64::MIN] {
            let mut config = valid_config();
            config.tick_size = Price::new(tick);

            assert_eq!(
                Instrument::new(config),
                Err(InstrumentError::InvalidTickSize),
            );
        }
    }

    #[test]
    fn invalid_lot_size() {
        let mut config = valid_config();
        config.lot_size = Quantity::ZERO;

        assert_eq!(
            Instrument::new(config),
            Err(InstrumentError::InvalidLotSize),
        );
    }

    #[test]
    fn inverted_price_range() {
        let mut config = valid_config();
        config.min_price = Price::new(200_000);

        assert_eq!(
            Instrument::new(config),
            Err(InstrumentError::InvalidPriceRange),
        );
    }

    #[test]
    fn misaligned_price_range() {
        let mut config = valid_config();
        config.min_price = Price::new(-100_003);

        assert_eq!(
            Instrument::new(config),
            Err(InstrumentError::InvalidPriceRange),
        );
    }

    #[test]
    fn inverted_quantity_range() {
        let mut config = valid_config();
        config.min_quantity = Quantity::new(2_000_000);

        assert_eq!(
            Instrument::new(config),
            Err(InstrumentError::InvalidQuantityRange),
        );
    }

    #[test]
    fn zero_minimum_quantity() {
        let mut config = valid_config();
        config.min_quantity = Quantity::ZERO;

        assert_eq!(
            Instrument::new(config),
            Err(InstrumentError::InvalidQuantityRange),
        );
    }

    #[test]
    fn misaligned_quantity_range() {
        let mut config = valid_config();
        config.max_quantity = Quantity::new(1_000_003);

        assert_eq!(
            Instrument::new(config),
            Err(InstrumentError::InvalidQuantityRange),
        );
    }

    #[test]
    fn scale_factors() {
        let instrument = instrument();

        assert_eq!(instrument.price_scale_factor(), 10_000);
        assert_eq!(instrument.quantity_scale_factor(), 1);
    }

    #[test]
    fn valid_prices() {
        let instrument = instrument();

        for raw in [-100_000, -50, 0, 50, 100_000] {
            assert_eq!(instrument.validate_price(Price::new(raw)), Ok(()),);
        }
    }

    #[test]
    fn price_out_of_range() {
        let instrument = instrument();

        assert_eq!(
            instrument.validate_price(Price::new(100_005)),
            Err(InstrumentError::PriceOutOfRange),
        );

        assert_eq!(
            instrument.validate_price(Price::new(-100_005)),
            Err(InstrumentError::PriceOutOfRange),
        );
    }

    #[test]
    fn price_not_tick_aligned() {
        let instrument = instrument();

        for raw in [-103, 103] {
            assert_eq!(
                instrument.validate_price(Price::new(raw)),
                Err(InstrumentError::PriceNotTickAligned),
            );
        }
    }

    #[test]
    fn valid_order_quantities() {
        let instrument = instrument();

        for raw in [10, 100, 1_000_000] {
            assert_eq!(
                instrument.validate_order_quantity(Quantity::new(raw),),
                Ok(()),
            );
        }
    }

    #[test]
    fn zero_order_quantity() {
        assert_eq!(
            instrument().validate_order_quantity(Quantity::ZERO,),
            Err(InstrumentError::ZeroQuantity),
        );
    }

    #[test]
    fn quantity_out_of_range() {
        let instrument = instrument();

        assert_eq!(
            instrument.validate_order_quantity(Quantity::new(5),),
            Err(InstrumentError::QuantityOutOfRange),
        );

        assert_eq!(
            instrument.validate_order_quantity(Quantity::new(1_000_010),),
            Err(InstrumentError::QuantityOutOfRange),
        );
    }

    #[test]
    fn quantity_not_lot_aligned() {
        assert_eq!(
            instrument().validate_order_quantity(Quantity::new(103),),
            Err(InstrumentError::QuantityNotLotAligned),
        );
    }

    #[test]
    fn valid_limit_order() {
        assert_eq!(
            instrument().validate_limit_order(Price::new(100), Quantity::new(500),),
            Ok(()),
        );
    }

    #[test]
    fn invalid_limit_order_price() {
        assert_eq!(
            instrument().validate_limit_order(Price::new(103), Quantity::new(500),),
            Err(InstrumentError::PriceNotTickAligned),
        );
    }

    #[test]
    fn invalid_limit_order_quantity() {
        assert_eq!(
            instrument().validate_limit_order(Price::new(100), Quantity::ZERO,),
            Err(InstrumentError::ZeroQuantity),
        );
    }

    #[test]
    fn price_to_tick_index() {
        let instrument = instrument();

        assert_eq!(
            instrument.price_to_tick_index(Price::new(-100_000)),
            Some(0),
        );

        assert_eq!(instrument.price_to_tick_index(Price::ZERO), Some(20_000),);

        assert_eq!(
            instrument.price_to_tick_index(Price::new(100_000)),
            Some(40_000),
        );
    }

    #[test]
    fn invalid_price_to_tick_index() {
        let instrument = instrument();

        assert_eq!(instrument.price_to_tick_index(Price::new(103)), None,);

        assert_eq!(instrument.price_to_tick_index(Price::new(200_000)), None,);
    }

    #[test]
    fn tick_index_to_price() {
        let instrument = instrument();

        assert_eq!(
            instrument.tick_index_to_price(0),
            Some(Price::new(-100_000)),
        );

        assert_eq!(instrument.tick_index_to_price(20_000), Some(Price::ZERO),);

        assert_eq!(
            instrument.tick_index_to_price(40_000),
            Some(Price::new(100_000)),
        );
    }

    #[test]
    fn invalid_tick_index() {
        let instrument = instrument();

        assert_eq!(instrument.tick_index_to_price(40_001), None,);

        assert_eq!(instrument.tick_index_to_price(u64::MAX), None,);
    }

    #[test]
    fn tick_index_roundtrip() {
        let instrument = instrument();

        for index in 0..40_001 {
            let price = instrument.tick_index_to_price(index).unwrap();

            assert_eq!(instrument.price_to_tick_index(price), Some(index),);
        }
    }

    #[test]
    fn price_level_count() {
        assert_eq!(instrument().price_level_count(), Some(40_001),);
    }

    #[test]
    fn single_price_level() {
        let mut config = valid_config();

        config.min_price = Price::new(100);
        config.max_price = Price::new(100);

        let instrument = Instrument::new(config).unwrap();

        assert_eq!(instrument.price_level_count(), Some(1));
        assert_eq!(instrument.tick_index_to_price(0), Some(Price::new(100)),);
    }

    #[test]
    fn full_signed_price_range() {
        let mut config = valid_config();

        config.min_price = Price::MIN;
        config.max_price = Price::MAX;
        config.tick_size = Price::new(1);

        let instrument = Instrument::new(config).unwrap();

        assert_eq!(instrument.price_to_tick_index(Price::MIN), Some(0),);

        assert_eq!(instrument.price_to_tick_index(Price::MAX), Some(u64::MAX),);

        assert_eq!(instrument.tick_index_to_price(u64::MAX), Some(Price::MAX),);

        assert_eq!(instrument.price_level_count(), None);
    }

    #[test]
    fn error_display() {
        assert_eq!(
            InstrumentError::InvalidTickSize.to_string(),
            "tick size must be positive",
        );
    }

    #[test]
    fn const_construction() {
        const RESULT: Result<Instrument, InstrumentError> = Instrument::new(valid_config());

        assert!(RESULT.is_ok());
    }
}
