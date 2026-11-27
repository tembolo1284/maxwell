#![forbid(unsafe_code)]

//! Binary market data protocol for Maxwell.
//!
//! Provides versioned binary encoding, decoding, validation,
//! and translation between wire messages and canonical
//! maxwell-core domain events.
//!
//! Design principles:
//!
//! - Allocation-free encoding and decoding.
//! - Explicit byte ordering.
//! - Checked buffer access.
//! - Deterministic wire representations.
//! - Version-specific protocol implementations.
//! - No unsafe code.
//! - No transport-specific dependencies.
//! - No implicit serialization of Rust memory layouts.

pub mod constants;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod header;
pub mod message;
pub mod reader;
pub mod validation;
pub mod version;
pub mod wire;
pub mod writer;

// ============================================================
// Protocol errors
// ============================================================

pub use error::{ProtocolError, ProtocolResult};

// ============================================================
// Protocol versioning
// ============================================================

pub use version::ProtocolVersion;

// ============================================================
// Binary buffer primitives
// ============================================================

pub use reader::ByteReader;
pub use writer::ByteWriter;

// ============================================================
// Message representations
// ============================================================

pub use header::MessageHeader;

pub use message::{MessageKind, ProtocolMessage};

// ============================================================
// Encoding and decoding
// ============================================================

pub use decoder::Decoder;
pub use encoder::Encoder;
