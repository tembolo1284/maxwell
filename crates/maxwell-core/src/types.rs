//! Strongly typed identifiers and common domain enumerations.
//!
//! Identifiers use transparent wrappers around primitive integer
//! types to provide compile-time type safety without additional
//! storage overhead.

use core::fmt;

// ============================================================
// Identifier generation
// ============================================================

/// Defines a strongly typed integer identifier.
///
/// Identifiers are opaque domain values. Their interpretation
/// belongs to the component responsible for assigning them.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $inner:ty) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
        )]
        pub struct $name($inner);

        impl $name {
            /// Constructs an identifier from its underlying value.
            #[must_use]
            #[inline]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// Returns the underlying integer value.
            #[must_use]
            #[inline]
            pub const fn raw(self) -> $inner {
                self.0
            }
        }

        impl From<$name> for $inner {
            #[inline]
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(
                &self,
                f: &mut fmt::Formatter<'_>,
            ) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

// ============================================================
// Domain identifiers
// ============================================================

define_id!(
    /// Identifies a financial instrument.
    InstrumentId, u32
);

define_id!(
    /// Identifies an order within its exchange-defined namespace.
    OrderId, u64
);

define_id!(
    /// Identifies an executed trade.
    TradeId, u64
);

define_id!(
    /// Identifies a trading participant.
    ParticipantId, u32
);

define_id!(
    /// Identifies a logical market data stream.
    StreamId, u32
);

// ============================================================
// Order side
// ============================================================

/// Represents the side of an order or book entry.
///
/// Discriminants are internal domain values and must not be
/// interpreted as external wire protocol values.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy = 0,
    Sell = 1,
}

impl Side {
    /// Returns the opposite side.
    #[must_use]
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }

    /// Returns whether this is the buy side.
    #[must_use]
    #[inline]
    pub const fn is_buy(self) -> bool {
        matches!(self, Self::Buy)
    }

    /// Returns whether this is the sell side.
    #[must_use]
    #[inline]
    pub const fn is_sell(self) -> bool {
        matches!(self, Self::Sell)
    }

    /// Returns the side as an array index.
    ///
    /// Buy maps to index 0 and Sell maps to index 1.
    #[must_use]
    #[inline]
    pub const fn as_index(self) -> usize {
        self as usize
    }

    /// Returns the human-readable side name.
    #[must_use]
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================
// Compile-time layout guarantees
// ============================================================

const _: () = {
    assert!(size_of::<InstrumentId>() == size_of::<u32>());
    assert!(size_of::<OrderId>() == size_of::<u64>());
    assert!(size_of::<TradeId>() == size_of::<u64>());
    assert!(size_of::<ParticipantId>() == size_of::<u32>());
    assert!(size_of::<StreamId>() == size_of::<u32>());
    assert!(size_of::<Side>() == size_of::<u8>());

    assert!(align_of::<InstrumentId>() == align_of::<u32>());
    assert!(align_of::<OrderId>() == align_of::<u64>());
    assert!(align_of::<TradeId>() == align_of::<u64>());
    assert!(align_of::<ParticipantId>() == align_of::<u32>());
    assert!(align_of::<StreamId>() == align_of::<u32>());
};

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn instrument_id_roundtrip() {
        let id = InstrumentId::new(42);

        assert_eq!(id.raw(), 42);
        assert_eq!(u32::from(id), 42);
    }

    #[test]
    fn order_id_roundtrip() {
        let id = OrderId::new(u64::MAX);

        assert_eq!(id.raw(), u64::MAX);
        assert_eq!(u64::from(id), u64::MAX);
    }

    #[test]
    fn trade_id_roundtrip() {
        let id = TradeId::new(1_000);

        assert_eq!(id.raw(), 1_000);
    }

    #[test]
    fn participant_id_roundtrip() {
        let id = ParticipantId::new(100);

        assert_eq!(id.raw(), 100);
    }

    #[test]
    fn stream_id_roundtrip() {
        let id = StreamId::new(5);

        assert_eq!(id.raw(), 5);
    }

    #[test]
    fn identifier_boundary_values() {
        assert_eq!(InstrumentId::new(0).raw(), 0);
        assert_eq!(InstrumentId::new(u32::MAX).raw(), u32::MAX,);

        assert_eq!(OrderId::new(0).raw(), 0);
        assert_eq!(OrderId::new(u64::MAX).raw(), u64::MAX);
    }

    #[test]
    fn identifier_equality() {
        assert_eq!(OrderId::new(10), OrderId::new(10));
        assert_ne!(OrderId::new(10), OrderId::new(20));
    }

    #[test]
    fn identifier_ordering() {
        assert!(OrderId::new(10) < OrderId::new(20));
        assert!(InstrumentId::new(2) > InstrumentId::new(1));
    }

    #[test]
    fn identifier_hashing() {
        let mut ids = HashSet::new();

        assert!(ids.insert(OrderId::new(10)));
        assert!(!ids.insert(OrderId::new(10)));
        assert!(ids.insert(OrderId::new(20)));

        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn identifier_display() {
        assert_eq!(InstrumentId::new(42).to_string(), "42");
        assert_eq!(OrderId::new(123).to_string(), "123");
    }

    #[test]
    fn side_opposite() {
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite(), Side::Buy);
    }

    #[test]
    fn side_double_opposite() {
        assert_eq!(Side::Buy.opposite().opposite(), Side::Buy);
        assert_eq!(Side::Sell.opposite().opposite(), Side::Sell);
    }

    #[test]
    fn side_predicates() {
        assert!(Side::Buy.is_buy());
        assert!(!Side::Buy.is_sell());

        assert!(Side::Sell.is_sell());
        assert!(!Side::Sell.is_buy());
    }

    #[test]
    fn side_array_index() {
        let values = [100, 200];

        assert_eq!(values[Side::Buy.as_index()], 100);
        assert_eq!(values[Side::Sell.as_index()], 200);
    }

    #[test]
    fn side_display() {
        assert_eq!(Side::Buy.to_string(), "BUY");
        assert_eq!(Side::Sell.to_string(), "SELL");
    }

    #[test]
    fn const_construction() {
        const INSTRUMENT: InstrumentId = InstrumentId::new(42);
        const ORDER: OrderId = OrderId::new(100);
        const SIDE: Side = Side::Buy.opposite();

        assert_eq!(INSTRUMENT.raw(), 42);
        assert_eq!(ORDER.raw(), 100);
        assert_eq!(SIDE, Side::Sell);
    }
}
