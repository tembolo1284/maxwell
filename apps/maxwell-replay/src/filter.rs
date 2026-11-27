//! Allocation-free journal record filtering.

use maxwell_core::{EventKind, InstrumentId};

use crate::error::{ReplayError, ReplayResult};
use crate::record::JournalRecord;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplayFilter {
    pub instrument: Option<InstrumentId>,
    pub event_kind: Option<EventKind>,
    pub first_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub first_timestamp_ns: Option<u64>,
    pub last_timestamp_ns: Option<u64>,
}

impl ReplayFilter {
    pub fn validate(&self) -> ReplayResult<()> {
        if let (Some(first), Some(last)) = (self.first_sequence, self.last_sequence)
            && first > last
        {
            return Err(ReplayError::InvalidConfig {
                field: "filter.sequence_range",
                reason: "first sequence exceeds last sequence",
            });
        }

        if let (Some(first), Some(last)) = (self.first_timestamp_ns, self.last_timestamp_ns)
            && first > last
        {
            return Err(ReplayError::InvalidConfig {
                field: "filter.timestamp_range",
                reason: "first timestamp exceeds last timestamp",
            });
        }

        Ok(())
    }

    #[must_use]
    pub const fn is_unrestricted(&self) -> bool {
        self.instrument.is_none()
            && self.event_kind.is_none()
            && self.first_sequence.is_none()
            && self.last_sequence.is_none()
            && self.first_timestamp_ns.is_none()
            && self.last_timestamp_ns.is_none()
    }

    #[must_use]
    pub fn matches(&self, record: JournalRecord) -> bool {
        if let Some(instrument) = self.instrument
            && record.event.instrument_id() != instrument
        {
            return false;
        }

        if let Some(kind) = self.event_kind
            && record.event.kind() != kind
        {
            return false;
        }

        if let Some(first) = self.first_sequence
            && record.sequence() < first
        {
            return false;
        }

        if let Some(last) = self.last_sequence
            && record.sequence() > last
        {
            return false;
        }

        if let Some(first) = self.first_timestamp_ns
            && record.recording_timestamp_ns < first
        {
            return false;
        }

        if let Some(last) = self.last_timestamp_ns
            && record.recording_timestamp_ns > last
        {
            return false;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_is_unrestricted() {
        let filter = ReplayFilter::default();

        assert!(filter.is_unrestricted());
        assert!(filter.validate().is_ok());
    }

    #[test]
    fn instrument_filter_is_restricted() {
        let filter = ReplayFilter {
            instrument: Some(InstrumentId::new(1)),
            ..ReplayFilter::default()
        };

        assert!(!filter.is_unrestricted());
    }

    #[test]
    fn invalid_sequence_range() {
        let filter = ReplayFilter {
            first_sequence: Some(100),
            last_sequence: Some(50),
            ..ReplayFilter::default()
        };

        assert!(filter.validate().is_err());
    }

    #[test]
    fn invalid_timestamp_range() {
        let filter = ReplayFilter {
            first_timestamp_ns: Some(100),
            last_timestamp_ns: Some(50),
            ..ReplayFilter::default()
        };

        assert!(filter.validate().is_err());
    }
}
