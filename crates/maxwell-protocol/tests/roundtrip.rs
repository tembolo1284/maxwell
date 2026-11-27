//! End-to-end encoding and decoding tests.

use maxwell_core::*;

use maxwell_protocol::{ByteWriter, Decoder, Encoder, ProtocolMessage};

const fn envelope(event: MarketEvent) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(
                FeedId::new(StreamId::new(1), SessionId::new(100)),
                SequenceNumber::new(500),
            ),
            InstrumentId::new(42),
            Some(ExchangeTimestamp::new(1_700_000_000_123_456_789)),
        ),
        event,
    )
}

fn roundtrip(event: MarketEvent) {
    let original = envelope(event);
    let mut buffer = [0_u8; 128];

    let encoder = Encoder::new();
    let decoder = Decoder::new();

    let written = encoder.encode_event(original, &mut buffer).unwrap();

    let roundtrip = decoder.decode_event(&buffer[..written]).unwrap();

    assert_eq!(original, roundtrip);
}

const fn order_added() -> MarketEvent {
    MarketEvent::OrderAdded(OrderAdded {
        order_id: OrderId::new(1_001),
        price: Price::new(10_000),
        quantity: Quantity::new(500),
        side: Side::Buy,
    })
}

// ============================================================
// Order events
// ============================================================

#[test]
fn order_added_roundtrip() {
    roundtrip(order_added());
}

#[test]
fn sell_order_roundtrip() {
    roundtrip(MarketEvent::OrderAdded(OrderAdded {
        order_id: OrderId::new(1),
        price: Price::new(100),
        quantity: Quantity::new(50),
        side: Side::Sell,
    }));
}

#[test]
fn order_modified_roundtrip() {
    roundtrip(MarketEvent::OrderModified(OrderModified {
        order_id: OrderId::new(1_001),
        new_price: Price::new(10_005),
        new_quantity: Quantity::new(400),
    }));
}

#[test]
fn order_cancelled_roundtrip() {
    roundtrip(MarketEvent::OrderCancelled(OrderCancelled {
        order_id: OrderId::new(1_001),
        cancelled_quantity: Quantity::new(100),
    }));
}

#[test]
fn order_deleted_roundtrip() {
    roundtrip(MarketEvent::OrderDeleted(OrderDeleted {
        order_id: OrderId::new(1_001),
    }));
}

#[test]
fn order_executed_roundtrip() {
    roundtrip(MarketEvent::OrderExecuted(OrderExecuted {
        order_id: OrderId::new(1_001),
        trade_id: TradeId::new(2_001),
        execution_price: Price::new(10_000),
        executed_quantity: Quantity::new(100),
    }));
}

// ============================================================
// Trade events
// ============================================================

#[test]
fn trade_roundtrip() {
    roundtrip(MarketEvent::Trade(Trade {
        trade_id: TradeId::new(2_001),
        price: Price::new(10_000),
        quantity: Quantity::new(100),
        aggressor_side: Some(Side::Sell),
    }));
}

#[test]
fn trade_without_aggressor_roundtrip() {
    roundtrip(MarketEvent::Trade(Trade {
        trade_id: TradeId::new(2_001),
        price: Price::new(10_000),
        quantity: Quantity::new(100),
        aggressor_side: None,
    }));
}

// ============================================================
// Price level events
// ============================================================

#[test]
fn price_level_roundtrip() {
    roundtrip(MarketEvent::PriceLevelUpdated(PriceLevelUpdated {
        price: Price::new(10_000),
        quantity: Quantity::new(500),
        order_count: Some(5),
        side: Side::Buy,
    }));
}

#[test]
fn price_level_without_count_roundtrip() {
    roundtrip(MarketEvent::PriceLevelUpdated(PriceLevelUpdated {
        price: Price::new(10_000),
        quantity: Quantity::new(500),
        order_count: None,
        side: Side::Sell,
    }));
}

#[test]
fn price_level_removal_roundtrip() {
    roundtrip(MarketEvent::PriceLevelUpdated(PriceLevelUpdated {
        price: Price::new(10_000),
        quantity: Quantity::ZERO,
        order_count: Some(0),
        side: Side::Buy,
    }));
}

// ============================================================
// Book and status events
// ============================================================

#[test]
fn book_clear_roundtrip() {
    for scope in [
        BookClearScope::BothSides,
        BookClearScope::BuySide,
        BookClearScope::SellSide,
    ] {
        roundtrip(MarketEvent::BookCleared(BookCleared { scope }));
    }
}

#[test]
fn trading_status_roundtrip() {
    for status in [
        TradingStatus::Unknown,
        TradingStatus::PreOpen,
        TradingStatus::Open,
        TradingStatus::Halted,
        TradingStatus::Auction,
        TradingStatus::Closed,
    ] {
        roundtrip(MarketEvent::TradingStatusChanged(TradingStatusChanged {
            status,
        }));
    }
}

// ============================================================
// Boundary values
// ============================================================

#[test]
fn negative_price_roundtrip() {
    roundtrip(MarketEvent::OrderAdded(OrderAdded {
        order_id: OrderId::new(1),
        price: Price::new(-10_000),
        quantity: Quantity::new(50),
        side: Side::Buy,
    }));
}

#[test]
fn maximum_values_roundtrip() {
    roundtrip(MarketEvent::OrderAdded(OrderAdded {
        order_id: OrderId::new(u64::MAX),
        price: Price::new(i64::MAX),
        quantity: Quantity::new(u64::MAX),
        side: Side::Sell,
    }));
}

#[test]
fn minimum_price_roundtrip() {
    roundtrip(MarketEvent::OrderAdded(OrderAdded {
        order_id: OrderId::new(1),
        price: Price::new(i64::MIN),
        quantity: Quantity::new(1),
        side: Side::Buy,
    }));
}

#[test]
fn missing_exchange_timestamp() {
    let mut original = envelope(order_added());
    original.header.exchange_timestamp = None;

    let mut buffer = [0_u8; 128];

    let written = Encoder::new().encode_event(original, &mut buffer).unwrap();

    assert_eq!(
        Decoder::new().decode_event(&buffer[..written]),
        Ok(original)
    );
}

#[test]
fn zero_timestamp_is_not_absent() {
    let mut original = envelope(order_added());

    original.header.exchange_timestamp = Some(ExchangeTimestamp::ZERO);

    let mut buffer = [0_u8; 128];

    let written = Encoder::new().encode_event(original, &mut buffer).unwrap();

    assert_eq!(
        Decoder::new().decode_event(&buffer[..written]),
        Ok(original)
    );
}

// ============================================================
// Concatenated messages
// ============================================================

#[test]
fn multiple_frames_roundtrip() {
    let first = ProtocolMessage::current(envelope(order_added()));

    let mut second = ProtocolMessage::current(envelope(MarketEvent::OrderDeleted(OrderDeleted {
        order_id: OrderId::new(1_001),
    })));

    second.envelope.header.id.sequence = SequenceNumber::new(501);

    let mut buffer = [0_u8; 256];

    let encoder = Encoder::new();
    let mut writer = ByteWriter::new(&mut buffer);

    encoder.encode_into(first, &mut writer).unwrap();
    encoder.encode_into(second, &mut writer).unwrap();

    let decoder = Decoder::new();

    let frames: Vec<_> = decoder
        .frames(writer.written())
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(frames, vec![first, second]);
}

#[test]
fn prefix_decoding() {
    let original = ProtocolMessage::current(envelope(order_added()));

    let mut buffer = [0_u8; 256];

    let written = Encoder::new().encode(original, &mut buffer).unwrap();

    let (decoded, consumed) = Decoder::new().decode_prefix(&buffer).unwrap();

    assert_eq!(decoded, original);
    assert_eq!(consumed, written);
}

#[test]
fn deterministic_encoding() {
    let original = envelope(order_added());

    let mut first = [0_u8; 128];
    let mut second = [0xFF_u8; 128];

    let encoder = Encoder::new();

    let first_size = encoder.encode_event(original, &mut first).unwrap();

    let second_size = encoder.encode_event(original, &mut second).unwrap();

    assert_eq!(first_size, second_size);

    assert_eq!(&first[..first_size], &second[..second_size]);
}
