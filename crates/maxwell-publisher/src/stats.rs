//! Lightweight publisher statistics.
//!
//! Counters are owned by the publisher thread and therefore
//! do not require atomic operations.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PublisherStats {
    /// Publications accepted by the publisher.
    pub publications_accepted: u64,

    /// Publications rejected due to backpressure.
    pub publications_rejected: u64,

    /// Accepted publications with no matching active subscriber.
    pub publications_unmatched: u64,

    /// Total publications inserted into subscriber queues.
    pub deliveries_enqueued: u64,

    /// Total publications removed through pop or successful drain.
    pub deliveries_dequeued: u64,

    /// Queued publications discarded during invalidation
    /// or subscriber removal.
    pub deliveries_discarded: u64,

    /// Matching deliveries not enqueued due to overflow.
    pub deliveries_missed: u64,

    /// Subscribers registered successfully.
    pub subscribers_registered: u64,

    /// Subscribers removed successfully.
    pub subscribers_removed: u64,

    /// Successful baseline activations.
    pub subscribers_activated: u64,

    /// Subscribers transitioned into ResyncRequired.
    pub resync_transitions: u64,

    /// Failed sink acceptance attempts.
    pub sink_failures: u64,

    /// Highest observed subscriber queue occupancy.
    pub queue_high_watermark: usize,
}

impl PublisherStats {
    #[inline]
    pub(crate) fn increment(counter: &mut u64) {
        *counter = counter.saturating_add(1);
    }

    #[inline]
    pub(crate) fn add(counter: &mut u64, amount: usize) {
        let amount = u64::try_from(amount).unwrap_or(u64::MAX);
        *counter = counter.saturating_add(amount);
    }

    #[inline]
    pub(crate) fn observe_queue_depth(&mut self, depth: usize) {
        self.queue_high_watermark = self.queue_high_watermark.max(depth);
    }

    /// Number of deliveries currently retained in queues,
    /// provided that counters have not saturated.
    pub fn pending_deliveries(&self) -> u64 {
        self.deliveries_enqueued
            .saturating_sub(self.deliveries_dequeued)
            .saturating_sub(self.deliveries_discarded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero() {
        let stats = PublisherStats::default();

        assert_eq!(stats.publications_accepted, 0);
        assert_eq!(stats.deliveries_enqueued, 0);
        assert_eq!(stats.pending_deliveries(), 0);
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let mut counter = u64::MAX - 1;

        PublisherStats::increment(&mut counter);
        PublisherStats::increment(&mut counter);

        assert_eq!(counter, u64::MAX);
    }

    #[test]
    fn records_high_watermark() {
        let mut stats = PublisherStats::default();

        stats.observe_queue_depth(4);
        stats.observe_queue_depth(2);
        stats.observe_queue_depth(8);

        assert_eq!(stats.queue_high_watermark, 8);
    }

    #[test]
    fn calculates_pending_deliveries() {
        let stats = PublisherStats {
            deliveries_enqueued: 100,
            deliveries_dequeued: 60,
            deliveries_discarded: 15,
            ..PublisherStats::default()
        };

        assert_eq!(stats.pending_deliveries(), 25);
    }
}
