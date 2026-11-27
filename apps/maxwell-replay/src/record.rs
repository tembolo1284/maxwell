//! Canonical journal records and stream validation.

use maxwell_core::EventEnvelope;
use maxwell_protocol::{Decoder, Encoder};

use crate::config::JournalMode;
use crate::error::{ReplayError, ReplayResult};
use crate::format::JournalHeader;

// ============================================================
// Journal record
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalRecord {
    pub recording_timestamp_ns: u64,
    pub event: EventEnvelope,
}

impl JournalRecord {
    #[must_use]
    pub const fn new(
        recording_timestamp_ns: u64,
        event: EventEnvelope,
    ) -> Self {
        Self {
            recording_timestamp_ns,
            event,
        }
    }

    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.event.sequence().raw()
    }

    pub fn encode_payload(
        self,
        buffer: &mut [u8],
    ) -> ReplayResult<usize> {
        Encoder
            .encode_event(self.event, buffer)
            .map_err(|error| ReplayError::EncodingFailed {
                reason: error.to_string(),
            })
    }

    pub fn decode_payload(
        offset: u64,
        sequence: u64,
        recording_timestamp_ns: u64,
        payload: &[u8],
    ) -> ReplayResult<Self> {
        if payload.is_empty() {
            return Err(ReplayError::EmptyPayload);
        }

        let event = Decoder
            .decode_event(payload)
            .map_err(|error| ReplayError::DecodingFailed {
                offset,
                reason: error.to_string(),
            })?;

        let actual = event.sequence().raw();

        if sequence != actual {
            return Err(ReplayError::SequenceMetadataMismatch {
                envelope: sequence,
                payload: actual,
            });
        }

        Ok(Self::new(recording_timestamp_ns, event))
    }
}

// ============================================================
// Stream validation
// ============================================================

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StreamValidator {
    previous_sequence: Option<u64>,
    previous_timestamp: Option<u64>,
}

impl StreamValidator {
    pub(crate) fn validate(
        &self,
        header: &JournalHeader,
        record: JournalRecord,
    ) -> ReplayResult<()> {
        if record.event.feed() != header.feed {
            return Err(ReplayError::FeedMismatch);
        }

        if let Some(previous) = self.previous_timestamp
            && record.recording_timestamp_ns < previous
        {
            return Err(ReplayError::TimestampRegression {
                previous,
                actual: record.recording_timestamp_ns,
            });
        }

        if header.mode == JournalMode::Authoritative
            && let Some(previous) = self.previous_sequence
        {
            let expected = previous.checked_add(1).ok_or(
                ReplayError::SequenceOverflow {
                    sequence: previous,
                },
            )?;

            if record.sequence() != expected {
                return Err(ReplayError::SequenceMismatch {
                    expected,
                    actual: record.sequence(),
                });
            }
        }

        Ok(())
    }

    pub(crate) fn commit(&mut self, record: JournalRecord) {
        self.previous_sequence = Some(record.sequence());
        self.previous_timestamp =
            Some(record.recording_timestamp_ns);
    }
}
