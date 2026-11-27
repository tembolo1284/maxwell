//! Single-owner gateway statistics.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GatewayStats {
    pub sessions_connected: u64,
    pub sessions_disconnected: u64,
    pub sessions_expired: u64,

    pub subscriptions_created: u64,
    pub subscriptions_removed: u64,
    pub snapshots_activated: u64,

    pub client_requests_accepted: u64,
    pub client_requests_rejected: u64,

    pub incremental_messages_sent: u64,
    pub heartbeats_sent: u64,
    pub heartbeats_received: u64,

    pub resync_transitions: u64,
    pub resync_notifications_sent: u64,

    pub sink_failures: u64,

    pub session_high_watermark: usize,
}

impl GatewayStats {
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
    pub(crate) fn observe_sessions(&mut self, count: usize) {
        self.session_high_watermark = self.session_high_watermark.max(count);
    }

    pub fn total_messages_sent(&self) -> u64 {
        self.incremental_messages_sent
            .saturating_add(self.heartbeats_sent)
            .saturating_add(self.resync_notifications_sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero() {
        let stats = GatewayStats::default();

        assert_eq!(stats.total_messages_sent(), 0);
        assert_eq!(stats.sessions_connected, 0);
    }

    #[test]
    fn counters_saturate() {
        let mut counter = u64::MAX;

        GatewayStats::increment(&mut counter);

        assert_eq!(counter, u64::MAX);
    }

    #[test]
    fn tracks_session_high_watermark() {
        let mut stats = GatewayStats::default();

        stats.observe_sessions(4);
        stats.observe_sessions(2);
        stats.observe_sessions(8);

        assert_eq!(stats.session_high_watermark, 8);
    }

    #[test]
    fn calculates_total_messages() {
        let stats = GatewayStats {
            incremental_messages_sent: 100,
            heartbeats_sent: 10,
            resync_notifications_sent: 2,
            ..GatewayStats::default()
        };

        assert_eq!(stats.total_messages_sent(), 112);
    }
}
