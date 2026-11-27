//! Application-level operational instrumentation.
//!
//! Metrics are owned by the server execution context.
//! No atomic operations or shared mutable state are needed.
//!
//! Metric registration occurs during initialization.
//! Prometheus formatting belongs outside the event
//! processing path.

use std::io::Write;

use maxwell_metrics::{MetricId, MetricSnapshot, MetricsRegistry, write_prometheus};

use crate::error::ServerResult;
use crate::pipeline::PublicationOutcome;
use crate::recovery::RecoveryOutcome;
use crate::state::ServerState;

// ============================================================
// Metric configuration
// ============================================================

/// Histogram bucket capacity reserved by the registry type.
///
/// Histograms are not registered during the initial
/// application implementation.
pub const HISTOGRAM_BUCKETS: usize = 16;

/// Number of application metrics registered at startup.
pub const METRIC_COUNT: usize = 21;

pub type ServerMetricSnapshot = MetricSnapshot<HISTOGRAM_BUCKETS>;

type Registry = MetricsRegistry<HISTOGRAM_BUCKETS>;

// ============================================================
// Metric identifiers
// ============================================================

/// Stable registry identifiers assigned during initialization.
struct MetricIds {
    upstream_received: MetricId,
    upstream_consumed: MetricId,
    upstream_duplicates: MetricId,
    upstream_buffered: MetricId,
    upstream_blocked: MetricId,
    upstream_errors: MetricId,

    publications_accepted: MetricId,
    publications_blocked: MetricId,
    publication_retries: MetricId,

    recovery_replays: MetricId,
    recovery_required: MetricId,
    recovery_completed: MetricId,

    downstream_messages: MetricId,
    downstream_blocked: MetricId,

    snapshots_accepted: MetricId,
    snapshots_rejected: MetricId,
    snapshot_entries: MetricId,

    service_iterations: MetricId,

    connected_sessions: MetricId,
    synchronized: MetricId,
    lifecycle: MetricId,
}

impl MetricIds {
    fn register(registry: &mut Registry) -> ServerResult<Self> {
        Ok(Self {
            upstream_received: registry.register_counter("maxwell_upstream_received_total")?,

            upstream_consumed: registry.register_counter("maxwell_upstream_consumed_total")?,

            upstream_duplicates: registry.register_counter("maxwell_upstream_duplicates_total")?,

            upstream_buffered: registry.register_counter("maxwell_upstream_buffered_total")?,

            upstream_blocked: registry.register_counter("maxwell_upstream_blocked_total")?,

            upstream_errors: registry.register_counter("maxwell_upstream_errors_total")?,

            publications_accepted: registry
                .register_counter("maxwell_publications_accepted_total")?,

            publications_blocked: registry
                .register_counter("maxwell_publications_blocked_total")?,

            publication_retries: registry.register_counter("maxwell_publication_retries_total")?,

            recovery_replays: registry
                .register_counter("maxwell_recovery_replay_operations_total")?,

            recovery_required: registry.register_counter("maxwell_recovery_required_total")?,

            recovery_completed: registry.register_counter("maxwell_recovery_completed_total")?,

            downstream_messages: registry
                .register_counter("maxwell_downstream_messages_accepted_total")?,

            downstream_blocked: registry.register_counter("maxwell_downstream_blocked_total")?,

            snapshots_accepted: registry.register_counter("maxwell_snapshots_accepted_total")?,

            snapshots_rejected: registry.register_counter("maxwell_snapshots_rejected_total")?,

            snapshot_entries: registry.register_counter("maxwell_snapshot_entries_total")?,

            service_iterations: registry.register_counter("maxwell_service_iterations_total")?,

            connected_sessions: registry.register_gauge("maxwell_connected_sessions")?,

            synchronized: registry.register_gauge("maxwell_synchronized")?,

            lifecycle: registry.register_gauge("maxwell_server_lifecycle")?,
        })
    }
}

// ============================================================
// Server metrics
// ============================================================

/// Single-owner application instrumentation.
///
/// Registry identifiers remain private so that callers
/// cannot accidentally record values against unrelated
/// metric types.
pub struct ServerMetrics {
    registry: Registry,
    ids: MetricIds,
}

impl ServerMetrics {
    /// Allocates and registers all application metrics.
    pub fn new() -> ServerResult<Self> {
        let mut registry = Registry::new(METRIC_COUNT)?;

        let ids = MetricIds::register(&mut registry)?;

        Ok(Self { registry, ids })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.registry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }

    // --------------------------------------------------------
    // Upstream instrumentation
    // --------------------------------------------------------

    /// Records one upstream admission attempt.
    ///
    /// Repeated attempts for blocked input are counted
    /// separately. This is not a unique-event counter.
    pub fn record_upstream_received(&mut self) -> ServerResult<()> {
        self.registry.increment(self.ids.upstream_received)?;

        Ok(())
    }

    /// Records an upstream processing failure.
    pub fn record_upstream_error(&mut self) -> ServerResult<()> {
        self.registry.increment(self.ids.upstream_errors)?;

        Ok(())
    }

    /// Records the outcome of an upstream admission.
    ///
    /// Recovery replay counters represent operations,
    /// not the number of individual replayed events.
    pub fn record_recovery_outcome(&mut self, outcome: RecoveryOutcome) -> ServerResult<()> {
        if !matches!(outcome, RecoveryOutcome::InputBlocked) {
            self.registry.increment(self.ids.upstream_consumed)?;
        }

        match outcome {
            RecoveryOutcome::Published { .. } => {
                self.registry.increment(self.ids.publications_accepted)?;
            }

            RecoveryOutcome::PublicationPending => {
                self.registry.increment(self.ids.publications_blocked)?;
            }

            RecoveryOutcome::InputBlocked => {
                self.registry.increment(self.ids.upstream_blocked)?;
            }

            RecoveryOutcome::Buffered => {
                self.registry.increment(self.ids.upstream_buffered)?;
            }

            RecoveryOutcome::Duplicate => {
                self.registry.increment(self.ids.upstream_duplicates)?;
            }

            RecoveryOutcome::Replayed => {
                self.registry.increment(self.ids.recovery_replays)?;
            }

            RecoveryOutcome::Resynchronized => {
                self.registry.increment(self.ids.recovery_completed)?;
            }

            RecoveryOutcome::RecoveryRequired => {
                self.registry.increment(self.ids.recovery_required)?;
            }
        }

        Ok(())
    }

    // --------------------------------------------------------
    // Publication instrumentation
    // --------------------------------------------------------

    /// Records an attempt to retry an outstanding
    /// publication.
    ///
    /// Idle observations are not counted as retries.
    pub fn record_publication_retry(&mut self, outcome: PublicationOutcome) -> ServerResult<()> {
        match outcome {
            PublicationOutcome::Published { .. } => {
                self.registry.increment(self.ids.publication_retries)?;

                self.registry.increment(self.ids.publications_accepted)?;
            }

            PublicationOutcome::Blocked => {
                self.registry.increment(self.ids.publication_retries)?;

                self.registry.increment(self.ids.publications_blocked)?;
            }

            PublicationOutcome::Idle => {}
        }

        Ok(())
    }

    // --------------------------------------------------------
    // Downstream instrumentation
    // --------------------------------------------------------

    /// Records gateway messages accepted by downstream
    /// transports.
    ///
    /// Acceptance does not imply that bytes have already
    /// reached the remote client.
    pub fn record_downstream_service(
        &mut self,
        accepted: usize,
        blocked: bool,
    ) -> ServerResult<()> {
        let accepted = u64::try_from(accepted).unwrap_or(u64::MAX);

        self.registry.add(self.ids.downstream_messages, accepted)?;

        if blocked {
            self.registry.increment(self.ids.downstream_blocked)?;
        }

        Ok(())
    }

    // --------------------------------------------------------
    // Snapshot instrumentation
    // --------------------------------------------------------

    /// Records a successfully accepted and activated
    /// downstream snapshot.
    pub fn record_snapshot_accepted(&mut self, entry_count: usize) -> ServerResult<()> {
        self.registry.increment(self.ids.snapshots_accepted)?;

        self.registry.add(
            self.ids.snapshot_entries,
            u64::try_from(entry_count).unwrap_or(u64::MAX),
        )?;

        Ok(())
    }

    /// Records a snapshot rejected by its destination.
    pub fn record_snapshot_rejected(&mut self) -> ServerResult<()> {
        self.registry.increment(self.ids.snapshots_rejected)?;

        Ok(())
    }

    // --------------------------------------------------------
    // Event loop instrumentation
    // --------------------------------------------------------

    pub fn record_service_iteration(&mut self) -> ServerResult<()> {
        self.registry.increment(self.ids.service_iterations)?;

        Ok(())
    }

    /// Updates point-in-time operational gauges.
    pub fn update_status(
        &mut self,
        lifecycle: ServerState,
        connected_sessions: usize,
        synchronized: bool,
    ) -> ServerResult<()> {
        let session_count = i64::try_from(connected_sessions).unwrap_or(i64::MAX);

        self.registry
            .set_gauge(self.ids.connected_sessions, session_count)?;

        self.registry
            .set_gauge(self.ids.synchronized, i64::from(synchronized))?;

        self.registry
            .set_gauge(self.ids.lifecycle, lifecycle_code(lifecycle))?;

        Ok(())
    }

    // --------------------------------------------------------
    // Metrics observation
    // --------------------------------------------------------

    /// Returns an owned snapshot of one registered metric.
    pub fn snapshot(&self, name: &str) -> ServerResult<Option<ServerMetricSnapshot>> {
        let Some(id) = self.registry.find(name) else {
            return Ok(None);
        };

        Ok(Some(self.registry.snapshot(id)?))
    }

    /// Iterates over current metric snapshots.
    pub fn snapshots(&self) -> impl ExactSizeIterator<Item = ServerMetricSnapshot> + '_ {
        self.registry.snapshots()
    }

    /// Writes Prometheus-compatible metric exposition.
    ///
    /// This is a control-path operation and may perform
    /// blocking I/O depending on the supplied writer.
    pub fn write_prometheus<W>(&self, writer: &mut W) -> ServerResult<()>
    where
        W: Write + ?Sized,
    {
        write_prometheus(writer, self.registry.snapshots())?;

        Ok(())
    }
}

// ============================================================
// Stable lifecycle encoding
// ============================================================

/// Numeric lifecycle representation used by metrics.
///
/// These values are an operational interface and should
/// remain stable across compatible releases.
#[must_use]
pub const fn lifecycle_code(state: ServerState) -> i64 {
    match state {
        ServerState::Initialized => 0,
        ServerState::Running => 1,
        ServerState::Draining => 2,
        ServerState::Stopped => 3,
        ServerState::Failed => 4,
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn counter(metrics: &ServerMetrics, name: &str) -> u64 {
        match metrics.snapshot(name).unwrap().unwrap() {
            MetricSnapshot::Counter { value, .. } => value,

            other => {
                panic!("expected counter, received {other:?}");
            }
        }
    }

    fn gauge(metrics: &ServerMetrics, name: &str) -> i64 {
        match metrics.snapshot(name).unwrap().unwrap() {
            MetricSnapshot::Gauge { value, .. } => value,

            other => {
                panic!("expected gauge, received {other:?}");
            }
        }
    }

    #[test]
    fn registers_all_application_metrics() {
        let metrics = ServerMetrics::new().unwrap();

        assert_eq!(metrics.len(), METRIC_COUNT);
        assert_eq!(metrics.snapshots().len(), METRIC_COUNT);
        assert!(!metrics.is_empty());
    }

    #[test]
    fn unknown_metric_returns_none() {
        let metrics = ServerMetrics::new().unwrap();

        assert!(metrics.snapshot("unknown_metric").unwrap().is_none());
    }

    #[test]
    fn records_upstream_admission_attempts() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics.record_upstream_received().unwrap();
        metrics.record_upstream_received().unwrap();

        assert_eq!(counter(&metrics, "maxwell_upstream_received_total",), 2,);
    }

    #[test]
    fn records_successful_publication() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics
            .record_recovery_outcome(RecoveryOutcome::Published { sequence: 1 })
            .unwrap();

        assert_eq!(counter(&metrics, "maxwell_publications_accepted_total",), 1,);

        assert_eq!(counter(&metrics, "maxwell_upstream_consumed_total",), 1,);
    }

    #[test]
    fn blocked_input_is_not_consumed() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics
            .record_recovery_outcome(RecoveryOutcome::InputBlocked)
            .unwrap();

        assert_eq!(counter(&metrics, "maxwell_upstream_blocked_total",), 1,);

        assert_eq!(counter(&metrics, "maxwell_upstream_consumed_total",), 0,);
    }

    #[test]
    fn publication_retry_does_not_count_input_twice() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics
            .record_recovery_outcome(RecoveryOutcome::PublicationPending)
            .unwrap();

        metrics
            .record_publication_retry(PublicationOutcome::Blocked)
            .unwrap();

        metrics
            .record_publication_retry(PublicationOutcome::Published { sequence: 1 })
            .unwrap();

        assert_eq!(counter(&metrics, "maxwell_upstream_consumed_total",), 1,);

        assert_eq!(counter(&metrics, "maxwell_publications_accepted_total",), 1,);

        assert_eq!(counter(&metrics, "maxwell_publications_blocked_total",), 2,);

        assert_eq!(counter(&metrics, "maxwell_publication_retries_total",), 2,);
    }

    #[test]
    fn idle_retry_does_not_increment_counter() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics
            .record_publication_retry(PublicationOutcome::Idle)
            .unwrap();

        assert_eq!(counter(&metrics, "maxwell_publication_retries_total",), 0,);
    }

    #[test]
    fn records_duplicate_events() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics
            .record_recovery_outcome(RecoveryOutcome::Duplicate)
            .unwrap();

        assert_eq!(counter(&metrics, "maxwell_upstream_duplicates_total",), 1,);
    }

    #[test]
    fn records_recovery_completion() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics
            .record_recovery_outcome(RecoveryOutcome::Resynchronized)
            .unwrap();

        assert_eq!(counter(&metrics, "maxwell_recovery_completed_total",), 1,);
    }

    #[test]
    fn records_downstream_service() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics.record_downstream_service(10, true).unwrap();

        metrics.record_downstream_service(5, false).unwrap();

        assert_eq!(
            counter(&metrics, "maxwell_downstream_messages_accepted_total",),
            15,
        );

        assert_eq!(counter(&metrics, "maxwell_downstream_blocked_total",), 1,);
    }

    #[test]
    fn records_snapshot_statistics() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics.record_snapshot_accepted(100).unwrap();
        metrics.record_snapshot_accepted(50).unwrap();
        metrics.record_snapshot_rejected().unwrap();

        assert_eq!(counter(&metrics, "maxwell_snapshots_accepted_total",), 2,);

        assert_eq!(counter(&metrics, "maxwell_snapshot_entries_total",), 150,);

        assert_eq!(counter(&metrics, "maxwell_snapshots_rejected_total",), 1,);
    }

    #[test]
    fn updates_operational_gauges() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics
            .update_status(ServerState::Running, 12, true)
            .unwrap();

        assert_eq!(gauge(&metrics, "maxwell_connected_sessions",), 12,);

        assert_eq!(gauge(&metrics, "maxwell_synchronized",), 1,);

        assert_eq!(gauge(&metrics, "maxwell_server_lifecycle",), 1,);
    }

    #[test]
    fn synchronization_gauge_can_be_invalidated() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics
            .update_status(ServerState::Running, 1, true)
            .unwrap();

        metrics
            .update_status(ServerState::Running, 1, false)
            .unwrap();

        assert_eq!(gauge(&metrics, "maxwell_synchronized",), 0,);
    }

    #[test]
    fn lifecycle_codes_are_stable() {
        assert_eq!(lifecycle_code(ServerState::Initialized), 0,);

        assert_eq!(lifecycle_code(ServerState::Running), 1,);

        assert_eq!(lifecycle_code(ServerState::Draining), 2,);

        assert_eq!(lifecycle_code(ServerState::Stopped), 3,);

        assert_eq!(lifecycle_code(ServerState::Failed), 4,);
    }

    #[test]
    fn exports_prometheus_metrics() {
        let mut metrics = ServerMetrics::new().unwrap();

        metrics.record_service_iteration().unwrap();

        let mut output = Vec::new();

        metrics.write_prometheus(&mut output).unwrap();

        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("# TYPE maxwell_service_iterations_total counter"));

        assert!(output.contains("maxwell_service_iterations_total 1"));
    }
}
