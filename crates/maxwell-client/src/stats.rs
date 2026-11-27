//! Single-owner client statistics.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClientStats {
    pub connections_established: u64,
    pub connections_closed: u64,
    pub connection_timeouts: u64,

    pub frames_received: u64,
    pub frames_accepted: u64,
    pub frames_rejected: u64,
    pub duplicate_frames: u64,
    pub transport_gaps: u64,

    pub snapshots_started: u64,
    pub snapshots_completed: u64,
    pub snapshots_aborted: u64,
    pub snapshot_entries_received: u64,
    pub snapshot_high_watermark: usize,

    pub incrementals_applied: u64,
    pub incrementals_ignored: u64,
    pub publication_regressions: u64,

    pub heartbeats_received: u64,
    pub heartbeats_sent: u64,

    pub recovery_transitions: u64,
    pub handler_failures: u64,
}

impl ClientStats {
    #[inline]
    pub(crate) fn increment(counter: &mut u64) {
        *counter = counter.saturating_add(1);
    }

    #[inline]
    pub(crate) fn observe_snapshot_depth(&mut self, depth: usize) {
        self.snapshot_high_watermark = self.snapshot_high_watermark.max(depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero() {
        let stats = ClientStats::default();

        assert_eq!(stats.frames_received, 0);
        assert_eq!(stats.incrementals_applied, 0);
        assert_eq!(stats.snapshots_completed, 0);
    }

    #[test]
    fn counters_saturate() {
        let mut counter = u64::MAX - 1;

        ClientStats::increment(&mut counter);
        ClientStats::increment(&mut counter);

        assert_eq!(counter, u64::MAX);
    }

    #[test]
    fn tracks_snapshot_high_watermark() {
        let mut stats = ClientStats::default();

        stats.observe_snapshot_depth(10);
        stats.observe_snapshot_depth(5);
        stats.observe_snapshot_depth(20);

        assert_eq!(stats.snapshot_high_watermark, 20);
    }
}
