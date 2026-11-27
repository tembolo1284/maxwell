//! Maxwell native binary protocol V1.
//!
//! Wire values are explicitly mapped to domain values.
//! No transmutation, pointer casting, packed structs,
//! or native-endian serialization is used.

use maxwell_core::{
    BookClearScope, BookCleared, MarketEvent, OrderAdded, OrderCancelled, OrderDeleted,
    OrderExecuted, OrderId, OrderModified, Price, PriceLevelUpdated, Quantity, Side, Trade,
    TradeId, TradingStatus, TradingStatusChanged,
};

use crate::error::{ProtocolError, ProtocolResult};
use crate::message::MessageKind;
use crate::reader::ByteReader;
use crate::writer::ByteWriter;

// ============================================================
// Side representation
// ============================================================

const SIDE_BUY: u8 = 0;
const SIDE_SELL: u8 = 1;
const SIDE_ABSENT: u8 = 255;

#[inline]
const fn encode_side(side: Side) -> u8 {
    match side {
        Side::Buy => SIDE_BUY,
        Side::Sell => SIDE_SELL,
    }
}

#[inline]
const fn decode_side(value: u8) -> ProtocolResult<Side> {
    match value {
        SIDE_BUY => Ok(Side::Buy),
        SIDE_SELL => Ok(Side::Sell),
        _ => Err(ProtocolError::InvalidSide(value)),
    }
}

#[inline]
const fn encode_optional_side(side: Option<Side>) -> u8 {
    match side {
        Some(value) => encode_side(value),
        None => SIDE_ABSENT,
    }
}

#[inline]
fn decode_optional_side(value: u8) -> ProtocolResult<Option<Side>> {
    if value == SIDE_ABSENT {
        Ok(None)
    } else {
        decode_side(value).map(Some)
    }
}

// ============================================================
// Trading status representation
// ============================================================

#[inline]
const fn encode_status(status: TradingStatus) -> u8 {
    match status {
        TradingStatus::Unknown => 0,
        TradingStatus::PreOpen => 1,
        TradingStatus::Open => 2,
        TradingStatus::Halted => 3,
        TradingStatus::Auction => 4,
        TradingStatus::Closed => 5,
    }
}

#[inline]
const fn decode_status(value: u8) -> ProtocolResult<TradingStatus> {
    match value {
        0 => Ok(TradingStatus::Unknown),
        1 => Ok(TradingStatus::PreOpen),
        2 => Ok(TradingStatus::Open),
        3 => Ok(TradingStatus::Halted),
        4 => Ok(TradingStatus::Auction),
        5 => Ok(TradingStatus::Closed),
        _ => Err(ProtocolError::InvalidTradingStatus(value)),
    }
}

// ============================================================
// Book clear scope representation
// ============================================================

#[inline]
const fn encode_scope(scope: BookClearScope) -> u8 {
    match scope {
        BookClearScope::BothSides => 0,
        BookClearScope::BuySide => 1,
        BookClearScope::SellSide => 2,
    }
}

#[inline]
const fn decode_scope(value: u8) -> ProtocolResult<BookClearScope> {
    match value {
        0 => Ok(BookClearScope::BothSides),
        1 => Ok(BookClearScope::BuySide),
        2 => Ok(BookClearScope::SellSide),
        _ => Err(ProtocolError::InvalidClearScope(value)),
    }
}

// ============================================================
// Optional order count representation
// ============================================================
//
// One presence byte followed by a four-byte count.
//
// Absent:  presence = 0, count = 0
// Present: presence = 1, count = supplied value

#[inline]
fn encode_order_count(count: Option<u32>, writer: &mut ByteWriter<'_>) -> ProtocolResult<()> {
    match count {
        Some(value) => {
            writer.write_u8(1)?;
            writer.write_u32(value)?;
        }
        None => {
            writer.write_u8(0)?;
            writer.write_u32(0)?;
        }
    }

    Ok(())
}

#[inline]
fn decode_order_count(reader: &mut ByteReader<'_>) -> ProtocolResult<Option<u32>> {
    let present = reader.read_u8()?;
    let value = reader.read_u32()?;

    match present {
        0 if value == 0 => Ok(None),
        0 => Err(ProtocolError::NonCanonicalEncoding),
        1 => Ok(Some(value)),
        _ => Err(ProtocolError::InvalidPresenceFlag(present)),
    }
}

// ============================================================
// Payload decoding
// ============================================================

#[inline]
pub fn decode_payload(
    kind: MessageKind,
    reader: &mut ByteReader<'_>,
) -> ProtocolResult<MarketEvent> {
    let mut cursor = reader.clone();

    let event = match kind {
        MessageKind::OrderAdded => MarketEvent::OrderAdded(OrderAdded {
            order_id: OrderId::new(cursor.read_u64()?),
            price: Price::new(cursor.read_i64()?),
            quantity: Quantity::new(cursor.read_u64()?),
            side: decode_side(cursor.read_u8()?)?,
        }),

        MessageKind::OrderModified => MarketEvent::OrderModified(OrderModified {
            order_id: OrderId::new(cursor.read_u64()?),
            new_price: Price::new(cursor.read_i64()?),
            new_quantity: Quantity::new(cursor.read_u64()?),
        }),

        MessageKind::OrderCancelled => MarketEvent::OrderCancelled(OrderCancelled {
            order_id: OrderId::new(cursor.read_u64()?),
            cancelled_quantity: Quantity::new(cursor.read_u64()?),
        }),

        MessageKind::OrderDeleted => MarketEvent::OrderDeleted(OrderDeleted {
            order_id: OrderId::new(cursor.read_u64()?),
        }),

        MessageKind::OrderExecuted => MarketEvent::OrderExecuted(OrderExecuted {
            order_id: OrderId::new(cursor.read_u64()?),
            trade_id: TradeId::new(cursor.read_u64()?),
            execution_price: Price::new(cursor.read_i64()?),
            executed_quantity: Quantity::new(cursor.read_u64()?),
        }),

        MessageKind::Trade => MarketEvent::Trade(Trade {
            trade_id: TradeId::new(cursor.read_u64()?),
            price: Price::new(cursor.read_i64()?),
            quantity: Quantity::new(cursor.read_u64()?),
            aggressor_side: decode_optional_side(cursor.read_u8()?)?,
        }),

        MessageKind::PriceLevelUpdated => {
            let price = Price::new(cursor.read_i64()?);
            let quantity = Quantity::new(cursor.read_u64()?);
            let side = decode_side(cursor.read_u8()?)?;
            let order_count = decode_order_count(&mut cursor)?;

            MarketEvent::PriceLevelUpdated(PriceLevelUpdated {
                price,
                quantity,
                order_count,
                side,
            })
        }

        MessageKind::BookCleared => MarketEvent::BookCleared(BookCleared {
            scope: decode_scope(cursor.read_u8()?)?,
        }),

        MessageKind::TradingStatusChanged => {
            MarketEvent::TradingStatusChanged(TradingStatusChanged {
                status: decode_status(cursor.read_u8()?)?,
            })
        }
    };

    event.validate()?;

    *reader = cursor;

    Ok(event)
}

// ============================================================
// Payload encoding
// ============================================================

#[inline]
pub fn encode_payload(event: MarketEvent, writer: &mut ByteWriter<'_>) -> ProtocolResult<()> {
    event.validate()?;

    let kind = MessageKind::from(event.kind());

    writer.ensure_capacity(kind.payload_size())?;

    match event {
        MarketEvent::OrderAdded(value) => {
            writer.write_u64(value.order_id.raw())?;
            writer.write_i64(value.price.raw())?;
            writer.write_u64(value.quantity.raw())?;
            writer.write_u8(encode_side(value.side))?;
        }

        MarketEvent::OrderModified(value) => {
            writer.write_u64(value.order_id.raw())?;
            writer.write_i64(value.new_price.raw())?;
            writer.write_u64(value.new_quantity.raw())?;
        }

        MarketEvent::OrderCancelled(value) => {
            writer.write_u64(value.order_id.raw())?;
            writer.write_u64(value.cancelled_quantity.raw())?;
        }

        MarketEvent::OrderDeleted(value) => {
            writer.write_u64(value.order_id.raw())?;
        }

        MarketEvent::OrderExecuted(value) => {
            writer.write_u64(value.order_id.raw())?;
            writer.write_u64(value.trade_id.raw())?;
            writer.write_i64(value.execution_price.raw())?;
            writer.write_u64(value.executed_quantity.raw())?;
        }

        MarketEvent::Trade(value) => {
            writer.write_u64(value.trade_id.raw())?;
            writer.write_i64(value.price.raw())?;
            writer.write_u64(value.quantity.raw())?;
            writer.write_u8(encode_optional_side(value.aggressor_side))?;
        }

        MarketEvent::PriceLevelUpdated(value) => {
            writer.write_i64(value.price.raw())?;
            writer.write_u64(value.quantity.raw())?;
            writer.write_u8(encode_side(value.side))?;
            encode_order_count(value.order_count, writer)?;
        }

        MarketEvent::BookCleared(value) => {
            writer.write_u8(encode_scope(value.scope))?;
        }

        MarketEvent::TradingStatusChanged(value) => {
            writer.write_u8(encode_status(value.status))?;
        }
    }

    Ok(())
}

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_roundtrip() {
        for side in [Side::Buy, Side::Sell] {
            assert_eq!(decode_side(encode_side(side)), Ok(side));
        }
    }

    #[test]
    fn invalid_side() {
        assert_eq!(decode_side(10), Err(ProtocolError::InvalidSide(10)));
    }

    #[test]
    fn optional_side_roundtrip() {
        for side in [None, Some(Side::Buy), Some(Side::Sell)] {
            assert_eq!(decode_optional_side(encode_optional_side(side)), Ok(side));
        }
    }

    #[test]
    fn status_roundtrip() {
        for status in [
            TradingStatus::Unknown,
            TradingStatus::PreOpen,
            TradingStatus::Open,
            TradingStatus::Halted,
            TradingStatus::Auction,
            TradingStatus::Closed,
        ] {
            assert_eq!(decode_status(encode_status(status)), Ok(status));
        }
    }

    #[test]
    fn scope_roundtrip() {
        for scope in [
            BookClearScope::BothSides,
            BookClearScope::BuySide,
            BookClearScope::SellSide,
        ] {
            assert_eq!(decode_scope(encode_scope(scope)), Ok(scope));
        }
    }

    #[test]
    fn order_count_roundtrip() {
        for count in [None, Some(0), Some(10), Some(u32::MAX)] {
            let mut buffer = [0_u8; 5];
            let mut writer = ByteWriter::new(&mut buffer);

            encode_order_count(count, &mut writer).unwrap();

            let mut reader = ByteReader::new(writer.written());

            assert_eq!(decode_order_count(&mut reader), Ok(count));
        }
    }

    #[test]
    fn noncanonical_order_count() {
        let mut reader = ByteReader::new(&[0, 1, 0, 0, 0]);

        assert_eq!(
            decode_order_count(&mut reader),
            Err(ProtocolError::NonCanonicalEncoding)
        );
    }

    #[test]
    fn invalid_presence_flag() {
        let mut reader = ByteReader::new(&[2, 0, 0, 0, 0]);

        assert_eq!(
            decode_order_count(&mut reader),
            Err(ProtocolError::InvalidPresenceFlag(2))
        );
    }

    #[test]
    fn failed_payload_decode_preserves_cursor() {
        let mut reader = ByteReader::new(&[0; 2]);

        assert!(decode_payload(MessageKind::OrderAdded, &mut reader).is_err());

        assert_eq!(reader.position(), 0);
    }
}
