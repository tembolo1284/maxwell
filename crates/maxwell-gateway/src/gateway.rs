//! Single-owner market data gateway.

use maxwell_publisher::{
    EventKind, Publication, PublicationSink, PublishReport, Publisher, PublisherStats,
    SubscriberState, SubscriptionFilter,
};

use crate::config::GatewayConfig;
use crate::error::GatewayError;
use crate::message::{ClientRequest, ServerMessage};
use crate::session::{Session, SessionId, SessionState};
use crate::sink::GatewaySink;
use crate::stats::GatewayStats;

/// Result of servicing one downstream session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceReport<E> {
    pub incremental_sent: usize,
    pub control_sent: usize,
    pub remaining: usize,
    pub expired: bool,
    pub error: Option<E>,
}

impl<E> Default for ServiceReport<E> {
    fn default() -> Self {
        Self {
            incremental_sent: 0,
            control_sent: 0,
            remaining: 0,
            expired: false,
            error: None,
        }
    }
}

impl<E> ServiceReport<E> {
    #[inline]
    pub fn total_sent(&self) -> usize {
        self.incremental_sent + self.control_sent
    }

    #[inline]
    pub fn is_blocked(&self) -> bool {
        self.error.is_some()
    }
}

/// Adapts a session-aware gateway sink to the publisher's
/// publication sink interface.
struct SessionSink<'a, S> {
    session_id: SessionId,
    sink: &'a mut S,
}

impl<I, P, S> PublicationSink<I, P> for SessionSink<'_, S>
where
    I: Copy,
    P: Copy,
    S: GatewaySink<I, P>,
{
    type Error = S::Error;

    #[inline]
    fn try_send(&mut self, publication: &Publication<I, P>) -> Result<(), Self::Error> {
        let message = ServerMessage::Incremental(*publication);

        self.sink.try_send(self.session_id, &message)
    }
}

/// Deterministic gateway owning publisher and session state.
///
/// The application is responsible for:
///
/// - Authentication and authorization.
/// - Network connection establishment.
/// - Wire encoding and decoding.
/// - Consistent snapshot construction and delivery.
/// - Fair scheduling of session service calls.
/// - Closing expired or disconnected transports.
///
/// All timestamps supplied to gateway operations must use
/// the same monotonic clock domain.
#[derive(Debug)]
pub struct Gateway<I, P> {
    config: GatewayConfig,
    publisher: Publisher<I, P>,
    sessions: Vec<Session>,
    stats: GatewayStats,
    now_ns: u64,
}

impl<I, P> Gateway<I, P>
where
    I: Copy + Ord,
    P: Copy,
{
    pub fn new(config: GatewayConfig) -> Result<Self, GatewayError> {
        config.validate()?;

        let publisher = Publisher::new(config.publisher)?;

        let mut sessions = Vec::new();

        sessions
            .try_reserve_exact(config.publisher.max_subscribers)
            .map_err(|_| GatewayError::AllocationFailed)?;

        Ok(Self {
            config,
            publisher,
            sessions,
            stats: GatewayStats::default(),
            now_ns: 0,
        })
    }

    #[inline]
    pub fn config(&self) -> &GatewayConfig {
        &self.config
    }

    #[inline]
    pub fn stats(&self) -> &GatewayStats {
        &self.stats
    }

    #[inline]
    pub fn publisher_stats(&self) -> &PublisherStats {
        self.publisher.stats()
    }

    #[inline]
    pub fn publisher_watermark(&self) -> u64 {
        self.publisher.last_sequence()
    }

    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.now_ns
    }

    #[inline]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn active_session_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.is_active())
            .count()
    }

    pub fn sessions(&self) -> impl ExactSizeIterator<Item = &Session> {
        self.sessions.iter()
    }

    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.find_index(id).ok().map(|index| &self.sessions[index])
    }

    #[inline]
    fn find_index(&self, id: SessionId) -> Result<usize, usize> {
        self.sessions
            .binary_search_by_key(&id, |session| session.id())
    }

    #[inline]
    fn require_index(&self, id: SessionId) -> Result<usize, GatewayError> {
        self.find_index(id)
            .map_err(|_| GatewayError::SessionNotFound { session_id: id })
    }

    /// Advance the gateway's monotonic clock.
    fn advance_clock(&mut self, now_ns: u64) -> Result<(), GatewayError> {
        if now_ns < self.now_ns {
            return Err(GatewayError::ClockRegression {
                previous_ns: self.now_ns,
                received_ns: now_ns,
            });
        }

        self.now_ns = now_ns;

        Ok(())
    }

    /// Register an authenticated downstream connection.
    pub fn connect(&mut self, id: SessionId, now_ns: u64) -> Result<(), GatewayError> {
        self.advance_clock(now_ns)?;

        let insertion_index = match self.find_index(id) {
            Ok(_) => {
                return Err(GatewayError::SessionAlreadyExists { session_id: id });
            }

            Err(index) => index,
        };

        let limit = self.config.publisher.max_subscribers;

        if self.sessions.len() >= limit {
            return Err(GatewayError::SessionLimitReached { limit });
        }

        self.sessions
            .insert(insertion_index, Session::new(id, now_ns));

        GatewayStats::increment(&mut self.stats.sessions_connected);

        self.stats.observe_sessions(self.sessions.len());

        Ok(())
    }

    /// Remove a session and any associated publisher subscription.
    ///
    /// The application must also close the corresponding
    /// network connection and discard its pending transport data.
    pub fn disconnect(&mut self, id: SessionId) -> Result<(), GatewayError> {
        let index = self.require_index(id)?;

        if self.sessions[index].has_subscription() {
            self.publisher.unsubscribe(id.subscriber_id())?;

            GatewayStats::increment(&mut self.stats.subscriptions_removed);
        }

        self.sessions.remove(index);

        GatewayStats::increment(&mut self.stats.sessions_disconnected);

        Ok(())
    }

    fn subscribe_at(
        &mut self,
        index: usize,
        filter: SubscriptionFilter<I>,
    ) -> Result<(), GatewayError> {
        let id = self.sessions[index].id();

        if self.sessions[index].state() != SessionState::Connected {
            return Err(GatewayError::InvalidSessionState { session_id: id });
        }

        self.publisher.subscribe(id.subscriber_id(), filter)?;

        self.sessions[index].set_state(SessionState::AwaitingSnapshot);

        GatewayStats::increment(&mut self.stats.subscriptions_created);

        Ok(())
    }

    fn unsubscribe_at(&mut self, index: usize) -> Result<(), GatewayError> {
        let id = self.sessions[index].id();

        if !self.sessions[index].has_subscription() {
            return Err(GatewayError::InvalidSessionState { session_id: id });
        }

        self.publisher.unsubscribe(id.subscriber_id())?;

        self.sessions[index].set_state(SessionState::Connected);

        GatewayStats::increment(&mut self.stats.subscriptions_removed);

        Ok(())
    }

    fn invalidate_at(&mut self, index: usize) -> Result<(), GatewayError> {
        let id = self.sessions[index].id();

        if !self.sessions[index].has_subscription() {
            return Err(GatewayError::InvalidSessionState { session_id: id });
        }

        if self.sessions[index].state() == SessionState::ResyncRequired {
            return Ok(());
        }

        self.publisher.require_resync(id.subscriber_id())?;

        self.sessions[index].set_state(SessionState::ResyncRequired);

        GatewayStats::increment(&mut self.stats.resync_transitions);

        Ok(())
    }

    /// Synchronize session state with publisher invalidations.
    ///
    /// A publisher queue overflow invalidates the corresponding
    /// subscriber. The gateway mirrors that state before
    /// permitting further downstream delivery.
    fn reconcile_subscribers(&mut self) {
        for session in &mut self.sessions {
            if !session.has_subscription() {
                continue;
            }

            let Some(subscriber) = self.publisher.subscriber(session.id().subscriber_id()) else {
                debug_assert!(false, "missing publisher subscription");
                continue;
            };

            if subscriber.state() == SubscriberState::ResyncRequired
                && session.state() != SessionState::ResyncRequired
            {
                session.set_state(SessionState::ResyncRequired);

                GatewayStats::increment(&mut self.stats.resync_transitions);
            }
        }
    }

    /// Expire a session and release its publisher resources.
    ///
    /// The expired session remains in the registry until the
    /// application closes the transport and calls disconnect.
    fn expire_at(&mut self, index: usize) -> Result<bool, GatewayError> {
        if self.sessions[index].is_expired() {
            return Ok(true);
        }

        if !self.sessions[index].timeout_due(self.now_ns, self.config.session_timeout_ns) {
            return Ok(false);
        }

        let id = self.sessions[index].id();

        if self.sessions[index].has_subscription() {
            self.publisher.unsubscribe(id.subscriber_id())?;

            GatewayStats::increment(&mut self.stats.subscriptions_removed);
        }

        self.sessions[index].set_state(SessionState::Expired);

        GatewayStats::increment(&mut self.stats.sessions_expired);

        Ok(true)
    }

    /// Process one decoded and authenticated client request.
    pub fn handle_request(
        &mut self,
        id: SessionId,
        request: ClientRequest<I>,
        now_ns: u64,
    ) -> Result<(), GatewayError> {
        let result = self.handle_request_inner(id, request, now_ns);

        match &result {
            Ok(()) => {
                GatewayStats::increment(&mut self.stats.client_requests_accepted);
            }

            Err(_) => {
                GatewayStats::increment(&mut self.stats.client_requests_rejected);
            }
        }

        result
    }

    fn handle_request_inner(
        &mut self,
        id: SessionId,
        request: ClientRequest<I>,
        now_ns: u64,
    ) -> Result<(), GatewayError> {
        self.advance_clock(now_ns)?;

        let index = self.require_index(id)?;

        if matches!(&request, ClientRequest::Disconnect) {
            return self.disconnect(id);
        }

        if self.expire_at(index)? {
            return Err(GatewayError::SessionExpired { session_id: id });
        }

        match request {
            ClientRequest::Subscribe { filter } => {
                self.subscribe_at(index, filter)?;
            }

            ClientRequest::Unsubscribe => {
                self.unsubscribe_at(index)?;
            }

            ClientRequest::Heartbeat => {
                GatewayStats::increment(&mut self.stats.heartbeats_received);
            }

            ClientRequest::RequestResync => {
                self.invalidate_at(index)?;
            }

            ClientRequest::Disconnect => {
                return self.disconnect(id);
            }
        }

        self.sessions[index].record_rx(now_ns);

        Ok(())
    }

    /// Activate a session after its snapshot has been accepted
    /// by the ordered downstream transport.
    ///
    /// The caller must establish a snapshot at the current
    /// publisher watermark without interleaving publications
    /// between snapshot construction and activation.
    ///
    /// Old transport data must not be delivered after the
    /// new snapshot.
    pub fn activate_snapshot(
        &mut self,
        id: SessionId,
        watermark: u64,
        now_ns: u64,
    ) -> Result<(), GatewayError> {
        self.advance_clock(now_ns)?;

        let index = self.require_index(id)?;

        if self.expire_at(index)? {
            return Err(GatewayError::SessionExpired { session_id: id });
        }

        let state = self.sessions[index].state();

        if !matches!(
            state,
            SessionState::AwaitingSnapshot | SessionState::ResyncRequired
        ) {
            return Err(GatewayError::InvalidSessionState { session_id: id });
        }

        self.publisher.activate(id.subscriber_id(), watermark)?;

        self.sessions[index].set_state(SessionState::Active);
        self.sessions[index].record_tx(now_ns);

        GatewayStats::increment(&mut self.stats.snapshots_activated);

        Ok(())
    }

    /// Explicitly invalidate a session's market data baseline.
    pub fn require_resync(&mut self, id: SessionId) -> Result<(), GatewayError> {
        let index = self.require_index(id)?;

        self.invalidate_at(index)
    }

    /// Invalidate all registered subscriptions.
    pub fn require_resync_all(&mut self) -> usize {
        let discarded = self.publisher.require_resync_all();

        self.reconcile_subscribers();

        discarded
    }

    /// Publish an upstream market data event.
    ///
    /// The event timestamp is market data metadata and is
    /// independent of the gateway's monotonic session clock.
    pub fn publish(
        &mut self,
        instrument: Option<I>,
        kind: EventKind,
        timestamp_ns: u64,
        payload: P,
    ) -> Result<PublishReport, GatewayError> {
        let report = self
            .publisher
            .publish(instrument, kind, timestamp_ns, payload)?;

        if report.resync_required != 0 {
            self.reconcile_subscribers();
        }

        Ok(report)
    }

    /// Check every session for expiration.
    ///
    /// Expiration order is deterministic because sessions
    /// are stored in ascending identifier order.
    pub fn expire_sessions(&mut self, now_ns: u64) -> Result<usize, GatewayError> {
        self.advance_clock(now_ns)?;

        let mut newly_expired = 0;

        for index in 0..self.sessions.len() {
            if self.sessions[index].is_expired() {
                continue;
            }

            if self.expire_at(index)? {
                newly_expired += 1;
            }
        }

        Ok(newly_expired)
    }

    /// Service one downstream session.
    ///
    /// Priority:
    ///
    /// 1. Session expiration.
    /// 2. Required resynchronization notification.
    /// 3. Incremental market data.
    /// 4. Idle heartbeat.
    ///
    /// A sink failure stops processing immediately.
    pub fn service<S>(
        &mut self,
        id: SessionId,
        sink: &mut S,
        now_ns: u64,
    ) -> Result<ServiceReport<S::Error>, GatewayError>
    where
        S: GatewaySink<I, P>,
    {
        self.advance_clock(now_ns)?;

        let index = self.require_index(id)?;
        let mut report = ServiceReport::default();

        if self.expire_at(index)? {
            report.expired = true;

            return Ok(report);
        }

        let budget = self.config.max_messages_per_service;

        if self.sessions[index].state() == SessionState::ResyncRequired
            && !self.sessions[index].resync_notified()
        {
            let message = ServerMessage::ResyncRequired {
                publisher_watermark: self.publisher_watermark(),
            };

            match sink.try_send(id, &message) {
                Ok(()) => {
                    self.sessions[index].mark_resync_notified();
                    self.sessions[index].record_tx(now_ns);

                    report.control_sent += 1;

                    GatewayStats::increment(&mut self.stats.resync_notifications_sent);
                }

                Err(error) => {
                    GatewayStats::increment(&mut self.stats.sink_failures);

                    report.error = Some(error);

                    return Ok(report);
                }
            }
        }

        if self.sessions[index].is_active() {
            let remaining_budget = budget - report.total_sent();

            let mut adapter = SessionSink {
                session_id: id,
                sink,
            };

            let drained =
                self.publisher
                    .drain(id.subscriber_id(), &mut adapter, remaining_budget)?;

            report.incremental_sent = drained.sent;
            report.remaining = drained.remaining;

            GatewayStats::add(&mut self.stats.incremental_messages_sent, drained.sent);

            if drained.sent != 0 {
                self.sessions[index].record_tx(now_ns);
            }

            if let Some(error) = drained.error {
                GatewayStats::increment(&mut self.stats.sink_failures);

                report.error = Some(error);

                return Ok(report);
            }
        }

        if report.total_sent() < budget
            && self.sessions[index].heartbeat_due(now_ns, self.config.heartbeat_interval_ns)
        {
            let message = ServerMessage::Heartbeat {
                timestamp_ns: now_ns,
                publisher_watermark: self.publisher_watermark(),
            };

            match sink.try_send(id, &message) {
                Ok(()) => {
                    self.sessions[index].record_tx(now_ns);

                    report.control_sent += 1;

                    GatewayStats::increment(&mut self.stats.heartbeats_sent);
                }

                Err(error) => {
                    GatewayStats::increment(&mut self.stats.sink_failures);

                    report.error = Some(error);
                }
            }
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestGateway = Gateway<u32, u64>;

    fn gateway() -> TestGateway {
        Gateway::new(GatewayConfig::default()).unwrap()
    }

    #[test]
    fn starts_empty() {
        let gateway = gateway();

        assert_eq!(gateway.session_count(), 0);
        assert_eq!(gateway.publisher_watermark(), 0);
    }

    #[test]
    fn connects_session() {
        let mut gateway = gateway();
        let id = SessionId::new(1);

        gateway.connect(id, 100).unwrap();

        assert_eq!(gateway.session_count(), 1);

        assert_eq!(
            gateway.session(id).unwrap().state(),
            SessionState::Connected
        );
    }

    #[test]
    fn rejects_duplicate_session() {
        let mut gateway = gateway();
        let id = SessionId::new(1);

        gateway.connect(id, 100).unwrap();

        assert_eq!(
            gateway.connect(id, 100),
            Err(GatewayError::SessionAlreadyExists { session_id: id })
        );
    }

    #[test]
    fn maintains_sorted_session_order() {
        let mut gateway = gateway();

        for value in [5, 2, 4, 1, 3] {
            gateway.connect(SessionId::new(value), 100).unwrap();
        }

        let ids: Vec<_> = gateway
            .sessions()
            .map(|session| session.id().get())
            .collect();

        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn rejects_clock_regression() {
        let mut gateway = gateway();

        gateway.connect(SessionId::new(1), 100).unwrap();

        assert_eq!(
            gateway.connect(SessionId::new(2), 99),
            Err(GatewayError::ClockRegression {
                previous_ns: 100,
                received_ns: 99,
            })
        );
    }

    #[test]
    fn subscription_requires_snapshot() {
        let mut gateway = gateway();
        let id = SessionId::new(1);

        gateway.connect(id, 100).unwrap();

        gateway
            .handle_request(
                id,
                ClientRequest::Subscribe {
                    filter: SubscriptionFilter::all(),
                },
                100,
            )
            .unwrap();

        assert_eq!(
            gateway.session(id).unwrap().state(),
            SessionState::AwaitingSnapshot
        );

        gateway.activate_snapshot(id, 0, 100).unwrap();

        assert!(gateway.session(id).unwrap().is_active());
    }

    #[test]
    fn disconnect_removes_session() {
        let mut gateway = gateway();
        let id = SessionId::new(1);

        gateway.connect(id, 100).unwrap();
        gateway.disconnect(id).unwrap();

        assert_eq!(gateway.session_count(), 0);
        assert!(gateway.session(id).is_none());
    }

    #[test]
    fn enforces_session_limit() {
        let mut config = GatewayConfig::default();
        config.publisher.max_subscribers = 1;

        let mut gateway = TestGateway::new(config).unwrap();

        gateway.connect(SessionId::new(1), 100).unwrap();

        assert_eq!(
            gateway.connect(SessionId::new(2), 100),
            Err(GatewayError::SessionLimitReached { limit: 1 })
        );
    }
}
