//! Version-one binary journal format.
//!
//! All journal envelope fields use little-endian encoding.
//!
//! Header layout:
//!
//! Offset  Size  Field
//! 0       8     Magic
//! 8       2     Version
//! 10      1     Journal mode
//! 11      1     Reserved
//! 12      4     Header length
//! 16      8     Creation timestamp
//! 24      4     Maximum payload size
//! 28      4     Stream identifier
//! 32      8     Session identifier
//! 40      8     Recording identifier
//! 48      12    Reserved
//! 60      4     CRC32C
//!
//! Record layout:
//!
//! 4 bytes: record length, excluding the length field
//! 8 bytes: upstream sequence
//! 8 bytes: recording timestamp
//! N bytes: Maxwell protocol frame
//! 4 bytes: CRC32C

use maxwell_core::{FeedId, SessionId, StreamId};

use crate::config::{JournalMode, MAX_PAYLOAD_BYTES};
use crate::error::{ReplayError, ReplayResult};

pub const JOURNAL_MAGIC: [u8; 8] = *b"MXWJRNL\0";
pub const JOURNAL_VERSION: u16 = 1;

pub const HEADER_SIZE: usize = 64;
pub const HEADER_CHECKSUM_OFFSET: usize = 60;

pub const RECORD_LENGTH_SIZE: usize = 4;
pub const RECORD_METADATA_SIZE: usize = 16;
pub const CHECKSUM_SIZE: usize = 4;

pub const RECORD_OVERHEAD: usize = RECORD_METADATA_SIZE + CHECKSUM_SIZE;

pub const MIN_RECORD_LENGTH: u32 = (RECORD_OVERHEAD + 1) as u32;

// ============================================================
// Journal header
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalHeader {
    pub mode: JournalMode,
    pub created_at_ns: u64,
    pub max_payload_bytes: u32,
    pub feed: FeedId,
    pub recording_id: u64,
}

impl JournalHeader {
    pub fn validate(&self) -> ReplayResult<()> {
        let maximum = self.max_payload_bytes as usize;

        if maximum == 0 || maximum > MAX_PAYLOAD_BYTES {
            return Err(ReplayError::InvalidHeader {
                reason: "invalid maximum payload size",
            });
        }

        Ok(())
    }

    pub fn encode(&self) -> ReplayResult<[u8; HEADER_SIZE]> {
        self.validate()?;

        let mut bytes = [0_u8; HEADER_SIZE];

        bytes[0..8].copy_from_slice(&JOURNAL_MAGIC);
        bytes[8..10].copy_from_slice(&JOURNAL_VERSION.to_le_bytes());

        bytes[10] = match self.mode {
            JournalMode::Authoritative => 0,
            JournalMode::RawCapture => 1,
        };

        bytes[12..16].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());

        bytes[16..24].copy_from_slice(&self.created_at_ns.to_le_bytes());

        bytes[24..28].copy_from_slice(&self.max_payload_bytes.to_le_bytes());

        bytes[28..32].copy_from_slice(&self.feed.stream_id.raw().to_le_bytes());

        bytes[32..40].copy_from_slice(&self.feed.session_id.raw().to_le_bytes());

        bytes[40..48].copy_from_slice(&self.recording_id.to_le_bytes());

        let checksum = crc32c(&bytes[..HEADER_CHECKSUM_OFFSET]);

        bytes[HEADER_CHECKSUM_OFFSET..].copy_from_slice(&checksum.to_le_bytes());

        Ok(bytes)
    }

    pub fn decode(bytes: &[u8; HEADER_SIZE]) -> ReplayResult<Self> {
        if bytes[..8] != JOURNAL_MAGIC {
            return Err(ReplayError::InvalidMagic);
        }

        let version = read_u16(&bytes[8..10]);

        if version != JOURNAL_VERSION {
            return Err(ReplayError::UnsupportedVersion {
                found: version,
                supported: JOURNAL_VERSION,
            });
        }

        let expected = read_u32(&bytes[60..64]);
        let actual = crc32c(&bytes[..60]);

        if expected != actual {
            return Err(ReplayError::ChecksumMismatch {
                offset: 0,
                expected,
                actual,
            });
        }

        if read_u32(&bytes[12..16]) != HEADER_SIZE as u32 {
            return Err(ReplayError::InvalidHeader {
                reason: "invalid header length",
            });
        }

        if bytes[11] != 0 || bytes[48..60].iter().any(|&byte| byte != 0) {
            return Err(ReplayError::InvalidHeader {
                reason: "reserved bytes must be zero",
            });
        }

        let mode = match bytes[10] {
            0 => JournalMode::Authoritative,
            1 => JournalMode::RawCapture,
            value => {
                return Err(ReplayError::InvalidJournalMode { value });
            }
        };

        let header = Self {
            mode,
            created_at_ns: read_u64(&bytes[16..24]),
            max_payload_bytes: read_u32(&bytes[24..28]),
            feed: FeedId::new(
                StreamId::new(read_u32(&bytes[28..32])),
                SessionId::new(read_u64(&bytes[32..40])),
            ),
            recording_id: read_u64(&bytes[40..48]),
        };

        header.validate()?;

        Ok(header)
    }
}

// ============================================================
// Internal integer decoding
// ============================================================

pub(crate) fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().expect("fixed-width u16 field"))
}

pub(crate) fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("fixed-width u32 field"))
}

pub(crate) fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("fixed-width u64 field"))
}

// ============================================================
// CRC32C
// ============================================================

const CRC32C_POLYNOMIAL: u32 = 0x82F6_3B78;

const fn build_crc32c_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut index = 0;

    while index < table.len() {
        let mut value = index as u32;
        let mut bit = 0;

        while bit < 8 {
            value = if value & 1 != 0 {
                (value >> 1) ^ CRC32C_POLYNOMIAL
            } else {
                value >> 1
            };

            bit += 1;
        }

        table[index] = value;
        index += 1;
    }

    table
}

const CRC32C_TABLE: [u32; 256] = build_crc32c_table();

#[derive(Debug, Clone, Copy)]
pub struct Crc32c {
    state: u32,
}

impl Default for Crc32c {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32c {
    #[must_use]
    pub const fn new() -> Self {
        Self { state: u32::MAX }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            let index = ((self.state ^ u32::from(byte)) & 0xFF) as usize;

            self.state = CRC32C_TABLE[index] ^ (self.state >> 8);
        }
    }

    #[must_use]
    pub const fn finish(self) -> u32 {
        !self.state
    }
}

#[must_use]
pub fn crc32c(bytes: &[u8]) -> u32 {
    let mut checksum = Crc32c::new();
    checksum.update(bytes);
    checksum.finish()
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> JournalHeader {
        JournalHeader {
            mode: JournalMode::Authoritative,
            created_at_ns: 1_000,
            max_payload_bytes: 65_575,
            feed: FeedId::new(StreamId::new(1), SessionId::new(2)),
            recording_id: 42,
        }
    }

    #[test]
    fn standard_crc32c_vector() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn empty_crc32c() {
        assert_eq!(crc32c(&[]), 0);
    }

    #[test]
    fn incremental_checksum() {
        let mut checksum = Crc32c::new();

        checksum.update(b"123");
        checksum.update(b"456");
        checksum.update(b"789");

        assert_eq!(checksum.finish(), crc32c(b"123456789"));
    }

    #[test]
    fn header_round_trip() {
        let original = header();
        let bytes = original.encode().unwrap();

        assert_eq!(JournalHeader::decode(&bytes).unwrap(), original);
    }

    #[test]
    fn corrupted_header_is_rejected() {
        let mut bytes = header().encode().unwrap();

        bytes[40] ^= 1;

        assert!(matches!(
            JournalHeader::decode(&bytes),
            Err(ReplayError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut bytes = header().encode().unwrap();

        bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());

        assert!(matches!(
            JournalHeader::decode(&bytes),
            Err(ReplayError::UnsupportedVersion { .. })
        ));
    }
}
