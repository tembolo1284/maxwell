//! Buffered and validated sequential journal reading.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::config::{ReaderConfig, TailPolicy};
use crate::error::{ReplayError, ReplayResult};
use crate::format::{
    Crc32c,
    HEADER_SIZE,
    JournalHeader,
    MIN_RECORD_LENGTH,
    RECORD_OVERHEAD,
    read_u32,
    read_u64,
};
use crate::record::{JournalRecord, StreamValidator};

pub struct JournalReader {
    input: BufReader<File>,
    config: ReaderConfig,
    header: JournalHeader,
    validator: StreamValidator,
    buffer: Vec<u8>,
    offset: u64,
    records_read: u64,
    finished: bool,
    failed: bool,
    incomplete_tail: bool,
}

impl JournalReader {
    pub fn open(
        path: impl AsRef<Path>,
        config: ReaderConfig,
    ) -> ReplayResult<Self> {
        config.validate()?;

        let file = File::open(path)?;

        let mut input =
            BufReader::with_capacity(config.buffer_bytes, file);

        let mut header_bytes = [0_u8; HEADER_SIZE];

        let actual = read_until_full(
            &mut input,
            &mut header_bytes,
        )?;

        if actual != HEADER_SIZE {
            return Err(ReplayError::TruncatedHeader {
                expected: HEADER_SIZE,
                actual,
            });
        }

        let header = JournalHeader::decode(&header_bytes)?;

        let maximum = config.max_payload_bytes.min(
            header.max_payload_bytes as usize,
        );

        let capacity = maximum + RECORD_OVERHEAD;

        let mut buffer = Vec::new();

        buffer.try_reserve_exact(capacity).map_err(|_| {
            ReplayError::AllocationFailed {
                resource: "journal decoding buffer",
                requested: capacity,
            }
        })?;

        buffer.resize(capacity, 0);

        Ok(Self {
            input,
            config,
            header,
            validator: StreamValidator::default(),
            buffer,
            offset: HEADER_SIZE as u64,
            records_read: 0,
            finished: false,
            failed: false,
            incomplete_tail: false,
        })
    }

    #[must_use]
    pub const fn header(&self) -> &JournalHeader {
        &self.header
    }

    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    #[must_use]
    pub const fn records_read(&self) -> u64 {
        self.records_read
    }

    #[must_use]
    pub const fn incomplete_tail(&self) -> bool {
        self.incomplete_tail
    }

    pub fn rewind(&mut self) -> ReplayResult<()> {
        self.input.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

        self.validator = StreamValidator::default();
        self.offset = HEADER_SIZE as u64;
        self.records_read = 0;
        self.finished = false;
        self.failed = false;
        self.incomplete_tail = false;

        Ok(())
    }

    pub(crate) fn seek_indexed(
        &mut self,
        offset: u64,
        ordinal: u64,
    ) -> ReplayResult<()> {
        if offset < HEADER_SIZE as u64 {
            return Err(ReplayError::InvalidIndex {
                reason: "offset precedes journal records",
            });
        }

        self.input.seek(SeekFrom::Start(offset))?;

        self.validator = StreamValidator::default();
        self.offset = offset;
        self.records_read = ordinal;
        self.finished = false;
        self.failed = false;
        self.incomplete_tail = false;

        Ok(())
    }

    pub fn next_record(
        &mut self,
    ) -> ReplayResult<Option<JournalRecord>> {
        if self.failed {
            return Err(ReplayError::InvalidState {
                operation: "next_record",
                state: "failed",
            });
        }

        if self.finished {
            return Ok(None);
        }

        match self.read_record() {
            Ok(record) => Ok(record),

            Err(error)
                if error.is_incomplete_record()
                    && self.config.tail_policy
                        == TailPolicy::AllowIncompleteTail =>
            {
                self.finished = true;
                self.incomplete_tail = true;

                Ok(None)
            }

            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    fn read_record(
        &mut self,
    ) -> ReplayResult<Option<JournalRecord>> {
        let mut length_bytes = [0_u8; 4];

        let actual = read_until_full(
            &mut self.input,
            &mut length_bytes,
        )?;

        if actual == 0 {
            self.finished = true;
            return Ok(None);
        }

        if actual != length_bytes.len() {
            return Err(ReplayError::TruncatedRecord {
                offset: self.offset,
                expected: length_bytes.len(),
                actual,
            });
        }

        let length = u32::from_le_bytes(length_bytes);

        if length < MIN_RECORD_LENGTH {
            return Err(ReplayError::InvalidRecordLength {
                offset: self.offset,
                length,
                minimum: MIN_RECORD_LENGTH,
            });
        }

        let body_length = length as usize;
        let payload_length = body_length - RECORD_OVERHEAD;

        if body_length > self.buffer.len() {
            return Err(ReplayError::PayloadTooLarge {
                actual: payload_length,
                maximum: self.buffer.len() - RECORD_OVERHEAD,
            });
        }

        let body = &mut self.buffer[..body_length];

        let actual = read_until_full(&mut self.input, body)?;

        if actual != body_length {
            return Err(ReplayError::TruncatedRecord {
                offset: self.offset,
                expected: body_length + 4,
                actual: actual + 4,
            });
        }

        let checksum_offset = body_length - 4;

        let expected = read_u32(&body[checksum_offset..]);

        let mut checksum = Crc32c::new();

        checksum.update(&length_bytes);
        checksum.update(&body[..checksum_offset]);

        let actual = checksum.finish();

        if expected != actual {
            return Err(ReplayError::ChecksumMismatch {
                offset: self.offset,
                expected,
                actual,
            });
        }

        let sequence = read_u64(&body[..8]);
        let timestamp = read_u64(&body[8..16]);

        let record = JournalRecord::decode_payload(
            self.offset,
            sequence,
            timestamp,
            &body[16..checksum_offset],
        )?;

        self.validator.validate(&self.header, record)?;

        let next_offset = self.offset
            .checked_add(u64::from(length) + 4)
            .ok_or(ReplayError::ArithmeticOverflow {
                operation: "journal file offset",
            })?;

        let next_count = self.records_read
            .checked_add(1)
            .ok_or(ReplayError::ArithmeticOverflow {
                operation: "journal record count",
            })?;

        self.validator.commit(record);
        self.offset = next_offset;
        self.records_read = next_count;

        Ok(Some(record))
    }
}

fn read_until_full(
    input: &mut impl Read,
    buffer: &mut [u8],
) -> io::Result<usize> {
    let mut total = 0;

    while total < buffer.len() {
        match input.read(&mut buffer[total..]) {
            Ok(0) => break,
            Ok(count) => total += count,
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    Ok(total)
}
