//! Version-specific wire implementations.
//!
//! The public encoder and decoder dispatch through this
//! module rather than directly depending on a particular
//! protocol version.

pub mod v1;

use maxwell_core::MarketEvent;

use crate::error::{ProtocolError, ProtocolResult};
use crate::message::MessageKind;
use crate::reader::ByteReader;
use crate::version::ProtocolVersion;
use crate::writer::ByteWriter;

/// Decodes a payload using the layout for `version`.
///
/// # Errors
///
/// Returns `UnsupportedVersion` for an unknown version, or the
/// error from the versioned decoder.
#[inline]
pub fn decode_payload(
    version: ProtocolVersion,
    kind: MessageKind,
    reader: &mut ByteReader<'_>,
) -> ProtocolResult<MarketEvent> {
    match (version.major, version.minor) {
        (1, 0) => v1::decode_payload(kind, reader),

        (major, minor) => Err(ProtocolError::UnsupportedVersion { major, minor }),
    }
}

/// Encodes a payload using the layout for `version`.
///
/// # Errors
///
/// Returns `UnsupportedVersion` for an unknown version, or the
/// error from the versioned encoder.
#[inline]
pub fn encode_payload(
    version: ProtocolVersion,
    event: MarketEvent,
    writer: &mut ByteWriter<'_>,
) -> ProtocolResult<()> {
    match (version.major, version.minor) {
        (1, 0) => v1::encode_payload(event, writer),

        (major, minor) => Err(ProtocolError::UnsupportedVersion { major, minor }),
    }
}
