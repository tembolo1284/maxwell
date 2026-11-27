//! Order book configuration and resource limits.

use crate::error::{BookError, BookResult};

pub const DEFAULT_MAX_PRICE_LEVELS: usize = 65_536;
pub const DEFAULT_MAX_ORDERS: usize = 65_536;
pub const DEFAULT_MAX_INSTRUMENTS: usize = 64;

pub const MAX_PRICE_LEVELS: usize = 4_194_304;
pub const MAX_ORDERS: usize = 4_194_304;
pub const MAX_INSTRUMENTS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookConfig {
    /// Maximum number of price levels per side.
    pub max_price_levels: usize,

    /// Maximum number of resting orders per instrument.
    pub max_orders: usize,

    /// Maximum number of registered instruments.
    pub max_instruments: usize,
}

impl BookConfig {
    #[must_use]
    pub const fn new(max_price_levels: usize, max_orders: usize, max_instruments: usize) -> Self {
        Self {
            max_price_levels,
            max_orders,
            max_instruments,
        }
    }

    pub const fn validate(self) -> BookResult<()> {
        if self.max_price_levels == 0 || self.max_price_levels > MAX_PRICE_LEVELS {
            return Err(BookError::InvalidPriceLevelCapacity(self.max_price_levels));
        }

        if self.max_orders == 0 || self.max_orders > MAX_ORDERS {
            return Err(BookError::InvalidOrderCapacity(self.max_orders));
        }

        if self.max_instruments == 0 || self.max_instruments > MAX_INSTRUMENTS {
            return Err(BookError::InvalidInstrumentCapacity(self.max_instruments));
        }

        Ok(())
    }
}

impl Default for BookConfig {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_PRICE_LEVELS,
            DEFAULT_MAX_ORDERS,
            DEFAULT_MAX_INSTRUMENTS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_valid() {
        assert!(BookConfig::default().validate().is_ok());
    }

    #[test]
    fn zero_price_levels_rejected() {
        assert!(BookConfig::new(0, 100, 1).validate().is_err());
    }

    #[test]
    fn zero_orders_rejected() {
        assert!(BookConfig::new(100, 0, 1).validate().is_err());
    }

    #[test]
    fn zero_instruments_rejected() {
        assert!(BookConfig::new(100, 100, 0).validate().is_err());
    }

    #[test]
    fn excessive_capacity_rejected() {
        assert!(
            BookConfig::new(MAX_PRICE_LEVELS + 1, 100, 1)
                .validate()
                .is_err()
        );
    }
}
