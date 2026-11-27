//! Malformed frame, invalid event, and boundary tests.

use maxwell_core::*;

use maxwell_protocol::{Decoder, Encoder, ProtocolError, ProtocolMessage, ProtocolVersion};

use maxwell_protocol::constants::*;

fn sample_event() -> EventEnvelope {
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

fn valid_frame() -> Vec<u8> {
    let mut buffer = [0_u8; 128];

    let written = Encoder::new()
        .encode_event(sample_event(), &mut buffer)
        .unwrap();

    buffer[..written].to_vec()
}

// ============================================================
// Header corruption
// ============================================================

#[test]
fn invalid_magic() {
    let mut frame = valid_frame();

    frame[MAGIC_OFFSET] = 0xFF;

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::InvalidMagic)
    );
}

#[test]
fn unsupported_major_version() {
    let mut frame = valid_frame();

    frame[MAJOR_VERSION_OFFSET] = 99;

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::UnsupportedVersion {
            major: 99,
            minor: 0,
        })
    );
}

#[test]
fn unsupported_minor_version() {
    let mut frame = valid_frame();

    frame[MINOR_VERSION_OFFSET] = 99;

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::UnsupportedVersion {
            major: 1,
            minor: 99,
        })
    );
}

#[test]
fn unknown_message_kind() {
    let mut frame = valid_frame();

    frame[MESSAGE_KIND_OFFSET] = 255;

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::UnknownMessageKind(255))
    );
}

#[test]
fn unknown_flags() {
    let mut frame = valid_frame();

    frame[FLAGS_OFFSET] = 0x80;

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::InvalidFlags(0x80))
    );
}

#[test]
fn noncanonical_timestamp() {
    let mut frame = valid_frame();

    frame[FLAGS_OFFSET] = 0;

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::NonCanonicalEncoding)
    );
}

#[test]
fn invalid_payload_length() {
    let mut frame = valid_frame();

    frame[PAYLOAD_LENGTH_OFFSET..PAYLOAD_LENGTH_OFFSET + 2].copy_from_slice(&10_u16.to_le_bytes());

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::InvalidPayloadLength {
            expected: ORDER_ADDED_SIZE,
            actual: 10,
        })
    );
}

// ============================================================
// Truncation and framing
// ============================================================

#[test]
fn every_truncation_rejected() {
    let frame = valid_frame();
    let decoder = Decoder::new();

    for length in 0..frame.len() {
        assert!(
            decoder.decode(&frame[..length]).is_err(),
            "accepted truncated frame of length {length}"
        );
    }
}

#[test]
fn trailing_bytes_rejected() {
    let mut frame = valid_frame();

    frame.extend_from_slice(&[0, 0, 0]);

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::TrailingBytes(3))
    );
}

#[test]
fn frame_length_from_header_only() {
    let frame = valid_frame();

    assert_eq!(
        Decoder::new().frame_length(&frame[..HEADER_SIZE]),
        Ok(frame.len())
    );
}

// ============================================================
// Invalid payloads
// ============================================================

#[test]
fn invalid_order_side() {
    let mut frame = valid_frame();

    let side_offset = HEADER_SIZE + 24;

    frame[side_offset] = 99;

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::InvalidSide(99))
    );
}

#[test]
fn zero_order_quantity() {
    let mut frame = valid_frame();

    let quantity_offset = HEADER_SIZE + 16;

    frame[quantity_offset..quantity_offset + 8].fill(0);

    assert_eq!(
        Decoder::new().decode(&frame),
        Err(ProtocolError::InvalidEvent(EventError::ZeroOrderQuantity))
    );
}

#[test]
fn invalid_optional_order_count() {
    let mut event = sample_event();

    event.event = MarketEvent::PriceLevelUpdated(PriceLevelUpdated {
        price: Price::new(100),
        quantity: Quantity::new(10),
        order_count: None,
        side: Side::Buy,
    });

    let mut buffer = [0_u8; 128];

    let written = Encoder::new().encode_event(event, &mut buffer).unwrap();

    // Price: 8, quantity: 8, side: 1.
    let presence_offset = HEADER_SIZE + 17;

    buffer[presence_offset] = 2;

    assert_eq!(
        Decoder::new().decode(&buffer[..written]),
        Err(ProtocolError::InvalidPresenceFlag(2))
    );
}

#[test]
fn invalid_trading_status() {
    let mut event = sample_event();

    event.event = MarketEvent::TradingStatusChanged(TradingStatusChanged {
        status: TradingStatus::Open,
    });

    let mut buffer = [0_u8; 128];

    let written = Encoder::new().encode_event(event, &mut buffer).unwrap();

    buffer[HEADER_SIZE] = 255;

    assert_eq!(
        Decoder::new().decode(&buffer[..written]),
        Err(ProtocolError::InvalidTradingStatus(255))
    );
}

// ============================================================
// Encoder rejection
// ============================================================

#[test]
fn invalid_event_does_not_modify_buffer() {
    let mut event = sample_event();

    if let MarketEvent::OrderAdded(ref mut value) = event.event {
        value.quantity = Quantity::ZERO;
    }

    let mut buffer = [0xAA_u8; 128];

    assert_eq!(
        Encoder::new().encode_event(event, &mut buffer),
        Err(ProtocolError::InvalidEvent(EventError::ZeroOrderQuantity))
    );

    assert_eq!(buffer, [0xAA; 128]);
}

#[test]
fn unsupported_version_does_not_modify_buffer() {
    let message = ProtocolMessage::new(ProtocolVersion::new(99, 0), sample_event());

    let mut buffer = [0xAA_u8; 128];

    assert!(Encoder::new().encode(message, &mut buffer).is_err());

    assert_eq!(buffer, [0xAA; 128]);
}

// ============================================================
// Deterministic mutation testing
// ============================================================

#[test]
fn single_byte_mutations_do_not_panic() {
    let original = valid_frame();
    let decoder = Decoder::new();

    for position in 0..original.len() {
        let mut mutated = original.clone();

        for value in u8::MIN..=u8::MAX {
            mutated[position] = value;

            // Mutations may still represent valid messages.
            // The invariant is that decoding never panics.
            let _ = decoder.decode(&mutated);
        }
    }
}

#[test]
fn arbitrary_buffers_do_not_panic() {
    let decoder = Decoder::new();
    let mut state = 0x1234_5678_ABCD_EF01_u64;
    let mut buffer = [0_u8; 256];

    for length in 0..=256 {
        for _ in 0..32 {
            for byte in &mut buffer[..length] {
                // Deterministic xorshift generator.
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;

                *byte = state.to_le_bytes()[0];
            }

            let _ = decoder.decode(&buffer[..length]);
        }
    }
}
