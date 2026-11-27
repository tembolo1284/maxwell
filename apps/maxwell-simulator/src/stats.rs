//! Simulator operational statistics.
//!
//! Authoritative generation, delivery, and recovery
//! statistics are tracked independently.
//!
//! All counters are single-owner and require no atomics
//! or synchronization primitives.

use std::fmt;
use std::time::{Duration, Instant};

use maxwell_core::{EventEnvelope, MarketEvent};

use crate::fault::FaultStats;
use crate::history::HistoryStats;

// ============================================================
// Event statistics
// ============================================================

/// Authoritative market event counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventStats {
    pub added: u64,
    pub modified: u64,
    pub cancelled: u64,
    pub deleted: u64,
    pub executed: u64,
    pub other: u64,
}

impl EventStats {
    pub fn record(&mut self, event: &EventEnvelope) {
        let counter = match event.event {
            MarketEvent::OrderAdded(_) => &mut self.added,
            MarketEvent::OrderModified(_) => &mut self.modified,
            MarketEvent::OrderCancelled(_) => &mut self.cancelled,
            MarketEvent::OrderDeleted(_) => &mut self.deleted,
            MarketEvent::OrderExecuted(_) => &mut self.executed,
            _ => &mut self.other,
        };

        *counter = counter.saturating_add(1);
    }

    #[must_use]
    pub const fn total(&self) -> u64 {
        self.added
            .saturating_add(self.modified)
            .saturating_add(self.cancelled)
            .saturating_add(self.deleted)
            .saturating_add(self.executed)
            .saturating_add(self.other)
    }
}

// ============================================================
// Delivery statistics
// ============================================================

/// Delivery attempts and successful message admissions.
///
/// A successful delivery means that the receiving sink
/// accepted the message, not necessarily that a remote
/// client received or processed it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryStats {
    pub accepted: u64,
    pub blocked: u64,
    pub failed: u64,
}

impl DeliveryStats {
    pub fn record_accepted(&mut self) {
        self.accepted = self.accepted.saturating_add(1);
    }

    pub fn record_blocked(&mut self) {
        self.blocked = self.blocked.saturating_add(1);
    }

    pub fn record_failed(&mut self) {
        self.failed = self.failed.saturating_add(1);
    }

    #[must_use]
    pub const fn attempts(&self) -> u64 {
        self.accepted
            .saturating_add(self.blocked)
            .saturating_add(self.failed)
    }

    #[must_use]
    pub fn blocked_ratio(&self) -> f64 {
        ratio(self.blocked, self.attempts())
    }
}

// ============================================================
// Recovery statistics
// ============================================================

/// Authoritative recovery service statistics.
///
/// Prepared recovery responses are counted separately
/// from their eventual transport delivery.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryStats {
    pub retransmission_requests: u64,
    pub retransmission_ranges_prepared: u64,
    pub retransmission_events_prepared: u64,
    pub retransmission_unavailable: u64,
    pub retransmission_failed: u64,

    pub snapshot_requests: u64,
    pub snapshots_prepared: u64,
    pub snapshot_entries_prepared: u64,
    pub snapshot_failed: u64,
}

impl RecoveryStats {
    pub fn record_retransmission_request(&mut self) {
        self.retransmission_requests = self.retransmission_requests.saturating_add(1);
    }

    pub fn record_retransmission_prepared(&mut self, event_count: usize) {
        self.retransmission_ranges_prepared = self.retransmission_ranges_prepared.saturating_add(1);

        self.retransmission_events_prepared = self
            .retransmission_events_prepared
            .saturating_add(event_count as u64);
    }

    pub fn record_retransmission_unavailable(&mut self) {
        self.retransmission_unavailable = self.retransmission_unavailable.saturating_add(1);
    }

    pub fn record_retransmission_failed(&mut self) {
        self.retransmission_failed = self.retransmission_failed.saturating_add(1);
    }

    pub fn record_snapshot_request(&mut self) {
        self.snapshot_requests = self.snapshot_requests.saturating_add(1);
    }

    pub fn record_snapshot_prepared(&mut self, entry_count: usize) {
        self.snapshots_prepared = self.snapshots_prepared.saturating_add(1);

        self.snapshot_entries_prepared = self
            .snapshot_entries_prepared
            .saturating_add(entry_count as u64);
    }

    pub fn record_snapshot_failed(&mut self) {
        self.snapshot_failed = self.snapshot_failed.saturating_add(1);
    }
}

// ============================================================
// Runtime statistics
// ============================================================

/// Mutable owner-local simulator statistics.
#[derive(Debug, Default)]
pub struct SimulatorStats {
    events: EventStats,
    delivery: DeliveryStats,
    recovery: RecoveryStats,

    generation_blocked: u64,
    service_iterations: u64,

    current_resting_orders: usize,
    peak_resting_orders: usize,

    started_at: Option<Instant>,
    finished_at: Option<Instant>,
}

impl SimulatorStats {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // --------------------------------------------------------
    // Timing
    // --------------------------------------------------------

    /// Starts wall-clock timing.
    ///
    /// Repeated calls do not reset the original origin.
    pub fn start(&mut self) {
        if self.started_at.is_none() {
            self.started_at = Some(Instant::now());
        }
    }

    /// Stops wall-clock timing.
    ///
    /// Repeated calls preserve the first completion time.
    pub fn finish(&mut self) {
        if self.started_at.is_some() && self.finished_at.is_none() {
            self.finished_at = Some(Instant::now());
        }
    }

    #[must_use]
    pub const fn is_started(&self) -> bool {
        self.started_at.is_some()
    }

    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.finished_at.is_some()
    }

    #[must_use]
    pub fn elapsed(&self) -> Duration {
        let Some(started_at) = self.started_at else {
            return Duration::ZERO;
        };

        let finished_at = self.finished_at.unwrap_or_else(Instant::now);

        finished_at.saturating_duration_since(started_at)
    }

    // --------------------------------------------------------
    // Authoritative generation
    // --------------------------------------------------------

    pub fn record_generated(&mut self, event: &EventEnvelope) {
        self.events.record(event);
    }

    pub fn record_generation_blocked(&mut self) {
        self.generation_blocked = self.generation_blocked.saturating_add(1);
    }

    pub fn record_service_iteration(&mut self) {
        self.service_iterations = self.service_iterations.saturating_add(1);
    }

    pub fn observe_resting_orders(&mut self, count: usize) {
        self.current_resting_orders = count;

        self.peak_resting_orders = self.peak_resting_orders.max(count);
    }

    // --------------------------------------------------------
    // Delivery and recovery
    // --------------------------------------------------------

    pub fn delivery_mut(&mut self) -> &mut DeliveryStats {
        &mut self.delivery
    }

    pub fn recovery_mut(&mut self) -> &mut RecoveryStats {
        &mut self.recovery
    }

    #[must_use]
    pub const fn events(&self) -> EventStats {
        self.events
    }

    #[must_use]
    pub const fn delivery(&self) -> DeliveryStats {
        self.delivery
    }

    #[must_use]
    pub const fn recovery(&self) -> RecoveryStats {
        self.recovery
    }

    // --------------------------------------------------------
    // Immutable statistics report
    // --------------------------------------------------------

    #[must_use]
    pub fn report(&self, faults: FaultStats, history: HistoryStats) -> StatsReport {
        StatsReport {
            events: self.events,
            delivery: self.delivery,
            recovery: self.recovery,
            faults,
            history,

            generation_blocked: self.generation_blocked,
            service_iterations: self.service_iterations,

            current_resting_orders: self.current_resting_orders,
            peak_resting_orders: self.peak_resting_orders,

            elapsed: self.elapsed(),
        }
    }
}

// ============================================================
// Immutable statistics report
// ============================================================

/// Point-in-time simulator statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatsReport {
    pub events: EventStats,
    pub delivery: DeliveryStats,
    pub recovery: RecoveryStats,
    pub faults: FaultStats,
    pub history: HistoryStats,

    pub generation_blocked: u64,
    pub service_iterations: u64,

    pub current_resting_orders: usize,
    pub peak_resting_orders: usize,

    pub elapsed: Duration,
}

impl StatsReport {
    #[must_use]
    pub const fn generated(&self) -> u64 {
        self.events.total()
    }

    #[must_use]
    pub fn generated_per_second(&self) -> f64 {
        rate(self.generated(), self.elapsed)
    }

    #[must_use]
    pub fn delivered_per_second(&self) -> f64 {
        rate(self.delivery.accepted, self.elapsed)
    }

    #[must_use]
    pub fn drop_ratio(&self) -> f64 {
        ratio(self.faults.dropped, self.faults.accepted)
    }

    #[must_use]
    pub fn duplicate_ratio(&self) -> f64 {
        ratio(self.faults.duplicated, self.faults.accepted)
    }

    #[must_use]
    pub fn reorder_ratio(&self) -> f64 {
        ratio(self.faults.reordered, self.faults.accepted)
    }

    /// Number of delivery messages expected after all
    /// delayed events have been released.
    #[must_use]
    pub fn expected_delivery_messages(&self) -> u128 {
        u128::from(self.faults.accepted).saturating_sub(u128::from(self.faults.dropped))
            + u128::from(self.faults.duplicated)
    }

    /// Checks cumulative accounting after a successful
    /// complete run and final queue drain.
    #[must_use]
    pub fn is_fully_accounted(&self) -> bool {
        let classified = u128::from(self.faults.passed)
            + u128::from(self.faults.dropped)
            + u128::from(self.faults.duplicated)
            + u128::from(self.faults.reordered);

        self.generated() == self.history.recorded
            && self.generated() == self.faults.accepted
            && classified == u128::from(self.faults.accepted)
            && self.expected_delivery_messages() == u128::from(self.faults.enqueued)
            && self.faults.enqueued == self.faults.dequeued
            && self.faults.dequeued == self.delivery.accepted
    }
}

// ============================================================
// Human-readable reporting
// ============================================================

impl fmt::Display for StatsReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Maxwell Simulator Statistics")?;
        writeln!(f, "============================")?;

        writeln!(
            f,
            "Elapsed seconds:          {:.6}",
            self.elapsed.as_secs_f64(),
        )?;

        writeln!(f, "Generated events:         {}", self.generated(),)?;

        writeln!(
            f,
            "Generated events/sec:     {:.2}",
            self.generated_per_second(),
        )?;

        writeln!(f, "Delivered messages:       {}", self.delivery.accepted,)?;

        writeln!(
            f,
            "Delivered messages/sec:   {:.2}",
            self.delivered_per_second(),
        )?;

        writeln!(f)?;
        writeln!(f, "Authoritative events")?;

        writeln!(f, "  Added:                  {}", self.events.added)?;
        writeln!(f, "  Modified:               {}", self.events.modified)?;
        writeln!(f, "  Cancelled:              {}", self.events.cancelled)?;
        writeln!(f, "  Deleted:                {}", self.events.deleted)?;
        writeln!(f, "  Executed:               {}", self.events.executed)?;
        writeln!(f, "  Other:                  {}", self.events.other)?;

        writeln!(f)?;
        writeln!(f, "Delivery faults")?;

        writeln!(
            f,
            "  Dropped:                {} ({:.2}%)",
            self.faults.dropped,
            self.drop_ratio() * 100.0,
        )?;

        writeln!(
            f,
            "  Duplicated:             {} ({:.2}%)",
            self.faults.duplicated,
            self.duplicate_ratio() * 100.0,
        )?;

        writeln!(
            f,
            "  Reordered:              {} ({:.2}%)",
            self.faults.reordered,
            self.reorder_ratio() * 100.0,
        )?;

        writeln!(f, "  Delivery blocked:       {}", self.delivery.blocked,)?;

        writeln!(f, "  Delivery failed:        {}", self.delivery.failed,)?;

        writeln!(f, "  Generation blocked:     {}", self.generation_blocked,)?;

        writeln!(f)?;
        writeln!(f, "Recovery")?;

        writeln!(f, "  History recorded:       {}", self.history.recorded,)?;

        writeln!(f, "  History evicted:        {}", self.history.evicted,)?;

        writeln!(
            f,
            "  Retransmission requests: {}",
            self.recovery.retransmission_requests,
        )?;

        writeln!(
            f,
            "  Ranges prepared:        {}",
            self.recovery.retransmission_ranges_prepared,
        )?;

        writeln!(
            f,
            "  Replay events prepared:  {}",
            self.recovery.retransmission_events_prepared,
        )?;

        writeln!(
            f,
            "  History unavailable:    {}",
            self.recovery.retransmission_unavailable,
        )?;

        writeln!(
            f,
            "  Retransmission failed:  {}",
            self.recovery.retransmission_failed,
        )?;

        writeln!(
            f,
            "  Snapshot requests:      {}",
            self.recovery.snapshot_requests,
        )?;

        writeln!(
            f,
            "  Snapshots prepared:     {}",
            self.recovery.snapshots_prepared,
        )?;

        writeln!(
            f,
            "  Snapshot entries:       {}",
            self.recovery.snapshot_entries_prepared,
        )?;

        writeln!(
            f,
            "  Snapshot failed:        {}",
            self.recovery.snapshot_failed,
        )?;

        writeln!(f)?;
        writeln!(f, "Runtime")?;

        writeln!(f, "  Service iterations:     {}", self.service_iterations,)?;

        writeln!(
            f,
            "  Resting orders:         {}",
            self.current_resting_orders,
        )?;

        writeln!(f, "  Peak resting orders:    {}", self.peak_resting_orders,)?;

        write!(f, "  Fully accounted:        {}", self.is_fully_accounted(),)
    }
}

// ============================================================
// Numeric helpers
// ============================================================

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }

    numerator as f64 / denominator as f64
}

fn rate(count: u64, elapsed: Duration) -> f64 {
    if elapsed.is_zero() {
        return 0.0;
    }

    count as f64 / elapsed.as_secs_f64()
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> StatsReport {
        StatsReport {
            events: EventStats::default(),
            delivery: DeliveryStats::default(),
            recovery: RecoveryStats::default(),
            faults: FaultStats::default(),
            history: HistoryStats::default(),

            generation_blocked: 0,
            service_iterations: 0,

            current_resting_orders: 0,
            peak_resting_orders: 0,

            elapsed: Duration::ZERO,
        }
    }

    #[test]
    fn empty_statistics_are_zero() {
        let stats = SimulatorStats::new();

        assert_eq!(stats.events().total(), 0);
        assert_eq!(stats.delivery().attempts(), 0);
        assert_eq!(stats.elapsed(), Duration::ZERO);
    }

    #[test]
    fn event_counts_are_aggregated() {
        let events = EventStats {
            added: 100,
            modified: 50,
            cancelled: 25,
            deleted: 15,
            executed: 10,
            other: 0,
        };

        assert_eq!(events.total(), 200);
    }

    #[test]
    fn zero_duration_has_zero_throughput() {
        let mut report = report();

        report.events.added = 100;

        assert_eq!(report.generated_per_second(), 0.0);
        assert_eq!(report.delivered_per_second(), 0.0);
    }

    #[test]
    fn throughput_uses_wall_clock_duration() {
        let mut report = report();

        report.events.added = 100_000;
        report.delivery.accepted = 90_000;
        report.elapsed = Duration::from_secs(2);

        assert_eq!(report.generated_per_second(), 50_000.0,);

        assert_eq!(report.delivered_per_second(), 45_000.0,);
    }

    #[test]
    fn delivery_attempts_include_blocked_and_failed() {
        let mut stats = DeliveryStats::default();

        stats.record_accepted();
        stats.record_accepted();
        stats.record_blocked();
        stats.record_failed();

        assert_eq!(stats.attempts(), 4);
        assert_eq!(stats.blocked_ratio(), 0.25);
    }

    #[test]
    fn zero_denominator_ratios_are_zero() {
        let report = report();

        assert_eq!(report.drop_ratio(), 0.0);
        assert_eq!(report.duplicate_ratio(), 0.0);
        assert_eq!(report.reorder_ratio(), 0.0);
    }

    #[test]
    fn fault_ratios_use_authoritative_admissions() {
        let mut report = report();

        report.faults.accepted = 100;
        report.faults.dropped = 10;
        report.faults.duplicated = 20;
        report.faults.reordered = 30;

        assert_eq!(report.drop_ratio(), 0.1);
        assert_eq!(report.duplicate_ratio(), 0.2);
        assert_eq!(report.reorder_ratio(), 0.3);
    }

    #[test]
    fn peak_resting_orders_never_decrease() {
        let mut stats = SimulatorStats::new();

        stats.observe_resting_orders(10);
        stats.observe_resting_orders(100);
        stats.observe_resting_orders(25);

        let report = stats.report(FaultStats::default(), HistoryStats::default());

        assert_eq!(report.current_resting_orders, 25);
        assert_eq!(report.peak_resting_orders, 100);
    }

    #[test]
    fn recovery_statistics_distinguish_requests_and_events() {
        let mut stats = RecoveryStats::default();

        stats.record_retransmission_request();
        stats.record_retransmission_prepared(10);

        stats.record_retransmission_request();
        stats.record_retransmission_unavailable();

        assert_eq!(stats.retransmission_requests, 2);
        assert_eq!(stats.retransmission_ranges_prepared, 1);
        assert_eq!(stats.retransmission_events_prepared, 10);
        assert_eq!(stats.retransmission_unavailable, 1);
    }

    #[test]
    fn snapshot_statistics_track_prepared_entries() {
        let mut stats = RecoveryStats::default();

        stats.record_snapshot_request();
        stats.record_snapshot_prepared(100);

        stats.record_snapshot_request();
        stats.record_snapshot_failed();

        assert_eq!(stats.snapshot_requests, 2);
        assert_eq!(stats.snapshots_prepared, 1);
        assert_eq!(stats.snapshot_entries_prepared, 100);
        assert_eq!(stats.snapshot_failed, 1);
    }

    #[test]
    fn completed_run_accounting_includes_faults() {
        let mut report = report();

        report.events.added = 100;
        report.history.recorded = 100;

        report.faults.accepted = 100;
        report.faults.passed = 60;
        report.faults.dropped = 10;
        report.faults.duplicated = 20;
        report.faults.reordered = 10;

        report.faults.enqueued = 110;
        report.faults.dequeued = 110;
        report.delivery.accepted = 110;

        assert_eq!(report.expected_delivery_messages(), 110,);

        assert!(report.is_fully_accounted());
    }

    #[test]
    fn undrained_delivery_is_not_fully_accounted() {
        let mut report = report();

        report.events.added = 10;
        report.history.recorded = 10;
        report.faults.accepted = 10;
        report.faults.passed = 10;
        report.faults.enqueued = 10;
        report.faults.dequeued = 9;
        report.delivery.accepted = 9;

        assert!(!report.is_fully_accounted());
    }

    #[test]
    fn finish_freezes_elapsed_time() {
        let mut stats = SimulatorStats::new();

        stats.start();
        stats.finish();

        let elapsed = stats.elapsed();

        stats.finish();
        stats.start();

        assert_eq!(stats.elapsed(), elapsed);
        assert!(stats.is_started());
        assert!(stats.is_finished());
    }

    #[test]
    fn report_has_human_readable_output() {
        let report = report().to_string();

        assert!(report.contains("Maxwell Simulator Statistics"));
        assert!(report.contains("Generated events"));
        assert!(report.contains("Recovery"));
        assert!(report.contains("Fully accounted"));
    }
}
