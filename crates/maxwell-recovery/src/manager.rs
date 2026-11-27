//! Single-owner deterministic recovery coordinator.

use maxwell_core::{EventEnvelope, FeedId, MonotonicTimestamp, SequenceNumber, SequenceRange};

use crate::buffer::{InsertOutcome, RecoveryBuffer};
use crate::config::RecoveryConfig;
use crate::error::{RecoveryError, RecoveryResult};
use crate::policy::{RecoveryDecision, RecoveryPolicy};
use crate::replay::ReplayCursor;

use crate::request::{RecoveryRequest, RequestId, RetransmissionRequest, SnapshotRequest};

use crate::retransmit::RetransmissionAttempt;
use crate::snapshot::SnapshotWatermark;
use crate::state::{RecoveryReason, RecoveryState};
use crate::stats::RecoveryStats;
use crate::timeout::RecoveryDeadline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    Applied,
    Buffered,
    Duplicate,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy)]
struct SnapshotAttempt {
    request: SnapshotRequest,
    deadline: RecoveryDeadline,
}

#[derive(Debug)]
pub struct RecoveryManager {
    feed: FeedId,
    config: RecoveryConfig,
    policy: RecoveryPolicy,

    state: RecoveryState,
    reason: Option<RecoveryReason>,

    cursor: ReplayCursor,
    buffer: RecoveryBuffer,

    /// Highest valid sequence observed from this feed.
    highest_seen: Option<SequenceNumber>,

    /// Snapshot must cover events discarded during
    /// buffer overflow or other invalidation.
    required_watermark: SequenceNumber,

    next_request_id: RequestId,

    retransmission: Option<RetransmissionAttempt>,
    snapshot: Option<SnapshotAttempt>,

    /// Request remains available until the runtime
    /// acknowledges successful transport submission.
    outbound: Option<RecoveryRequest>,

    last_now: Option<MonotonicTimestamp>,

    stats: RecoveryStats,
}

impl RecoveryManager {
    /// Creates an unsynchronized manager.
    ///
    /// Call `start_snapshot()` or `initialize_live()`
    /// before processing market data.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn new(feed: FeedId, config: RecoveryConfig) -> RecoveryResult<Self> {
        config.validate()?;

        Ok(Self {
            feed,
            config,
            policy: RecoveryPolicy::new(config)?,
            state: RecoveryState::Uninitialized,
            reason: None,
            cursor: ReplayCursor::new(SequenceNumber::ONE),
            buffer: RecoveryBuffer::new(feed, config.buffer_capacity)?,
            highest_seen: None,
            required_watermark: SequenceNumber::ZERO,
            next_request_id: RequestId::FIRST,
            retransmission: None,
            snapshot: None,
            outbound: None,
            last_now: None,
            stats: RecoveryStats::default(),
        })
    }

    /// Initializes from an externally verified book
    /// checkpoint.
    ///
    /// The caller must guarantee that the book already
    /// represents all events before `next_expected`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidState` unless the manager is uninitialized.
    pub fn initialize_live(&mut self, next_expected: SequenceNumber) -> RecoveryResult<()> {
        if self.state != RecoveryState::Uninitialized {
            return Err(RecoveryError::InvalidState);
        }

        self.cursor.reset(next_expected);
        self.highest_seen = next_expected.previous();

        self.required_watermark = next_expected.previous().unwrap_or(SequenceNumber::ZERO);

        self.state = RecoveryState::Live;
        self.reason = None;

        Ok(())
    }

    #[must_use]
    pub const fn feed(&self) -> FeedId {
        self.feed
    }

    #[must_use]
    pub const fn config(&self) -> RecoveryConfig {
        self.config
    }

    #[must_use]
    pub const fn state(&self) -> RecoveryState {
        self.state
    }

    #[must_use]
    pub const fn reason(&self) -> Option<RecoveryReason> {
        self.reason
    }

    #[must_use]
    pub const fn next_expected(&self) -> SequenceNumber {
        self.cursor.next_expected()
    }

    #[must_use]
    pub const fn highest_seen(&self) -> Option<SequenceNumber> {
        self.highest_seen
    }

    #[must_use]
    pub fn buffered_events(&self) -> usize {
        self.buffer.len()
    }

    #[must_use]
    pub const fn stats(&self) -> &RecoveryStats {
        &self.stats
    }

    #[must_use]
    pub const fn is_publishable(&self) -> bool {
        self.state.is_publishable()
    }

    #[must_use]
    pub const fn pending_request(&self) -> Option<RecoveryRequest> {
        self.outbound
    }

    /// Acknowledge that the complete request has been
    /// submitted to the transport.
    ///
    /// Do not acknowledge a partial TCP write.
    ///
    /// # Errors
    ///
    /// Returns `StaleResponse` if `id` is not the pending request.
    pub fn acknowledge_request(&mut self, id: RequestId) -> RecoveryResult<()> {
        match self.outbound {
            Some(request) if request.id() == id => {
                self.outbound = None;
                Ok(())
            }

            _ => Err(RecoveryError::StaleResponse),
        }
    }

    fn observe_time(&mut self, now: MonotonicTimestamp) -> RecoveryResult<()> {
        if self.last_now.is_some_and(|previous| now < previous) {
            return Err(RecoveryError::ClockWentBackward);
        }

        self.last_now = Some(now);

        Ok(())
    }

    fn allocate_request_id(&mut self) -> RecoveryResult<RequestId> {
        let id = self.next_request_id;

        self.next_request_id = id.next().ok_or(RecoveryError::RequestIdExhausted)?;

        Ok(id)
    }

    const fn cancel_requests(&mut self) {
        self.retransmission = None;
        self.snapshot = None;
        self.outbound = None;
    }

    const fn fail(&mut self, reason: RecoveryReason) {
        self.cancel_requests();
        self.state = RecoveryState::Failed;
        self.reason = Some(reason);
        self.stats.record_recovery_failure();
    }

    const fn mark_live(&mut self) {
        let recovering = self.state.is_recovering();

        self.cancel_requests();
        self.state = RecoveryState::Live;
        self.reason = None;

        if recovering {
            self.stats.record_recovery_completed();
        }
    }

    fn update_highest_seen(&mut self, sequence: SequenceNumber) {
        self.highest_seen = Some(
            self.highest_seen
                .map_or(sequence, |previous| previous.max(sequence)),
        );
    }

    fn minimum_snapshot_watermark(&self) -> SequenceNumber {
        let applied = self.cursor.last_applied().unwrap_or(SequenceNumber::ZERO);

        applied.max(self.required_watermark)
    }

    fn issue_retransmission(
        &mut self,
        range: SequenceRange,
        attempt: u32,
        now: MonotonicTimestamp,
    ) -> RecoveryResult<()> {
        let request = RetransmissionRequest {
            id: self.allocate_request_id()?,
            feed: self.feed,
            range,
            attempt,
        };

        let active = RetransmissionAttempt::new(request, now, self.config.retransmission_timeout)?;

        self.cancel_requests();

        self.retransmission = Some(active);
        self.outbound = Some(RecoveryRequest::Retransmission(request));

        self.state = RecoveryState::AwaitingRetransmission;

        self.stats.record_retransmission_request();

        if attempt > 1 {
            self.stats.record_retransmission_retry();
        }

        Ok(())
    }

    fn issue_snapshot(
        &mut self,
        reason: RecoveryReason,
        attempt: u32,
        now: MonotonicTimestamp,
    ) -> RecoveryResult<()> {
        let request = SnapshotRequest {
            id: self.allocate_request_id()?,
            feed: self.feed,
            minimum_watermark: self.minimum_snapshot_watermark(),
            reason,
            attempt,
        };

        let deadline = RecoveryDeadline::new(now, self.config.snapshot_timeout)?;

        self.cancel_requests();

        self.snapshot = Some(SnapshotAttempt { request, deadline });

        self.outbound = Some(RecoveryRequest::Snapshot(request));

        self.state = RecoveryState::AwaitingSnapshot;
        self.reason = Some(reason);

        self.stats.record_snapshot_request();

        if attempt > 1 {
            self.stats.record_snapshot_retry();
        }

        Ok(())
    }

    /// Begins snapshot recovery for the feed.
    ///
    /// # Errors
    ///
    /// Returns an error if `now` moves backward or the snapshot
    /// request cannot be issued.
    pub fn start_snapshot(&mut self, now: MonotonicTimestamp) -> RecoveryResult<()> {
        self.observe_time(now)?;

        let reason = if self.state == RecoveryState::Uninitialized {
            RecoveryReason::Startup
        } else {
            RecoveryReason::ManualResynchronization
        };

        self.state = RecoveryState::GapDetected;
        self.reason = Some(reason);
        self.cancel_requests();

        self.issue_snapshot(reason, 1, now)
    }

    /// Starts a new feed session.
    ///
    /// Session transitions must be explicitly authorized
    /// by the runtime. Arbitrary incoming packets cannot
    /// reset the recovery manager.
    ///
    /// # Errors
    ///
    /// Returns `InvalidRequest` if `feed` is the current feed
    /// identity, or an error if `now` moves backward or the
    /// snapshot request cannot be issued.
    pub fn transition_session(
        &mut self,
        feed: FeedId,
        now: MonotonicTimestamp,
    ) -> RecoveryResult<()> {
        self.observe_time(now)?;

        if feed == self.feed {
            return Err(RecoveryError::InvalidRequest);
        }

        self.cancel_requests();

        self.stats.record_discarded(self.buffer.len());

        self.feed = feed;
        self.buffer.reset(feed);
        self.cursor.reset(SequenceNumber::ONE);

        self.highest_seen = None;
        self.required_watermark = SequenceNumber::ZERO;

        self.state = RecoveryState::GapDetected;
        self.reason = Some(RecoveryReason::SessionChanged);

        self.stats.record_session_transition();

        // Request IDs intentionally survive session changes.
        self.issue_snapshot(RecoveryReason::SessionChanged, 1, now)
    }

    fn validate_event(&mut self, event: &EventEnvelope) -> RecoveryResult<()> {
        if event.feed() != self.feed {
            self.stats.record_rejected();

            return Err(RecoveryError::FeedMismatch {
                expected: self.feed,
                received: event.feed(),
            });
        }

        if event.validate().is_err() {
            self.stats.record_rejected();
            self.fail(RecoveryReason::BookInconsistency);

            return Err(RecoveryError::InvalidEvent);
        }

        Ok(())
    }

    /// Application callback must be transactional:
    /// failure must not expose partially applied state.
    fn apply_event<F>(
        &mut self,
        event: &EventEnvelope,
        apply: &mut F,
        replayed: bool,
    ) -> RecoveryResult<()>
    where
        F: FnMut(&EventEnvelope) -> RecoveryResult<()>,
    {
        if let Err(error) = self.cursor.validate(event.sequence()) {
            self.fail(RecoveryReason::SequenceExhausted);

            return Err(error);
        }

        if apply(event).is_err() {
            self.fail(RecoveryReason::BookInconsistency);

            return Err(RecoveryError::EventApplicationFailed);
        }

        self.cursor.commit(event.sequence())?;

        self.stats.record_applied();

        if replayed {
            self.stats.record_replayed();
        }

        Ok(())
    }

    fn drain_buffer<F>(&mut self, apply: &mut F) -> RecoveryResult<()>
    where
        F: FnMut(&EventEnvelope) -> RecoveryResult<()>,
    {
        loop {
            let expected = self.cursor.next_expected();

            let Some(event) = self.buffer.get(expected).copied() else {
                break;
            };

            // Remove only after successful application.
            self.apply_event(&event, apply, true)?;

            self.buffer.remove(expected);
        }

        Ok(())
    }

    /// Reconciles committed progress with the highest
    /// observed sequence and any outstanding request.
    fn reconcile(&mut self, now: MonotonicTimestamp) -> RecoveryResult<()> {
        if matches!(
            self.state,
            RecoveryState::AwaitingSnapshot | RecoveryState::Failed | RecoveryState::Uninitialized
        ) {
            return Ok(());
        }

        let expected = self.cursor.next_expected();

        if let Some(active) = self.retransmission {
            if !active.is_complete(expected) {
                self.state = RecoveryState::AwaitingRetransmission;

                return Ok(());
            }

            self.cancel_requests();
        }

        let Some(highest) = self.highest_seen else {
            self.mark_live();
            return Ok(());
        };

        if expected > highest {
            self.mark_live();

            return Ok(());
        }

        // Find the missing prefix preceding the next
        // buffered event. If no event remains buffered,
        // recover through the highest observed sequence.
        let missing = match self.buffer.first_sequence() {
            Some(first) if first > expected => SequenceRange::missing(expected, first),

            Some(_) => {
                return Err(RecoveryError::InvalidState);
            }

            None => SequenceRange::new(expected, highest),
        }
        .ok_or(RecoveryError::InvalidRequest)?;

        self.state = RecoveryState::GapDetected;
        self.reason = Some(RecoveryReason::SequenceGap);

        self.stats.record_gap(missing.count());

        match self.policy.on_gap(missing)? {
            RecoveryDecision::Retransmit(range) => self.issue_retransmission(range, 1, now),

            RecoveryDecision::Snapshot(reason) => self.issue_snapshot(reason, 1, now),

            RecoveryDecision::Fail(reason) => {
                self.fail(reason);
                Ok(())
            }
        }
    }

    fn buffer_event(
        &mut self,
        event: EventEnvelope,
        now: MonotonicTimestamp,
    ) -> RecoveryResult<IngestOutcome> {
        match self.buffer.insert(event) {
            Ok(InsertOutcome::Inserted) => {
                self.stats.record_buffered(self.buffer.len());

                Ok(IngestOutcome::Buffered)
            }

            Ok(InsertOutcome::Duplicate) => {
                self.stats.record_duplicate();

                Ok(IngestOutcome::Duplicate)
            }

            Err(RecoveryError::BufferFull { .. }) => {
                self.stats.record_buffer_overflow();

                // The replacement snapshot must cover
                // every event we are about to discard.
                let highest = self.highest_seen.unwrap_or_else(|| event.sequence());

                self.required_watermark = self.required_watermark.max(highest);

                self.stats
                    .record_discarded(self.buffer.len().saturating_add(1));

                self.buffer.clear();

                let attempt = self
                    .snapshot
                    .map_or(1, |active| active.request.attempt.saturating_add(1));

                self.cancel_requests();
                self.state = RecoveryState::GapDetected;
                self.reason = Some(RecoveryReason::BufferOverflow);

                if attempt > self.config.max_snapshot_attempts {
                    self.fail(RecoveryReason::BufferOverflow);
                } else {
                    self.issue_snapshot(RecoveryReason::BufferOverflow, attempt, now)?;
                }

                Ok(IngestOutcome::RecoveryRequired)
            }

            Err(error @ RecoveryError::ConflictingEvent { .. }) => {
                self.stats.record_conflict();
                self.fail(RecoveryReason::ConflictingEvent);

                Err(error)
            }

            Err(error) => Err(error),
        }
    }

    /// Ingests a live event or an already validated
    /// retransmission event.
    ///
    /// This method never applies an event ahead of the
    /// committed sequence cursor.
    ///
    /// # Errors
    ///
    /// Returns `InvalidState` if the manager is uninitialized or
    /// failed, `ConflictingEvent` if the event contradicts a buffered
    /// one, or any error from validation, buffering, or `apply`.
    pub fn ingest<F>(
        &mut self,
        event: EventEnvelope,
        now: MonotonicTimestamp,
        apply: &mut F,
    ) -> RecoveryResult<IngestOutcome>
    where
        F: FnMut(&EventEnvelope) -> RecoveryResult<()>,
    {
        self.observe_time(now)?;

        if matches!(
            self.state,
            RecoveryState::Uninitialized | RecoveryState::Failed
        ) {
            return Err(RecoveryError::InvalidState);
        }

        self.stats.record_received();
        self.validate_event(&event)?;

        let sequence = event.sequence();
        let expected = self.cursor.next_expected();

        if sequence < expected {
            self.stats.record_duplicate();

            return Ok(IngestOutcome::Duplicate);
        }

        self.update_highest_seen(sequence);

        // During snapshot recovery, do not mutate the
        // existing book, even for contiguous events.
        if self.state == RecoveryState::AwaitingSnapshot {
            return self.buffer_event(event, now);
        }

        if sequence == expected {
            if let Some(buffered) = self.buffer.get(sequence)
                && *buffered != event
            {
                self.stats.record_conflict();
                self.fail(RecoveryReason::ConflictingEvent);

                return Err(RecoveryError::ConflictingEvent { sequence });
            }

            let replayed = self.state.is_recovering();

            self.apply_event(&event, apply, replayed)?;
            self.buffer.remove(sequence);

            self.drain_buffer(apply)?;
            self.reconcile(now)?;

            return Ok(IngestOutcome::Applied);
        }

        // Mark stale before buffering or allocating a
        // recovery request.
        if self.state == RecoveryState::Live {
            self.state = RecoveryState::GapDetected;
            self.reason = Some(RecoveryReason::SequenceGap);
        }

        let outcome = self.buffer_event(event, now)?;

        self.reconcile(now)?;

        Ok(outcome)
    }

    /// Validates retransmission request correlation
    /// before admitting recovered events.
    ///
    /// # Errors
    ///
    /// Returns `StaleResponse` if no matching request is active, or
    /// any error from [`Self::ingest`].
    pub fn ingest_retransmission<F>(
        &mut self,
        request_id: RequestId,
        event: EventEnvelope,
        now: MonotonicTimestamp,
        apply: &mut F,
    ) -> RecoveryResult<IngestOutcome>
    where
        F: FnMut(&EventEnvelope) -> RecoveryResult<()>,
    {
        let active = self.retransmission.ok_or(RecoveryError::StaleResponse)?;

        active.validate_event(request_id, &event)?;

        self.ingest(event, now, apply)
    }

    /// Called when the upstream server cannot provide
    /// the requested historical events.
    ///
    /// # Errors
    ///
    /// Returns `StaleResponse` if no matching request is active, or
    /// an error if `now` moves backward.
    pub fn retransmission_unavailable(
        &mut self,
        request_id: RequestId,
        now: MonotonicTimestamp,
    ) -> RecoveryResult<()> {
        self.observe_time(now)?;

        let active = self.retransmission.ok_or(RecoveryError::StaleResponse)?;

        if active.request().id != request_id {
            return Err(RecoveryError::StaleResponse);
        }

        self.cancel_requests();
        self.state = RecoveryState::GapDetected;

        self.issue_snapshot(RecoveryReason::RetransmissionUnavailable, 1, now)
    }

    /// Validates and atomically installs a complete
    /// externally assembled snapshot.
    ///
    /// The install callback must replace the entire
    /// feed state, not merely one instrument.
    ///
    /// # Errors
    ///
    /// Returns `StaleResponse` if no snapshot request is active,
    /// `SnapshotTooOld` if the watermark does not cover the required
    /// sequence, `SnapshotInstallFailed` if `install` fails, or any
    /// error from replaying buffered events.
    pub fn install_snapshot<S, F>(
        &mut self,
        watermark: SnapshotWatermark,
        now: MonotonicTimestamp,
        install: S,
        apply: &mut F,
    ) -> RecoveryResult<()>
    where
        S: FnOnce() -> RecoveryResult<()>,
        F: FnMut(&EventEnvelope) -> RecoveryResult<()>,
    {
        self.observe_time(now)?;

        if self.state != RecoveryState::AwaitingSnapshot {
            return Err(RecoveryError::StaleResponse);
        }

        let active = self.snapshot.ok_or(RecoveryError::StaleResponse)?;

        watermark.validate(active.request)?;

        if watermark.last_sequence < self.minimum_snapshot_watermark() {
            return Err(RecoveryError::SnapshotTooOld);
        }

        let next_expected = watermark.next_expected()?;

        // The external book is replaced only after
        // validating the snapshot's metadata.
        if install().is_err() {
            self.fail(RecoveryReason::BookInconsistency);

            return Err(RecoveryError::SnapshotInstallFailed);
        }

        self.cursor.reset(next_expected);

        let discarded = self.buffer.discard_through(watermark.last_sequence);

        self.stats.record_discarded(discarded);
        self.stats.record_snapshot_installed();

        self.required_watermark = watermark.last_sequence;

        self.update_highest_seen(watermark.last_sequence);

        self.cancel_requests();
        self.state = RecoveryState::Replaying;

        self.drain_buffer(apply)?;
        self.reconcile(now)?;

        Ok(())
    }

    /// Advances timeout processing using the runtime's
    /// monotonic clock.
    ///
    /// Deadlines include outbound queueing time.
    ///
    /// # Errors
    ///
    /// Returns `InvalidState` if the manager is uninitialized or
    /// failed, or an error if `now` moves backward or a follow-up
    /// request cannot be issued.
    pub fn poll(&mut self, now: MonotonicTimestamp) -> RecoveryResult<()> {
        self.observe_time(now)?;

        if let Some(active) = self.retransmission {
            if !active.is_expired(now)? {
                return Ok(());
            }

            self.stats.record_retransmission_timeout();

            let request = active.request();

            if self.policy.can_retry_retransmission(request.attempt) {
                let remaining = active
                    .remaining_range(self.cursor.next_expected())
                    .ok_or(RecoveryError::InvalidState)?;

                self.issue_retransmission(remaining, request.attempt + 1, now)?;
            } else {
                self.cancel_requests();
                self.state = RecoveryState::GapDetected;

                self.issue_snapshot(RecoveryReason::RetransmissionTimeout, 1, now)?;
            }

            return Ok(());
        }

        if let Some(active) = self.snapshot {
            if !active.deadline.is_expired(now)? {
                return Ok(());
            }

            self.stats.record_snapshot_timeout();

            if self.policy.can_retry_snapshot(active.request.attempt) {
                self.issue_snapshot(active.request.reason, active.request.attempt + 1, now)?;
            } else {
                self.fail(RecoveryReason::SnapshotTimeout);
            }

            return Ok(());
        }

        if self.state == RecoveryState::GapDetected {
            self.reconcile(now)?;
        }

        Ok(())
    }

    /// Detects trailing packet loss using an authenticated
    /// upstream sequence watermark or heartbeat.
    ///
    /// Unlike ordinary gap detection, this does not
    /// require receiving a later market data event.
    ///
    /// # Errors
    ///
    /// Returns `FeedMismatch` for another feed, `InvalidState` if
    /// the manager is uninitialized or failed, or an error if `now`
    /// moves backward.
    pub fn observe_watermark(
        &mut self,
        feed: FeedId,
        last_sequence: SequenceNumber,
        now: MonotonicTimestamp,
    ) -> RecoveryResult<()> {
        self.observe_time(now)?;

        if feed != self.feed {
            return Err(RecoveryError::FeedMismatch {
                expected: self.feed,
                received: feed,
            });
        }

        if matches!(
            self.state,
            RecoveryState::Uninitialized | RecoveryState::Failed
        ) {
            return Err(RecoveryError::InvalidState);
        }

        self.update_highest_seen(last_sequence);

        if last_sequence >= self.cursor.next_expected() && self.state == RecoveryState::Live {
            self.state = RecoveryState::GapDetected;
            self.reason = Some(RecoveryReason::SequenceGap);
        }

        self.reconcile(now)
    }

    pub fn reset_stats(&mut self) {
        self.stats.reset();
    }
}
