//! Single-owner recovery instrumentation.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryStats {
    pub events_received: u64,
    pub events_applied: u64,
    pub events_buffered: u64,
    pub events_replayed: u64,
    pub duplicate_events: u64,
    pub conflicting_events: u64,
    pub rejected_events: u64,

    pub gaps_detected: u64,
    pub missing_sequences_detected: u64,

    pub retransmission_requests: u64,
    pub retransmission_retries: u64,
    pub retransmission_timeouts: u64,

    pub snapshot_requests: u64,
    pub snapshot_retries: u64,
    pub snapshot_timeouts: u64,
    pub snapshots_installed: u64,

    pub buffer_overflows: u64,
    pub buffer_high_watermark: usize,
    pub events_discarded: u64,

    pub recoveries_completed: u64,
    pub recovery_failures: u64,
    pub session_transitions: u64,
}

macro_rules! increment_counter {
    ($($method:ident => $field:ident),+ $(,)?) => {
        $(
            #[inline]
            pub fn $method(&mut self) {
                self.$field = self.$field.saturating_add(1);
            }
        )+
    };
}

impl RecoveryStats {
    increment_counter!(
        record_received => events_received,
        record_applied => events_applied,
        record_replayed => events_replayed,
        record_duplicate => duplicate_events,
        record_conflict => conflicting_events,
        record_rejected => rejected_events,
        record_retransmission_request => retransmission_requests,
        record_retransmission_retry => retransmission_retries,
        record_retransmission_timeout => retransmission_timeouts,
        record_snapshot_request => snapshot_requests,
        record_snapshot_retry => snapshot_retries,
        record_snapshot_timeout => snapshot_timeouts,
        record_snapshot_installed => snapshots_installed,
        record_buffer_overflow => buffer_overflows,
        record_recovery_completed => recoveries_completed,
        record_recovery_failure => recovery_failures,
        record_session_transition => session_transitions,
    );

    pub fn record_gap(&mut self, missing: u128) {
        self.gaps_detected = self.gaps_detected.saturating_add(1);

        let missing = u64::try_from(missing).unwrap_or(u64::MAX);

        self.missing_sequences_detected = self.missing_sequences_detected.saturating_add(missing);
    }

    pub fn record_buffered(&mut self, occupancy: usize) {
        self.events_buffered = self.events_buffered.saturating_add(1);

        self.buffer_high_watermark = self.buffer_high_watermark.max(occupancy);
    }

    pub fn record_discarded(&mut self, count: usize) {
        self.events_discarded = self
            .events_discarded
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_watermark_never_decreases() {
        let mut stats = RecoveryStats::default();

        stats.record_buffered(10);
        stats.record_buffered(5);

        assert_eq!(stats.buffer_high_watermark, 10);
        assert_eq!(stats.events_buffered, 2);
    }

    #[test]
    fn counters_saturate() {
        let mut stats = RecoveryStats {
            events_received: u64::MAX,
            ..RecoveryStats::default()
        };

        stats.record_received();

        assert_eq!(stats.events_received, u64::MAX);
    }
}
