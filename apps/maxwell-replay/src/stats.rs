//! Recording and playback statistics.

use crate::record::JournalRecord;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordingStats {
    pub records_written: u64,
    pub bytes_written: u64,
    pub flushes: u64,
    pub synchronizations: u64,
    pub first_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub first_timestamp_ns: Option<u64>,
    pub last_timestamp_ns: Option<u64>,
}

impl RecordingStats {
    pub(crate) fn record_written(
        &mut self,
        record: JournalRecord,
        bytes: u64,
    ) {
        self.records_written =
            self.records_written.saturating_add(1);

        self.bytes_written =
            self.bytes_written.saturating_add(bytes);

        self.first_sequence
            .get_or_insert(record.sequence());

        self.first_timestamp_ns
            .get_or_insert(record.recording_timestamp_ns);

        self.last_sequence = Some(record.sequence());

        self.last_timestamp_ns =
            Some(record.recording_timestamp_ns);
    }

    #[must_use]
    pub fn duration_ns(&self) -> Option<u64> {
        Some(
            self.last_timestamp_ns?
                .saturating_sub(self.first_timestamp_ns?),
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplayStats {
    pub records_read: u64,
    pub records_delivered: u64,
    pub records_filtered: u64,
    pub backpressure_attempts: u64,
    pub last_delivered_sequence: Option<u64>,
    pub last_delivered_timestamp_ns: Option<u64>,
}

impl ReplayStats {
    pub(crate) fn delivered(&mut self, record: JournalRecord) {
        self.records_delivered =
            self.records_delivered.saturating_add(1);

        self.last_delivered_sequence =
            Some(record.sequence());

        self.last_delivered_timestamp_ns =
            Some(record.recording_timestamp_ns);
    }
}
