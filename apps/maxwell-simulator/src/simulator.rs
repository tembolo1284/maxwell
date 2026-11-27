//! Integrated single-owner simulator runtime.
//!
//! Coordinates authoritative generation, retained history,
//! delivery fault injection, pacing, recovery, and statistics.
//!
//! The simulator does not depend on a particular network
//! transport or downstream consumer.

use std::fmt;

use maxwell_core::{
    EventEnvelope, FeedId, MonotonicTimestamp, SequenceNumber,
};

use crate::clock::Pacer;
use crate::config::SimulatorConfig;
use crate::error::{SimulatorError, SimulatorResult};
use crate::fault::FaultInjector;
use crate::generator::EventGenerator;
use crate::history::{EventHistory, HistoryRange};
use crate::market::Market;
use crate::snapshot::MarketSnapshot;
use crate::stats::{SimulatorStats, StatsReport};

// ============================================================
// Runtime limits
// ============================================================

/// Explicit storage and per-iteration processing limits.
///
/// These limits allow the runtime to be embedded in an
/// external event loop without unbounded service work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    pub history_capacity: usize,
    pub delivery_capacity: usize,
    pub snapshot_capacity: usize,
    pub generation_budget: usize,
    pub delivery_budget: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            history_capacity: 65_536,
            delivery_capacity: 4_096,
            snapshot_capacity: 65_536,
            generation_budget: 256,
            delivery_budget: 256,
        }
    }
}

impl RuntimeLimits {
    pub fn validate(&self) -> SimulatorResult<()> {
        if self.history_capacity == 0 {
            return Err(SimulatorError::InvalidConfig(
                "history capacity must be positive",
            ));
        }

        if self.delivery_capacity == 0 {
            return Err(SimulatorError::InvalidConfig(
                "delivery capacity must be positive",
            ));
        }

        if self.snapshot_capacity == 0 {
            return Err(SimulatorError::InvalidConfig(
                "snapshot capacity must be positive",
            ));
        }

        if self.generation_budget == 0 {
            return Err(SimulatorError::InvalidConfig(
                "generation budget must be positive",
            ));
        }

        if self.delivery_budget == 0 {
            return Err(SimulatorError::InvalidConfig(
                "delivery budget must be positive",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Simulator lifecycle
// ============================================================

/// Simulator lifecycle state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimulatorState {
    #[default]
    Initialized,
    Running,
    Draining,
    Completed,
    Failed,
}

impl SimulatorState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }

    #[must_use]
    pub const fn can_generate(self) -> bool {
        matches!(self, Self::Running)
    }

    #[must_use]
    pub const fn can_deliver(self) -> bool {
        matches!(self, Self::Running | Self::Draining)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initialized => "INITIALIZED",
            Self::Running => "RUNNING",
            Self::Draining => "DRAINING",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
        }
    }
}

impl fmt::Display for SimulatorState {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================
// Delivery abstraction
// ============================================================

/// Result of attempting to deliver one event.
///
/// Blocked means that the event was not accepted and
/// may safely be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Accepted,
    Blocked,
}

/// Consumer of simulated upstream market events.
///
/// Implementations must not partially accept an event
/// and subsequently return Blocked.
///
/// Errors are treated as terminal delivery failures.
pub trait EventSink {
    fn try_send(
        &mut self,
        event: &EventEnvelope,
        received_at: MonotonicTimestamp,
    ) -> SimulatorResult<DeliveryOutcome>;
}

/// Sink that accepts and discards all events.
///
/// Useful for measuring simulator-only throughput.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSink;

impl EventSink for NullSink {
    fn try_send(
        &mut self,
        _event: &EventEnvelope,
        _received_at: MonotonicTimestamp,
    ) -> SimulatorResult<DeliveryOutcome> {
        Ok(DeliveryOutcome::Accepted)
    }
}

// ============================================================
// Service report
// ============================================================

/// Result of one bounded simulator service iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceReport {
    pub state: SimulatorState,
    pub generated: usize,
    pub delivered: usize,
    pub released_delayed: usize,

    pub generation_blocked: bool,
    pub delivery_blocked: bool,

    pub pending_delivery: usize,
    pub pending_delayed: usize,
}

impl ServiceReport {
    fn new(state: SimulatorState) -> Self {
        Self {
            state,
            generated: 0,
            delivered: 0,
            released_delayed: 0,

            generation_blocked: false,
            delivery_blocked: false,

            pending_delivery: 0,
            pending_delayed: 0,
        }
    }

    #[must_use]
    pub const fn made_progress(&self) -> bool {
        self.generated != 0
            || self.delivered != 0
            || self.released_delayed != 0
    }

    #[must_use]
    pub const fn is_blocked(&self) -> bool {
        self.generation_blocked || self.delivery_blocked
    }

    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(self.state, SimulatorState::Completed)
    }
}

// ============================================================
// Simulator
// ============================================================

/// Integrated authoritative market simulator.
///
/// All mutable components are owned by this object.
/// External consumers receive events through EventSink.
#[derive(Debug)]
pub struct Simulator {
    config: SimulatorConfig,
    limits: RuntimeLimits,
    state: SimulatorState,

    generator: EventGenerator,
    history: EventHistory,
    faults: FaultInjector,
    pacer: Pacer,
    stats: SimulatorStats,

    delivery_clock_ns: u64,
}

impl Simulator {
    /// Constructs a simulator with explicit runtime limits.
    pub fn new(
        config: SimulatorConfig,
        feed: FeedId,
        limits: RuntimeLimits,
    ) -> SimulatorResult<Self> {
        config.validate()?;
        limits.validate()?;

        let required_snapshot_entries = config
            .market
            .instrument_count
            .checked_add(config.market.max_total_orders()?)
            .ok_or(SimulatorError::ArithmeticOverflow(
                "maximum snapshot entry count",
            ))?;

        if required_snapshot_entries > limits.snapshot_capacity {
            return Err(
                SimulatorError::SnapshotCapacityExceeded {
                    limit: limits.snapshot_capacity,
                    required: required_snapshot_entries,
                },
            );
        }

        let generator = EventGenerator::new(config, feed)?;

        let history = EventHistory::new(
            feed,
            limits.history_capacity,
        )?;

        let faults = FaultInjector::new(
            config.faults,
            config.run.seed,
            limits.delivery_capacity,
        )?;

        let pacer = Pacer::new(config.pacing)?;

        Ok(Self {
            config,
            limits,
            state: SimulatorState::Initialized,

            generator,
            history,
            faults,
            pacer,
            stats: SimulatorStats::new(),

            delivery_clock_ns: config.run.start_timestamp_ns,
        })
    }

    // --------------------------------------------------------
    // Observation
    // --------------------------------------------------------

    #[must_use]
    pub const fn config(&self) -> &SimulatorConfig {
        &self.config
    }

    #[must_use]
    pub const fn limits(&self) -> RuntimeLimits {
        self.limits
    }

    #[must_use]
    pub const fn state(&self) -> SimulatorState {
        self.state
    }

    #[must_use]
    pub const fn generator(&self) -> &EventGenerator {
        &self.generator
    }

    #[must_use]
    pub const fn market(&self) -> &Market {
        self.generator.market()
    }

    #[must_use]
    pub const fn history(&self) -> &EventHistory {
        &self.history
    }

    #[must_use]
    pub const fn faults(&self) -> &FaultInjector {
        &self.faults
    }

    #[must_use]
    pub const fn feed(&self) -> FeedId {
        self.generator.market().feed()
    }

    #[must_use]
    pub const fn generated(&self) -> u64 {
        self.generator.generated()
    }

    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.generator.remaining()
    }

    #[must_use]
    pub fn watermark(&self) -> SequenceNumber {
        self.market()
            .last_sequence()
            .unwrap_or(SequenceNumber::new(0))
    }

    #[must_use]
    pub fn pending_delivery(&self) -> usize {
        self.faults.pending_delivery()
    }

    #[must_use]
    pub fn pending_delayed(&self) -> usize {
        self.faults.pending_delayed()
    }

    #[must_use]
    pub fn stats(&self) -> StatsReport {
        self.stats.report(
            self.faults.stats(),
            self.history.stats(),
        )
    }

    // --------------------------------------------------------
    // Lifecycle
    // --------------------------------------------------------

    /// Starts authoritative event generation.
    pub fn start(&mut self) -> SimulatorResult<()> {
        if self.state != SimulatorState::Initialized {
            return Err(SimulatorError::InvalidState(
                "simulator can only start from initialized state",
            ));
        }

        self.pacer.reset();
        self.stats.start();

        self.state = SimulatorState::Running;

        Ok(())
    }

    /// Stops generating new events and drains existing
    /// outbound and delayed events.
    pub fn request_stop(&mut self) -> SimulatorResult<()> {
        match self.state {
            SimulatorState::Running => {
                self.state = SimulatorState::Draining;

                Ok(())
            }

            SimulatorState::Draining
            | SimulatorState::Completed => Ok(()),

            _ => Err(SimulatorError::InvalidState(
                "simulator cannot drain from current state",
            )),
        }
    }

    fn fail(&mut self) {
        self.state = SimulatorState::Failed;
        self.stats.finish();
    }

    fn complete(&mut self) {
        self.state = SimulatorState::Completed;
        self.stats.finish();
    }

    // --------------------------------------------------------
    // Authoritative generation
    // --------------------------------------------------------

    /// Generates one authoritative event when the fault
    /// injector can accept its complete delivery decision.
    ///
    /// No authoritative mutation occurs when delivery
    /// capacity is insufficient.
    fn generate_one(&mut self) -> SimulatorResult<bool> {
        if !self.state.can_generate() {
            return Ok(false);
        }

        if self.generator.is_finished() {
            self.state = SimulatorState::Draining;

            return Ok(false);
        }

        if !self.faults.can_accept_next() {
            self.stats.record_generation_blocked();

            return Ok(false);
        }

        // Both components must agree on the authoritative
        // sequence before generation mutates market state.
        if self.history.next_expected()
            != self.market().next_expected()
        {
            return Err(
                SimulatorError::InvariantViolation(
                    "market and history cursors differ",
                ),
            );
        }

        self.pacer.wait_for(self.generator.generated())?;

        let event = self.generator
            .generate()?
            .ok_or(SimulatorError::InvariantViolation(
                "generator unexpectedly returned no event",
            ))?;

        // These operations are expected to succeed because
        // sequence and capacity were preflighted above.
        //
        // An unexpected error is terminal: the caller must
        // not regenerate an already applied event.
        self.history.record(event)?;
        self.faults.accept(event)?;

        self.stats.record_generated(&event);

        self.stats.observe_resting_orders(
            self.generator.market().total_orders(),
        );

        if self.generator.is_finished() {
            self.state = SimulatorState::Draining;
        }

        Ok(true)
    }

    // --------------------------------------------------------
    // Delivery
    // --------------------------------------------------------

    /// Computes a monotonic synthetic reception timestamp.
    ///
    /// Exchange timestamps may arrive out of order due
    /// to fault injection. Reception timestamps must not.
    fn next_delivery_timestamp(
        &self,
    ) -> SimulatorResult<MonotonicTimestamp> {
        let incremented = self
            .delivery_clock_ns
            .checked_add(1)
            .ok_or(SimulatorError::TimestampOverflow)?;

        let generated_time =
            self.generator.clock().current_ns();

        Ok(MonotonicTimestamp::new(
            incremented.max(generated_time),
        ))
    }

    /// Delivers up to the specified number of messages.
    ///
    /// An event remains at the queue head until accepted.
    fn deliver<S: EventSink>(
        &mut self,
        sink: &mut S,
        budget: usize,
    ) -> SimulatorResult<(usize, bool)> {
        let mut delivered = 0;

        while delivered < budget {
            let Some(event) = self.faults.front().copied()
            else {
                break;
            };

            let timestamp = self.next_delivery_timestamp()?;

            match sink.try_send(&event, timestamp) {
                Ok(DeliveryOutcome::Accepted) => {
                    self.faults.pop_front();

                    self.delivery_clock_ns = timestamp.raw();

                    self.stats
                        .delivery_mut()
                        .record_accepted();

                    delivered += 1;
                }

                Ok(DeliveryOutcome::Blocked) => {
                    self.stats
                        .delivery_mut()
                        .record_blocked();

                    return Ok((delivered, true));
                }

                Err(error) => {
                    self.stats
                        .delivery_mut()
                        .record_failed();

                    return Err(error);
                }
            }
        }

        Ok((delivered, false))
    }

    // --------------------------------------------------------
    // Bounded service loop
    // --------------------------------------------------------

    /// Performs one bounded service iteration.
    ///
    /// A service iteration prioritizes pending delivery,
    /// generates additional authoritative events, and
    /// then uses any remaining delivery budget.
    ///
    /// Backpressure is reported without failing the
    /// simulator. Other runtime errors are terminal.
    pub fn service<S: EventSink>(
        &mut self,
        sink: &mut S,
    ) -> SimulatorResult<ServiceReport> {
        if self.state == SimulatorState::Completed {
            return Ok(ServiceReport::new(self.state));
        }

        if !self.state.can_deliver() {
            return Err(SimulatorError::InvalidState(
                "simulator is not running or draining",
            ));
        }

        self.stats.record_service_iteration();

        match self.service_inner(sink) {
            Ok(report) => Ok(report),

            Err(error) => {
                self.fail();

                Err(error)
            }
        }
    }

    fn service_inner<S: EventSink>(
        &mut self,
        sink: &mut S,
    ) -> SimulatorResult<ServiceReport> {
        let mut report = ServiceReport::new(self.state);

        // First make room for additional generation.
        let (delivered, blocked) = self.deliver(
            sink,
            self.limits.delivery_budget,
        )?;

        report.delivered = delivered;
        report.delivery_blocked = blocked;

        // Generate a bounded number of authoritative events.
        while report.generated < self.limits.generation_budget
            && self.state.can_generate()
        {
            if !self.generate_one()? {
                report.generation_blocked =
                    self.state.can_generate();

                break;
            }

            report.generated += 1;
        }

        // At end of generation, release withheld events.
        if self.state == SimulatorState::Draining
            && self.faults.pending_delayed() != 0
        {
            match self.faults.flush_delayed() {
                Ok(count) => {
                    report.released_delayed = count;
                }

                Err(error) if error.is_backpressure() => {
                    // Existing outbound messages must be
                    // accepted before delayed events fit.
                }

                Err(error) => return Err(error),
            }
        }

        // Avoid retrying a blocked sink in the same
        // iteration. Preserve the total delivery budget.
        let remaining_budget =
            self.limits.delivery_budget - report.delivered;

        if remaining_budget != 0
            && !report.delivery_blocked
        {
            let (delivered, blocked) =
                self.deliver(sink, remaining_budget)?;

            report.delivered += delivered;
            report.delivery_blocked = blocked;
        }

        if self.state == SimulatorState::Draining
            && self.faults.is_empty()
        {
            if !self.stats().is_fully_accounted() {
                return Err(
                    SimulatorError::InvariantViolation(
                        "completed simulator accounting mismatch",
                    ),
                );
            }

            self.complete();
        }

        report.state = self.state;

        report.pending_delivery =
            self.faults.pending_delivery();

        report.pending_delayed =
            self.faults.pending_delayed();

        Ok(report)
    }

    // --------------------------------------------------------
    // Convenience execution
    // --------------------------------------------------------

    /// Runs until completion or until a service iteration
    /// cannot make progress.
    ///
    /// This method never busy-spins on a blocked sink.
    /// The caller may resume execution after making the
    /// sink ready to accept additional events.
    pub fn run_until_blocked<S: EventSink>(
        &mut self,
        sink: &mut S,
    ) -> SimulatorResult<ServiceReport> {
        if self.state == SimulatorState::Initialized {
            self.start()?;
        }

        loop {
            let report = self.service(sink)?;

            if report.is_completed()
                || !report.made_progress()
            {
                return Ok(report);
            }
        }
    }

    // --------------------------------------------------------
    // Authoritative recovery
    // --------------------------------------------------------

    /// Prepares an allocation-free retransmission range.
    ///
    /// Recovery bypasses the ordinary fault injector.
    /// The borrowed iterator prevents history eviction
    /// while the caller is consuming the response.
    pub fn retransmission(
        &mut self,
        start: SequenceNumber,
        end: SequenceNumber,
    ) -> SimulatorResult<HistoryRange<'_>> {
        self.stats
            .recovery_mut()
            .record_retransmission_request();

        match self.history.range(start, end) {
            Ok(range) => {
                self.stats
                    .recovery_mut()
                    .record_retransmission_prepared(
                        range.len(),
                    );

                Ok(range)
            }

            Err(error) => {
                if error.is_history_unavailable() {
                    self.stats
                        .recovery_mut()
                        .record_retransmission_unavailable();
                } else {
                    self.stats
                        .recovery_mut()
                        .record_retransmission_failed();
                }

                Err(error)
            }
        }
    }

    /// Captures an authoritative snapshot at the current
    /// upstream sequence watermark.
    pub fn snapshot(
        &mut self,
        minimum_watermark: SequenceNumber,
    ) -> SimulatorResult<MarketSnapshot> {
        self.stats
            .recovery_mut()
            .record_snapshot_request();

        let result = MarketSnapshot::capture(
            self.generator.market(),
            self.limits.snapshot_capacity,
        )
        .and_then(|snapshot| {
            snapshot.validate_for(
                self.feed(),
                minimum_watermark,
            )?;

            Ok(snapshot)
        });

        match result {
            Ok(snapshot) => {
                self.stats
                    .recovery_mut()
                    .record_snapshot_prepared(
                        snapshot.entry_count(),
                    );

                Ok(snapshot)
            }

            Err(error) => {
                self.stats
                    .recovery_mut()
                    .record_snapshot_failed();

                Err(error)
            }
        }
    }

    // --------------------------------------------------------
    // Diagnostics
    // --------------------------------------------------------

    /// Performs a full consistency check of authoritative
    /// market state, history, and cumulative accounting.
    pub fn validate_consistency(&self) -> SimulatorResult<()> {
        self.market().validate_consistency()?;
        self.history.validate_consistency()?;

        if self.market().next_expected()
            != self.history.next_expected()
        {
            return Err(
                SimulatorError::InvariantViolation(
                    "market and history sequence mismatch",
                ),
            );
        }

        if self.generated() != self.history.stats().recorded {
            return Err(
                SimulatorError::InvariantViolation(
                    "generated and recorded event counts differ",
                ),
            );
        }

        if self.generated() != self.faults.stats().accepted {
            return Err(
                SimulatorError::InvariantViolation(
                    "generated and fault admission counts differ",
                ),
            );
        }

        if self.generated() != self.stats.events().total() {
            return Err(
                SimulatorError::InvariantViolation(
                    "generated and statistics counts differ",
                ),
            );
        }

        if self.state == SimulatorState::Completed
            && (!self.faults.is_empty()
                || !self.stats().is_fully_accounted())
        {
            return Err(
                SimulatorError::InvariantViolation(
                    "completed simulator contains pending work",
                ),
            );
        }

        Ok(())
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{SessionId, StreamId};

    use crate::config::{BASIS_POINTS, FaultConfig};
    use crate::scenario::Scenario;

    fn feed() -> FeedId {
        FeedId::new(
            StreamId::new(1),
            SessionId::new(1),
        )
    }

    fn simulator(event_count: u64) -> Simulator {
        let mut config = Scenario::Smoke.config().unwrap();

        config.run.event_count = event_count;

        Simulator::new(
            config,
            feed(),
            RuntimeLimits::default(),
        )
        .unwrap()
    }

    #[derive(Default)]
    struct RecordingSink {
        sequences: Vec<u64>,
        timestamps: Vec<u64>,
        blocked: bool,
    }

    impl EventSink for RecordingSink {
        fn try_send(
            &mut self,
            event: &EventEnvelope,
            received_at: MonotonicTimestamp,
        ) -> SimulatorResult<DeliveryOutcome> {
            if self.blocked {
                return Ok(DeliveryOutcome::Blocked);
            }

            self.sequences.push(event.sequence().raw());
            self.timestamps.push(received_at.raw());

            Ok(DeliveryOutcome::Accepted)
        }
    }

    #[test]
    fn simulator_starts_initialized() {
        let simulator = simulator(100);

        assert_eq!(
            simulator.state(),
            SimulatorState::Initialized,
        );

        assert_eq!(simulator.generated(), 0);
        assert_eq!(simulator.remaining(), 100);
        assert_eq!(simulator.watermark().raw(), 0);
    }

    #[test]
    fn service_requires_started_simulator() {
        let mut simulator = simulator(100);

        assert!(simulator.service(&mut NullSink).is_err());
    }

    #[test]
    fn simulator_cannot_start_twice() {
        let mut simulator = simulator(100);

        simulator.start().unwrap();

        assert!(simulator.start().is_err());
    }

    #[test]
    fn clean_simulation_runs_to_completion() {
        let mut simulator = simulator(1_000);
        let mut sink = RecordingSink::default();

        let report = simulator
            .run_until_blocked(&mut sink)
            .unwrap();

        assert!(report.is_completed());
        assert_eq!(simulator.generated(), 1_000);
        assert_eq!(sink.sequences.len(), 1_000);

        assert_eq!(
            sink.sequences,
            (1..=1_000).collect::<Vec<_>>(),
        );

        assert!(simulator.stats().is_fully_accounted());

        simulator.validate_consistency().unwrap();
    }

    #[test]
    fn service_respects_generation_and_delivery_budgets() {
        let config = Scenario::Smoke.config().unwrap();

        let limits = RuntimeLimits {
            generation_budget: 7,
            delivery_budget: 3,
            ..RuntimeLimits::default()
        };

        let mut simulator =
            Simulator::new(config, feed(), limits).unwrap();

        simulator.start().unwrap();

        let report = simulator.service(&mut NullSink).unwrap();

        assert_eq!(report.generated, 7);
        assert_eq!(report.delivered, 3);
        assert_eq!(report.pending_delivery, 4);
    }

    #[test]
    fn blocked_sink_preserves_undelivered_events() {
        let config = Scenario::Smoke.config().unwrap();

        let limits = RuntimeLimits {
            delivery_capacity: 4,
            generation_budget: 8,
            delivery_budget: 8,
            ..RuntimeLimits::default()
        };

        let mut simulator =
            Simulator::new(config, feed(), limits).unwrap();

        let mut sink = RecordingSink {
            blocked: true,
            ..RecordingSink::default()
        };

        let report = simulator
            .run_until_blocked(&mut sink)
            .unwrap();

        assert!(report.is_blocked());
        assert_eq!(simulator.generated(), 4);
        assert_eq!(simulator.pending_delivery(), 4);
        assert!(sink.sequences.is_empty());

        sink.blocked = false;

        let report = simulator
            .run_until_blocked(&mut sink)
            .unwrap();

        assert!(report.is_completed());

        assert_eq!(
            sink.sequences,
            (1..=1_000).collect::<Vec<_>>(),
        );

        simulator.validate_consistency().unwrap();
    }

    #[test]
    fn duplicate_delivery_preserves_authoritative_count() {
        let mut config = Scenario::Smoke.config().unwrap();

        config.run.event_count = 100;

        config.faults = FaultConfig {
            duplicate_bps: BASIS_POINTS,
            ..FaultConfig::default()
        };

        let mut simulator = Simulator::new(
            config,
            feed(),
            RuntimeLimits::default(),
        )
        .unwrap();

        let mut sink = RecordingSink::default();

        simulator.run_until_blocked(&mut sink).unwrap();

        assert_eq!(simulator.generated(), 100);
        assert_eq!(simulator.history().len(), 100);
        assert_eq!(sink.sequences.len(), 200);

        assert!(simulator.stats().is_fully_accounted());
    }

    #[test]
    fn dropped_events_remain_in_authoritative_history() {
        let mut config = Scenario::Smoke.config().unwrap();

        config.run.event_count = 100;

        config.faults = FaultConfig {
            drop_bps: BASIS_POINTS,
            ..FaultConfig::default()
        };

        let mut simulator = Simulator::new(
            config,
            feed(),
            RuntimeLimits::default(),
        )
        .unwrap();

        let mut sink = RecordingSink::default();

        simulator.run_until_blocked(&mut sink).unwrap();

        assert!(sink.sequences.is_empty());
        assert_eq!(simulator.generated(), 100);
        assert_eq!(simulator.history().len(), 100);
        assert_eq!(simulator.stats().faults.dropped, 100);

        let replay = simulator
            .retransmission(
                SequenceNumber::new(1),
                SequenceNumber::new(100),
            )
            .unwrap();

        assert_eq!(replay.len(), 100);
    }

    #[test]
    fn reordered_events_have_monotonic_reception_times() {
        let mut config = Scenario::Smoke.config().unwrap();

        config.run.event_count = 100;

        config.faults = FaultConfig {
            reorder_bps: BASIS_POINTS,
            reorder_window: 4,
            ..FaultConfig::default()
        };

        let mut simulator = Simulator::new(
            config,
            feed(),
            RuntimeLimits::default(),
        )
        .unwrap();

        let mut sink = RecordingSink::default();

        simulator.run_until_blocked(&mut sink).unwrap();

        assert_eq!(sink.sequences.len(), 100);

        assert!(
            sink.sequences.windows(2).any(|pair| {
                pair[0] > pair[1]
            })
        );

        assert!(
            sink.timestamps.windows(2).all(|pair| {
                pair[0] < pair[1]
            })
        );

        assert!(simulator.stats().is_fully_accounted());
    }

    #[test]
    fn graceful_stop_drains_without_new_generation() {
        let config = Scenario::Smoke.config().unwrap();

        let limits = RuntimeLimits {
            generation_budget: 10,
            delivery_budget: 2,
            ..RuntimeLimits::default()
        };

        let mut simulator =
            Simulator::new(config, feed(), limits).unwrap();

        simulator.start().unwrap();

        simulator.service(&mut NullSink).unwrap();

        assert_eq!(simulator.generated(), 10);

        simulator.request_stop().unwrap();

        let report = simulator
            .run_until_blocked(&mut NullSink)
            .unwrap();

        assert!(report.is_completed());
        assert_eq!(simulator.generated(), 10);
        assert_eq!(simulator.pending_delivery(), 0);

        assert!(simulator.stats().is_fully_accounted());
    }

    #[test]
    fn completed_simulator_service_is_idempotent() {
        let mut simulator = simulator(10);

        simulator.run_until_blocked(&mut NullSink).unwrap();

        let report = simulator.service(&mut NullSink).unwrap();

        assert!(report.is_completed());
        assert!(!report.made_progress());
    }

    #[test]
    fn retransmission_updates_recovery_statistics() {
        let mut simulator = simulator(100);

        simulator.run_until_blocked(&mut NullSink).unwrap();

        let replay = simulator
            .retransmission(
                SequenceNumber::new(10),
                SequenceNumber::new(20),
            )
            .unwrap();

        assert_eq!(replay.len(), 11);

        let stats = simulator.stats();

        assert_eq!(stats.recovery.retransmission_requests, 1);

        assert_eq!(
            stats.recovery.retransmission_events_prepared,
            11,
        );
    }

    #[test]
    fn unavailable_history_is_reported() {
        let config = Scenario::Smoke.config().unwrap();

        let limits = RuntimeLimits {
            history_capacity: 8,
            ..RuntimeLimits::default()
        };

        let mut simulator =
            Simulator::new(config, feed(), limits).unwrap();

        simulator.run_until_blocked(&mut NullSink).unwrap();

        assert!(matches!(
            simulator.retransmission(
                SequenceNumber::new(1),
                SequenceNumber::new(10),
            ),
            Err(SimulatorError::HistoryUnavailable { .. }),
        ));

        assert_eq!(
            simulator.stats().recovery.retransmission_unavailable,
            1,
        );
    }

    #[test]
    fn authoritative_snapshot_matches_current_watermark() {
        let mut simulator = simulator(100);

        simulator.run_until_blocked(&mut NullSink).unwrap();

        let snapshot = simulator
            .snapshot(SequenceNumber::new(100))
            .unwrap();

        assert_eq!(snapshot.watermark().raw(), 100);

        assert_eq!(
            snapshot.order_count(),
            simulator.market().total_orders(),
        );

        assert_eq!(
            simulator.stats().recovery.snapshots_prepared,
            1,
        );
    }

    #[test]
    fn future_snapshot_watermark_is_rejected() {
        let mut simulator = simulator(100);

        assert!(
            simulator.snapshot(SequenceNumber::new(1)).is_err()
        );

        assert_eq!(
            simulator.stats().recovery.snapshot_failed,
            1,
        );
    }

    #[test]
    fn terminal_delivery_error_fails_simulator() {
        struct FailingSink;

        impl EventSink for FailingSink {
            fn try_send(
                &mut self,
                _event: &EventEnvelope,
                _received_at: MonotonicTimestamp,
            ) -> SimulatorResult<DeliveryOutcome> {
                Err(SimulatorError::InvalidState(
                    "simulated transport failure",
                ))
            }
        }

        let mut simulator = simulator(100);

        assert!(
            simulator.run_until_blocked(&mut FailingSink).is_err()
        );

        assert_eq!(
            simulator.state(),
            SimulatorState::Failed,
        );

        assert_eq!(simulator.stats().delivery.failed, 1);
        assert!(simulator.pending_delivery() > 0);
    }
}
