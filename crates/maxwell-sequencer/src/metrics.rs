//! Single-writer sequencing metrics.
//!
//! Counters require no atomic operations because the
//! owning stream sequencer has exclusive mutable access.
//!
//! Diagnostic counters saturate rather than wrap.
//! Domain sequence numbers never use saturating arithmetic.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SequencerMetrics {
    pub received: u64,
    pub delivered: u64,
    pub duplicates: u64,
    pub out_of_order: u64,
    pub gaps_detected: u64,
    pub gaps_resolved: u64,
    pub recovery_requests: u64,
    pub recoveries_completed: u64,
    pub session_changes: u64,
    pub rejected: u64,
    pub buffer_high_watermark: usize,
}

impl SequencerMetrics {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            received: 0,
            delivered: 0,
            duplicates: 0,
            out_of_order: 0,
            gaps_detected: 0,
            gaps_resolved: 0,
            recovery_requests: 0,
            recoveries_completed: 0,
            session_changes: 0,
            rejected: 0,
            buffer_high_watermark: 0,
        }
    }

    #[inline]
    pub(crate) const fn on_received(&mut self) {
        self.received = self.received.saturating_add(1);
    }

    #[inline]
    pub(crate) const fn on_delivered(&mut self) {
        self.delivered = self.delivered.saturating_add(1);
    }

    #[inline]
    pub(crate) const fn on_duplicate(&mut self) {
        self.duplicates = self.duplicates.saturating_add(1);
    }

    #[inline]
    pub(crate) fn on_out_of_order(&mut self, buffered: usize) {
        self.out_of_order = self.out_of_order.saturating_add(1);

        self.buffer_high_watermark = self.buffer_high_watermark.max(buffered);
    }

    #[inline]
    pub(crate) const fn on_gap_detected(&mut self) {
        self.gaps_detected = self.gaps_detected.saturating_add(1);
    }

    #[inline]
    pub(crate) const fn on_gap_resolved(&mut self) {
        self.gaps_resolved = self.gaps_resolved.saturating_add(1);
    }

    #[inline]
    pub(crate) const fn on_recovery_requested(&mut self) {
        self.recovery_requests = self.recovery_requests.saturating_add(1);
    }

    #[inline]
    pub(crate) const fn on_recovery_completed(&mut self) {
        self.recoveries_completed = self.recoveries_completed.saturating_add(1);
    }

    #[inline]
    pub(crate) const fn on_session_changed(&mut self) {
        self.session_changes = self.session_changes.saturating_add(1);
    }

    #[inline]
    pub(crate) const fn on_rejected(&mut self) {
        self.rejected = self.rejected.saturating_add(1);
    }

    /// Resets diagnostic counters only.
    ///
    /// Does not affect sequencing state.
    pub const fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_metrics_are_zero() {
        assert_eq!(SequencerMetrics::default(), SequencerMetrics::new(),);
    }

    #[test]
    fn counters_increment() {
        let mut metrics = SequencerMetrics::new();

        metrics.on_received();
        metrics.on_delivered();
        metrics.on_duplicate();

        assert_eq!(metrics.received, 1);
        assert_eq!(metrics.delivered, 1);
        assert_eq!(metrics.duplicates, 1);
    }

    #[test]
    fn high_watermark_never_decreases() {
        let mut metrics = SequencerMetrics::new();

        metrics.on_out_of_order(10);
        metrics.on_out_of_order(5);

        assert_eq!(metrics.buffer_high_watermark, 10);
        assert_eq!(metrics.out_of_order, 2);
    }

    #[test]
    fn counter_saturation() {
        let mut metrics = SequencerMetrics {
            received: u64::MAX,
            ..SequencerMetrics::new()
        };

        metrics.on_received();

        assert_eq!(metrics.received, u64::MAX);
    }

    #[test]
    fn reset_metrics() {
        let mut metrics = SequencerMetrics::new();

        metrics.on_received();
        metrics.on_gap_detected();
        metrics.reset();

        assert_eq!(metrics, SequencerMetrics::new());
    }
}
