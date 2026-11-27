//! Golden wire-format compatibility tests.
//!
//! Golden fixtures are independent of the encoder and
//! protect the Maxwell V1 format against accidental changes.
//!
//! These fixtures describe Maxwell's native protocol.
//! Actual Planck-generated fixtures must be added separately.

use maxwell_core::*;

use maxwell_protocol::{Decoder, Encoder, MessageKind, ProtocolVersion};

use maxwell_protocol::constants::*;

// ============================================================
// Golden order-added fixture
// ============================================================
//
// Version:       1.0
// Message kind:  OrderAdded
// Stream:        1
// Session:       100
// Sequence:      500
// Instrument:    42
// Timestamp:     1,000,000 ns
// Order:         1,001
// Price:         10,000
// Quantity:      500
// Side:          Buy

const GOLDEN_ORDER_ADDED: [u8; 65] = [
    // Magic
    0x4D, 0x58, // Major, minor
    0x01, 0x00, // Message kind
    0x01, // Flags
    0x01, // Payload length: 25
    0x19, 0x00, // Stream identifier: 1
    0x01, 0x00, 0x00, 0x00, // Session identifier: 100
    0x64, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Sequence number: 500
    0xF4, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Instrument identifier: 42
    0x2A, 0x00, 0x00, 0x00, // Exchange timestamp: 1,000,000
    0x40, 0x42, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, // Order identifier: 1,001
    0xE9, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Price: 10,000
    0x10, 0x27, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Quantity: 500
    0xF4, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Side: Buy
    0x00,
];

fn expected_order_added() -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(
                FeedId::new(StreamId::new(1), SessionId::new(100)),
                SequenceNumber::new(500),
            ),
            InstrumentId::new(42),
            Some(ExchangeTimestamp::new(1_000_000)),
        ),
        MarketEvent::OrderAdded(OrderAdded {
            order_id: OrderId::new(1_001),
            price: Price::new(10_000),
            quantity: Quantity::new(500),
            side: Side::Buy,
        }),
    )
}

// ============================================================
// Golden order-deleted fixture
// ============================================================

const GOLDEN_ORDER_DELETED: [u8; 48] = [
    // Magic, version, kind, flags, payload length
    0x4D, 0x58, 0x01, 0x00, 0x04, 0x00, 0x08, 0x00, // Stream identifier: 1
    0x01, 0x00, 0x00, 0x00, // Session identifier: 100
    0x64, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Sequence number: 501
    0xF5, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Instrument identifier: 42
    0x2A, 0x00, 0x00, 0x00, // Absent exchange timestamp
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Order identifier: 1,001
    0xE9, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

fn expected_order_deleted() -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(
                FeedId::new(StreamId::new(1), SessionId::new(100)),
                SequenceNumber::new(501),
            ),
            InstrumentId::new(42),
            None,
        ),
        MarketEvent::OrderDeleted(OrderDeleted {
            order_id: OrderId::new(1_001),
        }),
    )
}

// ============================================================
// Golden decoding tests
// ============================================================

#[test]
fn decode_golden_order_added() {
    let decoded = Decoder::new().decode(&GOLDEN_ORDER_ADDED).unwrap();

    assert_eq!(decoded.version, ProtocolVersion::V1);
    assert_eq!(decoded.envelope, expected_order_added());
}

#[test]
fn decode_golden_order_deleted() {
    let decoded = Decoder::new().decode_event(&GOLDEN_ORDER_DELETED).unwrap();

    assert_eq!(decoded, expected_order_deleted());
}

// ============================================================
// Golden encoding tests
// ============================================================

#[test]
fn encode_golden_order_added() {
    let mut buffer = [0_u8; 128];

    let written = Encoder::new()
        .encode_event(expected_order_added(), &mut buffer)
        .unwrap();

    assert_eq!(written, GOLDEN_ORDER_ADDED.len());

    assert_eq!(&buffer[..written], &GOLDEN_ORDER_ADDED);
}

#[test]
fn encode_golden_order_deleted() {
    let mut buffer = [0_u8; 128];

    let written = Encoder::new()
        .encode_event(expected_order_deleted(), &mut buffer)
        .unwrap();

    assert_eq!(&buffer[..written], &GOLDEN_ORDER_DELETED);
}

// ============================================================
// Wire contract stability
// ============================================================

#[test]
fn stable_message_codes() {
    let cases = [
        (MessageKind::OrderAdded, 1),
        (MessageKind::OrderModified, 2),
        (MessageKind::OrderCancelled, 3),
        (MessageKind::OrderDeleted, 4),
        (MessageKind::OrderExecuted, 5),
        (MessageKind::Trade, 6),
        (MessageKind::PriceLevelUpdated, 7),
        (MessageKind::BookCleared, 8),
        (MessageKind::TradingStatusChanged, 9),
    ];

    for (kind, expected) in cases {
        assert_eq!(kind.code(), expected);
    }
}

#[test]
fn stable_payload_sizes() {
    let cases = [
        (MessageKind::OrderAdded, 25),
        (MessageKind::OrderModified, 24),
        (MessageKind::OrderCancelled, 16),
        (MessageKind::OrderDeleted, 8),
        (MessageKind::OrderExecuted, 32),
        (MessageKind::Trade, 25),
        (MessageKind::PriceLevelUpdated, 22),
        (MessageKind::BookCleared, 1),
        (MessageKind::TradingStatusChanged, 1),
    ];

    for (kind, expected) in cases {
        assert_eq!(kind.payload_size(), expected);
    }
}

#[test]
fn stable_header_layout() {
    assert_eq!(HEADER_SIZE, 40);
    assert_eq!(MAGIC_OFFSET, 0);
    assert_eq!(MAJOR_VERSION_OFFSET, 2);
    assert_eq!(MINOR_VERSION_OFFSET, 3);
    assert_eq!(MESSAGE_KIND_OFFSET, 4);
    assert_eq!(FLAGS_OFFSET, 5);
    assert_eq!(PAYLOAD_LENGTH_OFFSET, 6);
    assert_eq!(STREAM_ID_OFFSET, 8);
    assert_eq!(SESSION_ID_OFFSET, 12);
    assert_eq!(SEQUENCE_OFFSET, 20);
    assert_eq!(INSTRUMENT_ID_OFFSET, 28);
    assert_eq!(EXCHANGE_TIMESTAMP_OFFSET, 32);
}

#[test]
fn stable_magic() {
    assert_eq!(PROTOCOL_MAGIC, [0x4D, 0x58]);
}

#[test]
fn stable_timestamp_flag() {
    assert_eq!(FLAG_EXCHANGE_TIMESTAMP, 1);
}

#[test]
fn golden_frames_concatenated() {
    let mut buffer = Vec::new();

    buffer.extend_from_slice(&GOLDEN_ORDER_ADDED);
    buffer.extend_from_slice(&GOLDEN_ORDER_DELETED);

    let decoder = Decoder::new();

    let decoded: Vec<_> = decoder.frames(&buffer).collect::<Result<_, _>>().unwrap();

    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].envelope, expected_order_added());
    assert_eq!(decoded[1].envelope, expected_order_deleted());
}
