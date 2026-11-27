
//! Single-owner transport statistics.
//!
//! Counters saturate rather than wrap.
//! These counters are observational and do not participate
//! in market data sequencing or correctness decisions.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportStats {
    pub bytes_received: u64,
    pub bytes_sent: u64,

    pub datagrams_received: u64,
    pub datagrams_sent: u64,

    pub read_operations: u64,
    pub write_operations: u64,

    pub read_would_block: u64,
    pub write_would_block: u64,

    pub interrupted_operations: u64,

    pub rejected_datagrams: u64,
    pub partial_writes: u64,

    pub connection_closures: u64,
    pub io_errors: u64,
}

impl TransportStats {
    #[inline]
    pub fn record_read(&mut self, bytes: usize) {
        self.bytes_received = self.bytes_received
            .saturating_add(bytes as u64);

        self.read_operations =
            self.read_operations.saturating_add(1);
    }

    #[inline]
    pub fn record_write(&mut self, bytes: usize) {
        self.bytes_sent = self.bytes_sent
            .saturating_add(bytes as u64);

        self.write_operations =
            self.write_operations.saturating_add(1);
    }

    #[inline]
    pub fn record_datagram_received(&mut self, bytes: usize) {
        self.record_read(bytes);

        self.datagrams_received =
            self.datagrams_received.saturating_add(1);
    }

    #[inline]
    pub fn record_datagram_sent(&mut self, bytes: usize) {
        self.record_write(bytes);

        self.datagrams_sent =
            self.datagrams_sent.saturating_add(1);
    }

    #[inline]
    pub fn record_read_would_block(&mut self) {
        self.read_would_block =
            self.read_would_block.saturating_add(1);
    }

    #[inline]
    pub fn record_write_would_block(&mut self) {
        self.write_would_block =
            self.write_would_block.saturating_add(1);
    }

    #[inline]
    pub fn record_interrupted(&mut self) {
        self.interrupted_operations =
            self.interrupted_operations.saturating_add(1);
    }

    #[inline]
    pub fn record_rejected_datagram(&mut self) {
        self.rejected_datagrams =
            self.rejected_datagrams.saturating_add(1);
    }

    #[inline]
    pub fn record_partial_write(&mut self) {
        self.partial_writes =
            self.partial_writes.saturating_add(1);
    }

    #[inline]
    pub fn record_connection_closed(&mut self) {
        self.connection_closures =
            self.connection_closures.saturating_add(1);
    }

    #[inline]
    pub fn record_error(&mut self) {
        self.io_errors = self.io_errors.saturating_add(1);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datagram_statistics() {
        let mut stats = TransportStats::default();

        stats.record_datagram_received(100);
        stats.record_datagram_received(200);
        stats.record_datagram_sent(50);

        assert_eq!(stats.bytes_received, 300);
        assert_eq!(stats.bytes_sent, 50);
        assert_eq!(stats.datagrams_received, 2);
        assert_eq!(stats.datagrams_sent, 1);
    }

    #[test]
    fn counters_saturate() {
        let mut stats = TransportStats {
            bytes_received: u64::MAX,
            ..TransportStats::default()
        };

        stats.record_read(100);

        assert_eq!(stats.bytes_received, u64::MAX);
    }

    #[test]
    fn reset_statistics() {
        let mut stats = TransportStats::default();

        stats.record_read(100);
        stats.record_error();
        stats.reset();

        assert_eq!(stats, TransportStats::default());
    }
}

