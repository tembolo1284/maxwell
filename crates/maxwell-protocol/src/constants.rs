//! Constants governing the Maxwell binary protocol.
//!
//! These constants define Maxwell's native V1 format.
//! They must not be assumed to match Planck's existing
//! wire protocol without explicit interoperability tests.

/// ASCII representation of "MX".
pub const PROTOCOL_MAGIC: [u8; 2] = *b"MX";

pub const VERSION_MAJOR: u8 = 1;
pub const VERSION_MINOR: u8 = 0;

// ============================================================
// Primitive wire sizes
// ============================================================

pub const U8_SIZE: usize = 1;
pub const U16_SIZE: usize = 2;
pub const U32_SIZE: usize = 4;
pub const U64_SIZE: usize = 8;
pub const I64_SIZE: usize = 8;

// ============================================================
// Message header layout
// ============================================================
//
// Offset   Size    Field
// ------   ----    ------------------
// 0        2       Magic
// 2        1       Major version
// 3        1       Minor version
// 4        1       Message kind
// 5        1       Flags
// 6        2       Payload length
// 8        4       Stream identifier
// 12       8       Session identifier
// 20       8       Sequence number
// 28       4       Instrument identifier
// 32       8       Exchange timestamp
//
// Total: 40 bytes.
//
// All multibyte fields use little-endian encoding.

pub const HEADER_SIZE: usize = 40;

pub const MAGIC_OFFSET: usize = 0;
pub const MAJOR_VERSION_OFFSET: usize = 2;
pub const MINOR_VERSION_OFFSET: usize = 3;
pub const MESSAGE_KIND_OFFSET: usize = 4;
pub const FLAGS_OFFSET: usize = 5;
pub const PAYLOAD_LENGTH_OFFSET: usize = 6;
pub const STREAM_ID_OFFSET: usize = 8;
pub const SESSION_ID_OFFSET: usize = 12;
pub const SEQUENCE_OFFSET: usize = 20;
pub const INSTRUMENT_ID_OFFSET: usize = 28;
pub const EXCHANGE_TIMESTAMP_OFFSET: usize = 32;

// ============================================================
// Message flags
// ============================================================

/// Indicates that an exchange timestamp is present.
pub const FLAG_EXCHANGE_TIMESTAMP: u8 = 0b0000_0001;

pub const KNOWN_FLAGS: u8 = FLAG_EXCHANGE_TIMESTAMP;

// ============================================================
// Protocol limits
// ============================================================

/// Maximum payload representable by the length field.
pub const MAX_PAYLOAD_SIZE: usize = u16::MAX as usize;

pub const MAX_MESSAGE_SIZE: usize = HEADER_SIZE + MAX_PAYLOAD_SIZE;

/// Conservative UDP application payload target.
///
/// This is a deployment recommendation, not a wire limit.
pub const RECOMMENDED_UDP_PAYLOAD_SIZE: usize = 1_200;

// ============================================================
// V1 event payload sizes
// ============================================================

pub const ORDER_ADDED_SIZE: usize = 25;
pub const ORDER_MODIFIED_SIZE: usize = 24;
pub const ORDER_CANCELLED_SIZE: usize = 16;
pub const ORDER_DELETED_SIZE: usize = 8;
pub const ORDER_EXECUTED_SIZE: usize = 32;
pub const TRADE_SIZE: usize = 25;
pub const PRICE_LEVEL_UPDATED_SIZE: usize = 22;
pub const BOOK_CLEARED_SIZE: usize = 1;
pub const TRADING_STATUS_CHANGED_SIZE: usize = 1;

// ============================================================
// Compile-time invariants
// ============================================================

const _: () = {
    assert!(EXCHANGE_TIMESTAMP_OFFSET + U64_SIZE == HEADER_SIZE);

    assert!(ORDER_ADDED_SIZE == U64_SIZE + I64_SIZE + U64_SIZE + U8_SIZE);

    assert!(ORDER_MODIFIED_SIZE == U64_SIZE + I64_SIZE + U64_SIZE);

    assert!(ORDER_CANCELLED_SIZE == U64_SIZE + U64_SIZE);

    assert!(ORDER_DELETED_SIZE == U64_SIZE);

    assert!(ORDER_EXECUTED_SIZE == U64_SIZE + U64_SIZE + I64_SIZE + U64_SIZE);

    assert!(TRADE_SIZE == U64_SIZE + I64_SIZE + U64_SIZE + U8_SIZE);

    assert!(PRICE_LEVEL_UPDATED_SIZE == I64_SIZE + U64_SIZE + U8_SIZE + U8_SIZE + U32_SIZE);
};
