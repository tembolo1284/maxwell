//! Single-owner Maxwell server runtime.
//!
//! The server coordinates component ownership, upstream
//! admission, downstream service, snapshot synchronization,
//! operational metrics, and lifecycle transitions.
//!
//! The initial runtime is externally driven and does not
//! open network sockets or create background threads.

use maxwell_core::{
    EventEnvelope, FeedId, Instrument, InstrumentId, MonotonicTimestamp, SequenceNumber,
};
use maxwell_gateway::{ClientRequest, GatewaySink, SessionId};
use maxwell_publisher::SubscriptionFilter;

use crate::config::ServerConfig;
use crate::downstream::SnapshotSink;
use crate::error::{ServerError, ServerResult};
use crate::metrics::ServerMetrics;
use crate::pipeline::{Pipeline, PublicationOutcome};
use crate::recovery::{RecoveryCoordinator, RecoveryOutcome};
use crate::snapshot::{SnapshotOutcome, synchronize_session};
use crate::state::ServerState;
use crate::upstream::{MaxwellDecoder, Upstream, UpstreamDecoder};

// ============================================================
// Server status
// ============================================================

/// Point-in-time observation of server state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerStatus {
    pub lifecycle: ServerState,
    pub synchronized: bool,
    pub publication_pending: bool,
    pub publication_watermark: u64,
    pub next_expected: SequenceNumber,
    pub connected_sessions: usize,
    pub registered_instruments: usize,
}

// ============================================================
// Server runtime
// ============================================================

/// Integrated single-feed market data server.
///
/// All mutable processing components are owned by this
/// runtime and accessed through explicit mutable references.
pub struct Server<D = MaxwellDecoder> {
    config: ServerConfig,
    state: ServerState,
    upstream: Upstream<D>,
    recovery: RecoveryCoordinator,
    pipeline: Pipeline,
    metrics: ServerMetrics,

    subscriptions: Vec<(SessionId, SubscriptionFilter<InstrumentId>)>,

    last_timestamp_ns: u64,
    shutdown_started_ns: Option<u64>,
}

impl Server<MaxwellDecoder> {
    /// Constructs a server using the native Maxwell protocol.
    pub fn new(config: ServerConfig, feed: FeedId) -> ServerResult<Self> {
        Self::with_decoder(config, feed, MaxwellDecoder::new())
    }
}

impl<D: UpstreamDecoder> Server<D> {
    /// Constructs a server using a custom upstream decoder.
    pub fn with_decoder(config: ServerConfig, feed: FeedId, decoder: D) -> ServerResult<Self> {
        config.validate()?;

        let pipeline = Pipeline::new(&config)?;
        let recovery = RecoveryCoordinator::new(feed, &config)?;

        let metrics = ServerMetrics::new()?;

        let mut subscriptions = Vec::new();

        subscriptions
            .try_reserve_exact(config.gateway.publisher.max_subscribers)
            .map_err(|_| ServerError::AllocationFailed)?;

        Ok(Self {
            config,
            state: ServerState::Initialized,
            upstream: Upstream::new(feed, decoder),
            recovery,
            pipeline,
            metrics,
            subscriptions,
            last_timestamp_ns: 0,
            shutdown_started_ns: None,
        })
    }

    // --------------------------------------------------------
    // Component observation
    // --------------------------------------------------------

    #[must_use]
    pub const fn config(&self) -> &ServerConfig {
        &self.config
    }

    #[must_use]
    pub const fn state(&self) -> ServerState {
        self.state
    }

    #[must_use]
    pub const fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    #[must_use]
    pub const fn recovery(&self) -> &RecoveryCoordinator {
        &self.recovery
    }

    #[must_use]
    pub const fn metrics(&self) -> &ServerMetrics {
        &self.metrics
    }

    #[must_use]
    pub const fn upstream(&self) -> &Upstream<D> {
        &self.upstream
    }

    /// Returns whether the reconstructed upstream market
    /// has an authoritative synchronized baseline.
    #[must_use]
    pub fn is_synchronized(&self) -> bool {
        self.recovery.is_synchronized() && self.pipeline.is_valid()
    }

    /// Returns whether current state may be advertised.
    #[must_use]
    pub fn is_publishable(&self) -> bool {
        self.state == ServerState::Running
            && self.is_synchronized()
            && !self.pipeline.has_pending_publication()
    }

    #[must_use]
    pub fn status(&self) -> ServerStatus {
        ServerStatus {
            lifecycle: self.state,
            synchronized: self.is_synchronized(),
            publication_pending: self.pipeline.has_pending_publication(),
            publication_watermark: self.pipeline.publication_watermark(),
            next_expected: self.recovery.next_expected(),
            connected_sessions: self.pipeline.gateway().session_count(),
            registered_instruments: self.pipeline.books().instrument_count(),
        }
    }

    // --------------------------------------------------------
    // Internal validation
    // --------------------------------------------------------

    fn require_running(&self) -> ServerResult<()> {
        if self.state != ServerState::Running {
            return Err(ServerError::InvalidState("server is not running"));
        }

        Ok(())
    }

    fn require_serviceable(&self) -> ServerResult<()> {
        if !self.state.services_downstream() {
            return Err(ServerError::InvalidState(
                "server cannot service downstream connections",
            ));
        }

        Ok(())
    }

    fn advance_clock(&mut self, now: MonotonicTimestamp) -> ServerResult<()> {
        let now_ns = now.raw();

        if now_ns < self.last_timestamp_ns {
            return Err(ServerError::InvalidState(
                "server monotonic timestamp regressed",
            ));
        }

        self.last_timestamp_ns = now_ns;

        Ok(())
    }

    fn refresh_metrics(&mut self) -> ServerResult<()> {
        let synchronized = self.is_publishable();

        self.metrics.update_status(
            self.state,
            self.pipeline.gateway().session_count(),
            synchronized,
        )
    }

    /// A fatal processing failure terminates the runtime.
    ///
    /// No further upstream events may be accepted.
    fn fail(&mut self) {
        self.pipeline.invalidate();

        if !self.state.is_terminal() {
            self.state = ServerState::Failed;
        }

        // Instrumentation failure must not replace the
        // original processing error.
        let _ = self.refresh_metrics();
    }

    /// Removes subscription metadata for disconnected
    /// or expired gateway sessions.
    fn prune_subscriptions(&mut self) {
        let gateway = self.pipeline.gateway();

        self.subscriptions.retain(|(id, _)| {
            gateway
                .session(*id)
                .is_some_and(|session| !session.is_expired())
        });
    }

    // --------------------------------------------------------
    // Initialization
    // --------------------------------------------------------

    /// Registers an instrument before processing begins.
    pub fn register_instrument(&mut self, instrument: Instrument) -> ServerResult<()> {
        if self.state != ServerState::Initialized {
            return Err(ServerError::InvalidState(
                "instruments must be registered before startup",
            ));
        }

        self.pipeline
            .books_mut()
            .register(instrument, self.upstream.feed())?;

        Ok(())
    }

    /// Starts from an authoritative empty market baseline.
    ///
    /// This is appropriate for a newly established feed
    /// whose first event has sequence number one.
    ///
    /// It must not be used to join an existing live market
    /// with unknown resting liquidity.
    pub fn start_empty(&mut self, now: MonotonicTimestamp) -> ServerResult<()> {
        if self.state != ServerState::Initialized {
            return Err(ServerError::InvalidState("server has already been started"));
        }

        self.advance_clock(now)?;

        self.recovery
            .initialize_live(SequenceNumber::ONE, &mut self.pipeline)?;

        self.state.transition_to(ServerState::Running)?;

        self.refresh_metrics()
    }

    /// Starts without an authoritative market baseline.
    ///
    /// The server remains unsynchronized while waiting
    /// for authoritative upstream recovery.
    pub fn start_recovering(&mut self, now: MonotonicTimestamp) -> ServerResult<()> {
        if self.state != ServerState::Initialized {
            return Err(ServerError::InvalidState("server has already been started"));
        }

        self.advance_clock(now)?;

        self.recovery.start_snapshot(now, &mut self.pipeline)?;

        self.state.transition_to(ServerState::Running)?;

        self.refresh_metrics()
    }

    // --------------------------------------------------------
    // Upstream processing
    // --------------------------------------------------------

    /// Decodes and processes one complete upstream payload.
    pub fn ingest_payload(
        &mut self,
        payload: &[u8],
        now: MonotonicTimestamp,
    ) -> ServerResult<RecoveryOutcome> {
        self.require_running()?;

        if self.pipeline.has_pending_publication() {
            return Ok(RecoveryOutcome::InputBlocked);
        }

        let event = self.upstream.decode(payload)?;

        self.ingest_event(event, now)
    }

    /// Processes one canonical upstream event.
    ///
    /// InputBlocked indicates that the supplied event
    /// has not been consumed and must be retained.
    ///
    /// PublicationPending indicates that the event was
    /// consumed and only its publication requires retry.
    pub fn ingest_event(
        &mut self,
        event: EventEnvelope,
        now: MonotonicTimestamp,
    ) -> ServerResult<RecoveryOutcome> {
        self.require_running()?;
        self.advance_clock(now)?;

        self.metrics.record_upstream_received()?;

        if let Err(error) = self.upstream.validate_event(&event) {
            self.metrics.record_upstream_error()?;

            return Err(error);
        }

        let outcome = match self.recovery.ingest(event, now, &mut self.pipeline) {
            Ok(outcome) => outcome,

            Err(error) => {
                let _ = self.metrics.record_upstream_error();

                self.fail();

                return Err(error);
            }
        };

        // The event may already have been consumed.
        // Instrumentation failure is therefore fatal,
        // rather than a reason to retry the event.
        if let Err(error) = self
            .metrics
            .record_recovery_outcome(outcome)
            .and_then(|()| self.refresh_metrics())
        {
            self.fail();

            return Err(error);
        }

        Ok(outcome)
    }

    // --------------------------------------------------------
    // Publication retry
    // --------------------------------------------------------

    /// Retries a pending publication without mutating
    /// the book or advancing upstream sequencing.
    pub fn retry_publication(&mut self) -> ServerResult<PublicationOutcome> {
        self.require_serviceable()?;

        let outcome = match self.recovery.retry_publication(&mut self.pipeline) {
            Ok(outcome) => outcome,

            Err(error) => {
                self.fail();

                return Err(error);
            }
        };

        if let Err(error) = self
            .metrics
            .record_publication_retry(outcome)
            .and_then(|()| self.refresh_metrics())
        {
            self.fail();

            return Err(error);
        }

        Ok(outcome)
    }

    // --------------------------------------------------------
    // Downstream session management
    // --------------------------------------------------------

    /// Registers a new downstream session.
    pub fn connect(&mut self, session_id: SessionId, now: MonotonicTimestamp) -> ServerResult<()> {
        self.require_running()?;
        self.advance_clock(now)?;

        self.pipeline.gateway_mut().connect(session_id, now.raw())?;

        self.refresh_metrics()
    }

    /// Removes a downstream session and its subscription.
    pub fn disconnect(&mut self, session_id: SessionId) -> ServerResult<()> {
        self.pipeline.gateway_mut().disconnect(session_id)?;

        self.subscriptions.retain(|(id, _)| *id != session_id);

        self.refresh_metrics()
    }

    /// Processes one decoded downstream client request.
    pub fn handle_request(
        &mut self,
        session_id: SessionId,
        request: ClientRequest<InstrumentId>,
        now: MonotonicTimestamp,
    ) -> ServerResult<()> {
        self.require_running()?;
        self.advance_clock(now)?;

        let subscription = match &request {
            ClientRequest::Subscribe { filter } => Some(filter.clone()),

            _ => None,
        };

        let remove_subscription = matches!(
            &request,
            ClientRequest::Unsubscribe | ClientRequest::Disconnect
        );

        self.pipeline
            .gateway_mut()
            .handle_request(session_id, request, now.raw())?;

        if remove_subscription {
            self.subscriptions.retain(|(id, _)| *id != session_id);
        }

        if let Some(filter) = subscription {
            if let Some((_, existing)) = self
                .subscriptions
                .iter_mut()
                .find(|(id, _)| *id == session_id)
            {
                *existing = filter;
            } else {
                self.subscriptions.push((session_id, filter));
            }
        }

        self.prune_subscriptions();

        self.refresh_metrics()
    }

    // --------------------------------------------------------
    // Downstream service
    // --------------------------------------------------------

    /// Services one downstream session using the supplied
    /// ordered transport destination.
    ///
    /// The gateway enforces its per-service message budget.
    pub fn service_session<S>(
        &mut self,
        session_id: SessionId,
        sink: &mut S,
        now: MonotonicTimestamp,
    ) -> ServerResult<maxwell_gateway::ServiceReport<S::Error>>
    where
        S: GatewaySink<InstrumentId, EventEnvelope>,
    {
        self.require_serviceable()?;
        self.advance_clock(now)?;

        let report = self
            .pipeline
            .gateway_mut()
            .service(session_id, sink, now.raw())?;

        self.metrics
            .record_downstream_service(report.total_sent(), report.is_blocked())?;

        self.prune_subscriptions();

        self.refresh_metrics()?;

        Ok(report)
    }

    // --------------------------------------------------------
    // Snapshot synchronization
    // --------------------------------------------------------

    /// Synchronizes a session using its registered filter.
    ///
    /// Snapshot capture and activation occur within one
    /// exclusive mutable borrow of the server runtime.
    pub fn synchronize_session<S>(
        &mut self,
        session_id: SessionId,
        sink: &mut S,
        now: MonotonicTimestamp,
    ) -> ServerResult<SnapshotOutcome<S::Error>>
    where
        S: SnapshotSink,
    {
        self.require_running()?;
        self.advance_clock(now)?;

        if !self.is_publishable() {
            return Err(ServerError::SnapshotUnavailable);
        }

        let filter = self
            .subscriptions
            .iter()
            .find(|(id, _)| *id == session_id)
            .map(|(_, filter)| filter.clone())
            .ok_or(ServerError::InvalidState(
                "session has no registered subscription",
            ))?;

        let result = synchronize_session(
            &mut self.pipeline,
            session_id,
            &filter,
            self.config.snapshot,
            sink,
            now.raw(),
        );

        self.prune_subscriptions();

        let outcome = result?;

        match &outcome {
            SnapshotOutcome::Activated { entry_count, .. } => {
                self.metrics.record_snapshot_accepted(*entry_count)?;
            }

            SnapshotOutcome::Rejected(_) => {
                self.metrics.record_snapshot_rejected()?;
            }
        }

        self.refresh_metrics()?;

        Ok(outcome)
    }

    // --------------------------------------------------------
    // Periodic maintenance
    // --------------------------------------------------------

    /// Services recovery timers and session expiration.
    ///
    /// This does not read upstream sockets or flush
    /// downstream transport queues.
    pub fn poll(&mut self, now: MonotonicTimestamp) -> ServerResult<()> {
        self.require_serviceable()?;
        self.advance_clock(now)?;

        self.metrics.record_service_iteration()?;

        if self.state == ServerState::Running {
            if let Err(error) = self.recovery.poll(now, &mut self.pipeline) {
                self.fail();

                return Err(error);
            }
        }

        self.pipeline.gateway_mut().expire_sessions(now.raw())?;

        self.prune_subscriptions();

        if self.state == ServerState::Draining {
            self.check_shutdown_deadline(now)?;
        }

        self.refresh_metrics()
    }

    // --------------------------------------------------------
    // Shutdown
    // --------------------------------------------------------

    /// Stops accepting new upstream events and sessions.
    pub fn begin_shutdown(&mut self, now: MonotonicTimestamp) -> ServerResult<()> {
        self.advance_clock(now)?;

        match self.state {
            ServerState::Initialized => {
                self.state.transition_to(ServerState::Stopped)?;
            }

            ServerState::Running => {
                self.state.transition_to(ServerState::Draining)?;

                self.shutdown_started_ns = Some(now.raw());
            }

            ServerState::Draining | ServerState::Stopped => {}

            ServerState::Failed => {
                return Err(ServerError::InvalidState(
                    "failed server cannot begin graceful shutdown",
                ));
            }
        }

        self.refresh_metrics()
    }

    fn check_shutdown_deadline(&mut self, now: MonotonicTimestamp) -> ServerResult<()> {
        let Some(started) = self.shutdown_started_ns else {
            return Err(ServerError::InvariantViolation(
                "draining server has no shutdown timestamp",
            ));
        };

        let elapsed = now.raw().saturating_sub(started);

        if u128::from(elapsed) >= self.config.shutdown.timeout.as_nanos() {
            self.fail();

            return Err(ServerError::ShutdownTimeout);
        }

        Ok(())
    }

    /// Completes shutdown after the transport owner has
    /// drained and disconnected every downstream session.
    ///
    /// The caller is responsible for ensuring that accepted
    /// transport frames have actually been flushed.
    pub fn finish_shutdown(&mut self, now: MonotonicTimestamp) -> ServerResult<()> {
        if self.state == ServerState::Stopped {
            return Ok(());
        }

        if self.state != ServerState::Draining {
            return Err(ServerError::InvalidState("server is not draining"));
        }

        self.advance_clock(now)?;
        self.check_shutdown_deadline(now)?;

        if self.pipeline.has_pending_publication() {
            return Err(ServerError::InvalidState(
                "shutdown has pending publication",
            ));
        }

        if self.pipeline.gateway().session_count() != 0 {
            return Err(ServerError::InvalidState(
                "shutdown has connected downstream sessions",
            ));
        }

        self.state.transition_to(ServerState::Stopped)?;

        self.refresh_metrics()
    }
}
