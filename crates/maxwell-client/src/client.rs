//! Deterministic single-owner market data client.

use std::marker::PhantomData;

use maxwell_gateway::{ClientRequest, ServerMessage};
use maxwell_publisher::Publication;

use crate::config::ClientConfig;
use crate::error::ClientError;
use crate::handler::MarketDataHandler;
use crate::message::{InboundFrame, InboundMessage};
use crate::sequence::{SequenceStatus, SequenceTracker};
use crate::state::{ClientState, RecoveryReason};
use crate::stats::ClientStats;

/// Result of processing one inbound frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveOutcome {
    SnapshotStarted,
    SnapshotEntryAccepted,
    SnapshotInstalled,
    IncrementalApplied,
    IncrementalIgnored,
    HeartbeatReceived,
    DuplicateIgnored,
    RecoveryRequired(RecoveryReason),
}

/// Metadata for a snapshot currently being assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapshotTransaction {
    publisher_watermark: u64,
    expected_entries: usize,
}

/// Single-owner market data client.
///
/// The application owns network connections, wire decoding,
/// outbound requests and transport sequence assignment.
///
/// Snapshot storage is allocated once during construction.
/// The incremental processing path performs no allocations
/// within this client.
///
/// Handler implementations must independently enforce
/// their own memory and latency requirements.
#[derive(Debug)]
pub struct Client<I, P, S, H> {
    config: ClientConfig,
    state: ClientState,
    recovery_reason: Option<RecoveryReason>,

    sequence: SequenceTracker,

    publisher_watermark: Option<u64>,
    minimum_snapshot_watermark: u64,
    server_watermark: Option<u64>,

    snapshot: Option<SnapshotTransaction>,
    snapshot_entries: Vec<S>,

    reconnect_required: bool,
    recovery_request_sent: bool,

    now_ns: u64,
    last_rx_ns: u64,
    last_tx_ns: u64,

    handler: H,
    stats: ClientStats,

    marker: PhantomData<(I, P)>,
}

impl<I, P, S, H> Client<I, P, S, H>
where
    I: Copy + Ord,
    P: Copy,
    S: Copy,
    H: MarketDataHandler<I, P, S>,
{
    pub fn new(config: ClientConfig, handler: H) -> Result<Self, ClientError> {
        config.validate()?;

        let mut snapshot_entries = Vec::new();

        snapshot_entries
            .try_reserve_exact(config.max_snapshot_entries)
            .map_err(|_| ClientError::AllocationFailed)?;

        Ok(Self {
            config,
            state: ClientState::Disconnected,
            recovery_reason: None,

            sequence: SequenceTracker::new(),

            publisher_watermark: None,
            minimum_snapshot_watermark: 0,
            server_watermark: None,

            snapshot: None,
            snapshot_entries,

            reconnect_required: false,
            recovery_request_sent: false,

            now_ns: 0,
            last_rx_ns: 0,
            last_tx_ns: 0,

            handler,
            stats: ClientStats::default(),

            marker: PhantomData,
        })
    }

    #[inline]
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    #[inline]
    pub fn state(&self) -> ClientState {
        self.state
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    #[inline]
    pub fn recovery_reason(&self) -> Option<RecoveryReason> {
        self.recovery_reason
    }

    #[inline]
    pub fn reconnect_required(&self) -> bool {
        self.reconnect_required
    }

    #[inline]
    pub fn stats(&self) -> &ClientStats {
        &self.stats
    }

    #[inline]
    pub fn handler(&self) -> &H {
        &self.handler
    }

    #[inline]
    pub fn publisher_watermark(&self) -> Option<u64> {
        self.publisher_watermark
    }

    #[inline]
    pub fn server_watermark(&self) -> Option<u64> {
        self.server_watermark
    }

    #[inline]
    pub fn last_transport_sequence(&self) -> u64 {
        self.sequence.last_accepted()
    }

    #[inline]
    pub fn next_transport_sequence(&self) -> Option<u64> {
        self.sequence.next_expected()
    }

    #[inline]
    pub fn snapshot_entry_count(&self) -> usize {
        self.snapshot_entries.len()
    }

    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.now_ns
    }

    #[inline]
    pub fn last_rx_ns(&self) -> u64 {
        self.last_rx_ns
    }

    #[inline]
    pub fn last_tx_ns(&self) -> u64 {
        self.last_tx_ns
    }

    #[inline]
    fn invalid_state(&self, operation: &'static str) -> ClientError {
        ClientError::InvalidState {
            state: self.state,
            operation,
        }
    }

    fn advance_clock(&mut self, now_ns: u64) -> Result<(), ClientError> {
        if now_ns < self.now_ns {
            return Err(ClientError::ClockRegression {
                previous_ns: self.now_ns,
                received_ns: now_ns,
            });
        }

        self.now_ns = now_ns;

        Ok(())
    }

    fn abort_snapshot(&mut self) {
        if self.snapshot.take().is_some() {
            ClientStats::increment(&mut self.stats.snapshots_aborted);
        }

        self.snapshot_entries.clear();
    }

    fn reset_connection_state(&mut self) {
        self.abort_snapshot();

        self.sequence.reset();

        self.publisher_watermark = None;
        self.minimum_snapshot_watermark = 0;
        self.server_watermark = None;

        self.recovery_reason = None;
        self.reconnect_required = false;
        self.recovery_request_sent = false;
    }

    /// Establish a new transport connection.
    ///
    /// The caller must send a subscription request and
    /// arrange delivery of the initial snapshot.
    pub fn connect(&mut self, now_ns: u64) -> Result<(), ClientError> {
        self.advance_clock(now_ns)?;

        if self.state != ClientState::Disconnected {
            return Err(self.invalid_state("connect"));
        }

        self.reset_connection_state();

        self.state = ClientState::AwaitingSnapshot;
        self.last_rx_ns = now_ns;
        self.last_tx_ns = now_ns;

        ClientStats::increment(&mut self.stats.connections_established);

        Ok(())
    }

    /// Close the current logical connection.
    ///
    /// The application must also close its underlying
    /// transport and discard outstanding connection data.
    pub fn disconnect(&mut self) {
        if self.state == ClientState::Disconnected {
            return;
        }

        self.reset_connection_state();

        self.state = ClientState::Disconnected;

        self.handler.on_disconnected();

        ClientStats::increment(&mut self.stats.connections_closed);
    }

    fn enter_recovery(&mut self, reason: RecoveryReason, reconnect_required: bool) {
        self.abort_snapshot();

        if let Some(watermark) = self.publisher_watermark {
            self.minimum_snapshot_watermark = self.minimum_snapshot_watermark.max(watermark);
        }

        self.publisher_watermark = None;

        self.reconnect_required |= reconnect_required;

        if self.state != ClientState::RecoveryRequired {
            self.state = ClientState::RecoveryRequired;
            self.recovery_request_sent = false;

            ClientStats::increment(&mut self.stats.recovery_transitions);

            self.handler.on_invalidated(reason);
        }

        self.recovery_reason = Some(reason);
    }

    /// Explicitly invalidate the current baseline.
    pub fn require_recovery(&mut self) -> Result<(), ClientError> {
        if !self.state.is_connected() {
            return Err(self.invalid_state("require_recovery"));
        }

        self.enter_recovery(RecoveryReason::ExplicitRequest, false);

        Ok(())
    }

    /// Return a recovery request if one is needed.
    ///
    /// The request remains pending until the application
    /// confirms successful transport acceptance.
    pub fn pending_recovery_request(&self) -> Option<ClientRequest<I>> {
        if self.state == ClientState::RecoveryRequired
            && !self.reconnect_required
            && !self.recovery_request_sent
        {
            Some(ClientRequest::RequestResync)
        } else {
            None
        }
    }

    /// Confirm successful acceptance of a recovery request.
    pub fn mark_recovery_request_sent(&mut self, now_ns: u64) -> Result<(), ClientError> {
        self.advance_clock(now_ns)?;

        if self.check_timeout() {
            return Err(self.invalid_state("mark_recovery_request_sent"));
        }

        if self.pending_recovery_request().is_none() {
            return Err(self.invalid_state("mark_recovery_request_sent"));
        }

        self.recovery_request_sent = true;
        self.last_tx_ns = now_ns;

        Ok(())
    }

    /// Determine whether an outbound heartbeat is due.
    pub fn heartbeat_due(&self) -> bool {
        self.state.is_connected()
            && !self.reconnect_required
            && self.now_ns.saturating_sub(self.last_tx_ns) >= self.config.heartbeat_interval_ns
    }

    pub fn pending_heartbeat(&self) -> Option<ClientRequest<I>> {
        if self.heartbeat_due() {
            Some(ClientRequest::Heartbeat)
        } else {
            None
        }
    }

    /// Record a heartbeat only after successful acceptance
    /// by the outbound transport.
    pub fn mark_heartbeat_sent(&mut self, now_ns: u64) -> Result<(), ClientError> {
        self.advance_clock(now_ns)?;

        if self.check_timeout() || !self.heartbeat_due() {
            return Err(self.invalid_state("mark_heartbeat_sent"));
        }

        self.last_tx_ns = now_ns;

        ClientStats::increment(&mut self.stats.heartbeats_sent);

        Ok(())
    }

    /// Record successful acceptance of another outbound
    /// request, such as the initial subscription request.
    pub fn record_outbound(&mut self, now_ns: u64) -> Result<(), ClientError> {
        self.advance_clock(now_ns)?;

        if self.check_timeout() || !self.state.is_connected() || self.reconnect_required {
            return Err(self.invalid_state("record_outbound"));
        }

        self.last_tx_ns = now_ns;

        Ok(())
    }

    /// Advance the client clock and evaluate server timeout.
    ///
    /// Returns true if the client is timed out.
    pub fn tick(&mut self, now_ns: u64) -> Result<bool, ClientError> {
        self.advance_clock(now_ns)?;

        Ok(self.check_timeout())
    }

    fn check_timeout(&mut self) -> bool {
        if self.state == ClientState::TimedOut {
            return true;
        }

        if !self.state.is_connected() {
            return false;
        }

        if self.now_ns.saturating_sub(self.last_rx_ns) < self.config.server_timeout_ns {
            return false;
        }

        self.abort_snapshot();

        self.publisher_watermark = None;
        self.state = ClientState::TimedOut;
        self.recovery_reason = Some(RecoveryReason::ServerTimeout);
        self.reconnect_required = true;

        self.handler.on_invalidated(RecoveryReason::ServerTimeout);

        ClientStats::increment(&mut self.stats.connection_timeouts);

        true
    }

    /// Process a decoded inbound transport frame.
    ///
    /// In-order frames consume their transport sequence
    /// even if their logical message is subsequently rejected.
    ///
    /// A transport gap invalidates the connection. No later
    /// frames are processed until the connection is replaced.
    pub fn receive(
        &mut self,
        frame: InboundFrame<I, P, S>,
        now_ns: u64,
    ) -> Result<ReceiveOutcome, ClientError> {
        self.advance_clock(now_ns)?;

        ClientStats::increment(&mut self.stats.frames_received);

        let result = self.receive_inner(frame);

        if result.is_err() {
            ClientStats::increment(&mut self.stats.frames_rejected);
        }

        result
    }

    fn receive_inner(
        &mut self,
        frame: InboundFrame<I, P, S>,
    ) -> Result<ReceiveOutcome, ClientError> {
        if self.check_timeout() || !self.state.is_connected() || self.reconnect_required {
            return Err(self.invalid_state("receive"));
        }

        let status = match self.sequence.observe(frame.transport_sequence) {
            Ok(status) => status,

            Err(error) => {
                self.enter_recovery(RecoveryReason::UnexpectedMessage, true);

                return Err(error);
            }
        };

        match status {
            SequenceStatus::Duplicate { .. } => {
                ClientStats::increment(&mut self.stats.duplicate_frames);

                return Ok(ReceiveOutcome::DuplicateIgnored);
            }

            SequenceStatus::Gap { expected, received } => {
                ClientStats::increment(&mut self.stats.transport_gaps);

                ClientStats::increment(&mut self.stats.frames_rejected);

                let reason = RecoveryReason::TransportGap { expected, received };

                self.enter_recovery(reason, true);

                return Ok(ReceiveOutcome::RecoveryRequired(reason));
            }

            SequenceStatus::InOrder => {}
        }

        self.last_rx_ns = self.now_ns;

        let outcome = self.process_message(frame.message);

        // A connection cannot safely continue after consuming
        // the final representable transport sequence.
        if self.sequence.next_expected().is_none() {
            let reason = RecoveryReason::TransportSequenceExhausted;

            self.enter_recovery(reason, true);

            if outcome.is_ok() {
                ClientStats::increment(&mut self.stats.frames_accepted);
            }

            return outcome.map(|_| ReceiveOutcome::RecoveryRequired(reason));
        }

        if outcome.is_ok() {
            ClientStats::increment(&mut self.stats.frames_accepted);
        }

        outcome
    }

    fn process_message(
        &mut self,
        message: InboundMessage<I, P, S>,
    ) -> Result<ReceiveOutcome, ClientError> {
        match message {
            InboundMessage::SnapshotBegin {
                publisher_watermark,
                entry_count,
            } => self.begin_snapshot(publisher_watermark, entry_count),

            InboundMessage::SnapshotEntry(entry) => self.append_snapshot_entry(entry),

            InboundMessage::SnapshotEnd {
                publisher_watermark,
            } => self.finish_snapshot(publisher_watermark),

            InboundMessage::Gateway(message) => self.process_gateway_message(message),
        }
    }

    fn begin_snapshot(
        &mut self,
        watermark: u64,
        entry_count: usize,
    ) -> Result<ReceiveOutcome, ClientError> {
        if !matches!(
            self.state,
            ClientState::AwaitingSnapshot | ClientState::RecoveryRequired
        ) {
            self.enter_recovery(RecoveryReason::UnexpectedMessage, false);

            return Err(self.invalid_state("begin_snapshot"));
        }

        if entry_count > self.config.max_snapshot_entries {
            self.enter_recovery(RecoveryReason::InvalidSnapshot, false);

            return Err(ClientError::SnapshotCapacityExceeded {
                limit: self.config.max_snapshot_entries,
                received: entry_count,
            });
        }

        if watermark < self.minimum_snapshot_watermark {
            self.enter_recovery(RecoveryReason::InvalidSnapshot, false);

            return Err(ClientError::SnapshotWatermarkRegression {
                minimum: self.minimum_snapshot_watermark,
                received: watermark,
            });
        }

        self.snapshot_entries.clear();

        self.snapshot = Some(SnapshotTransaction {
            publisher_watermark: watermark,
            expected_entries: entry_count,
        });

        self.state = ClientState::ReceivingSnapshot;
        self.recovery_request_sent = false;

        ClientStats::increment(&mut self.stats.snapshots_started);

        Ok(ReceiveOutcome::SnapshotStarted)
    }

    fn append_snapshot_entry(&mut self, entry: S) -> Result<ReceiveOutcome, ClientError> {
        let Some(snapshot) = self.snapshot else {
            self.enter_recovery(RecoveryReason::UnexpectedMessage, false);

            return Err(self.invalid_state("append_snapshot_entry"));
        };

        if self.snapshot_entries.len() >= snapshot.expected_entries {
            let received = self.snapshot_entries.len().saturating_add(1);

            self.enter_recovery(RecoveryReason::InvalidSnapshot, false);

            return Err(ClientError::SnapshotCountMismatch {
                expected: snapshot.expected_entries,
                received,
            });
        }

        self.snapshot_entries.push(entry);

        ClientStats::increment(&mut self.stats.snapshot_entries_received);

        self.stats
            .observe_snapshot_depth(self.snapshot_entries.len());

        Ok(ReceiveOutcome::SnapshotEntryAccepted)
    }

    fn finish_snapshot(&mut self, watermark: u64) -> Result<ReceiveOutcome, ClientError> {
        let Some(snapshot) = self.snapshot else {
            self.enter_recovery(RecoveryReason::UnexpectedMessage, false);

            return Err(self.invalid_state("finish_snapshot"));
        };

        if watermark != snapshot.publisher_watermark {
            self.enter_recovery(RecoveryReason::InvalidSnapshot, false);

            return Err(ClientError::SnapshotWatermarkMismatch {
                expected: snapshot.publisher_watermark,
                received: watermark,
            });
        }

        let received = self.snapshot_entries.len();

        if received != snapshot.expected_entries {
            self.enter_recovery(RecoveryReason::InvalidSnapshot, false);

            return Err(ClientError::SnapshotCountMismatch {
                expected: snapshot.expected_entries,
                received,
            });
        }

        if self
            .handler
            .install_snapshot(watermark, &self.snapshot_entries)
            .is_err()
        {
            ClientStats::increment(&mut self.stats.handler_failures);

            self.enter_recovery(RecoveryReason::HandlerFailure, false);

            return Err(ClientError::HandlerFailure);
        }

        self.snapshot = None;
        self.snapshot_entries.clear();

        self.publisher_watermark = Some(watermark);
        self.minimum_snapshot_watermark = watermark;

        self.state = ClientState::Active;
        self.recovery_reason = None;
        self.recovery_request_sent = false;

        ClientStats::increment(&mut self.stats.snapshots_completed);

        Ok(ReceiveOutcome::SnapshotInstalled)
    }

    fn observe_server_watermark(&mut self, watermark: u64) {
        self.server_watermark = Some(
            self.server_watermark
                .map_or(watermark, |previous| previous.max(watermark)),
        );
    }

    fn process_gateway_message(
        &mut self,
        message: ServerMessage<I, P>,
    ) -> Result<ReceiveOutcome, ClientError> {
        match message {
            ServerMessage::Incremental(publication) => self.apply_incremental(&publication),

            ServerMessage::Heartbeat {
                publisher_watermark,
                ..
            } => {
                self.observe_server_watermark(publisher_watermark);

                ClientStats::increment(&mut self.stats.heartbeats_received);

                Ok(ReceiveOutcome::HeartbeatReceived)
            }

            ServerMessage::ResyncRequired {
                publisher_watermark,
            } => {
                self.observe_server_watermark(publisher_watermark);

                self.minimum_snapshot_watermark =
                    self.minimum_snapshot_watermark.max(publisher_watermark);

                let reason = RecoveryReason::ServerRequested {
                    publisher_watermark,
                };

                self.enter_recovery(reason, false);

                Ok(ReceiveOutcome::RecoveryRequired(reason))
            }
        }
    }

    fn apply_incremental(
        &mut self,
        publication: &Publication<I, P>,
    ) -> Result<ReceiveOutcome, ClientError> {
        if self.state == ClientState::ReceivingSnapshot {
            let reason = RecoveryReason::UnexpectedMessage;

            self.enter_recovery(reason, false);

            return Ok(ReceiveOutcome::RecoveryRequired(reason));
        }

        if !self.state.is_active() {
            ClientStats::increment(&mut self.stats.incrementals_ignored);

            return Ok(ReceiveOutcome::IncrementalIgnored);
        }

        let Some(previous) = self.publisher_watermark else {
            let reason = RecoveryReason::UnexpectedMessage;

            self.enter_recovery(reason, true);

            return Ok(ReceiveOutcome::RecoveryRequired(reason));
        };

        // Publisher sequences need only increase.
        //
        // Filtering may legitimately produce gaps in the
        // global publication sequence.
        if publication.sequence <= previous {
            ClientStats::increment(&mut self.stats.publication_regressions);

            let reason = RecoveryReason::PublicationRegression {
                previous,
                received: publication.sequence,
            };

            self.enter_recovery(reason, false);

            return Ok(ReceiveOutcome::RecoveryRequired(reason));
        }

        if self.handler.apply_incremental(publication).is_err() {
            ClientStats::increment(&mut self.stats.handler_failures);

            self.minimum_snapshot_watermark =
                self.minimum_snapshot_watermark.max(publication.sequence);

            self.enter_recovery(RecoveryReason::HandlerFailure, false);

            return Err(ClientError::HandlerFailure);
        }

        self.publisher_watermark = Some(publication.sequence);

        self.observe_server_watermark(publication.sequence);

        ClientStats::increment(&mut self.stats.incrementals_applied);

        Ok(ReceiveOutcome::IncrementalApplied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct TestHandler;

    impl MarketDataHandler<u32, u64, u64> for TestHandler {
        type Error = ();

        fn install_snapshot(
            &mut self,
            _watermark: u64,
            _entries: &[u64],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn apply_incremental(
            &mut self,
            _publication: &Publication<u32, u64>,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn on_invalidated(&mut self, _reason: RecoveryReason) {}

        fn on_disconnected(&mut self) {}
    }

    type TestClient = Client<u32, u64, u64, TestHandler>;

    fn client() -> TestClient {
        Client::new(
            ClientConfig {
                heartbeat_interval_ns: 50,
                server_timeout_ns: 150,
                max_snapshot_entries: 8,
            },
            TestHandler,
        )
        .unwrap()
    }

    #[test]
    fn starts_disconnected() {
        let client = client();

        assert_eq!(client.state(), ClientState::Disconnected);
        assert_eq!(client.publisher_watermark(), None);
        assert_eq!(client.next_transport_sequence(), Some(1));
    }

    #[test]
    fn connects_and_awaits_snapshot() {
        let mut client = client();

        client.connect(100).unwrap();

        assert_eq!(client.state(), ClientState::AwaitingSnapshot);

        assert_eq!(client.last_rx_ns(), 100);
        assert_eq!(client.last_tx_ns(), 100);
    }

    #[test]
    fn rejects_duplicate_connection() {
        let mut client = client();

        client.connect(100).unwrap();

        assert!(client.connect(100).is_err());
    }

    #[test]
    fn rejects_clock_regression() {
        let mut client = client();

        client.connect(100).unwrap();

        assert_eq!(
            client.tick(99),
            Err(ClientError::ClockRegression {
                previous_ns: 100,
                received_ns: 99,
            })
        );
    }

    #[test]
    fn disconnect_is_idempotent() {
        let mut client = client();

        client.connect(100).unwrap();
        client.disconnect();
        client.disconnect();

        assert_eq!(client.state(), ClientState::Disconnected);
        assert_eq!(client.stats().connections_closed, 1);
    }

    #[test]
    fn detects_heartbeat_boundary() {
        let mut client = client();

        client.connect(100).unwrap();

        client.tick(149).unwrap();
        assert!(!client.heartbeat_due());

        client.tick(150).unwrap();
        assert!(client.heartbeat_due());

        client.mark_heartbeat_sent(150).unwrap();

        assert!(!client.heartbeat_due());
        assert_eq!(client.stats().heartbeats_sent, 1);
    }

    #[test]
    fn detects_timeout_boundary() {
        let mut client = client();

        client.connect(100).unwrap();

        assert!(!client.tick(249).unwrap());
        assert!(client.tick(250).unwrap());

        assert_eq!(client.state(), ClientState::TimedOut);
        assert!(client.reconnect_required());
    }

    #[test]
    fn timeout_is_idempotent() {
        let mut client = client();

        client.connect(100).unwrap();

        client.tick(250).unwrap();
        client.tick(300).unwrap();

        assert_eq!(client.stats().connection_timeouts, 1);
    }

    #[test]
    fn reconnect_resets_transport_sequence() {
        let mut client = client();

        client.connect(100).unwrap();

        client
            .receive(
                InboundFrame::new(
                    1,
                    InboundMessage::Gateway(ServerMessage::Heartbeat {
                        timestamp_ns: 110,
                        publisher_watermark: 0,
                    }),
                ),
                110,
            )
            .unwrap();

        client.disconnect();
        client.connect(200).unwrap();

        assert_eq!(client.next_transport_sequence(), Some(1));
        assert_eq!(client.server_watermark(), None);
    }

    #[test]
    fn snapshot_storage_does_not_grow() {
        let mut client = client();

        let capacity = client.snapshot_entries.capacity();

        client.connect(0).unwrap();

        for cycle in 0u64..100 {
            let sequence = client.next_transport_sequence().unwrap();

            client
                .receive(
                    InboundFrame::new(
                        sequence,
                        InboundMessage::SnapshotBegin {
                            publisher_watermark: cycle,
                            entry_count: 8,
                        },
                    ),
                    0,
                )
                .unwrap();

            for entry in 0u64..8 {
                let sequence = client.next_transport_sequence().unwrap();

                client
                    .receive(
                        InboundFrame::new(sequence, InboundMessage::SnapshotEntry(entry)),
                        0,
                    )
                    .unwrap();
            }

            let sequence = client.next_transport_sequence().unwrap();

            client
                .receive(
                    InboundFrame::new(
                        sequence,
                        InboundMessage::SnapshotEnd {
                            publisher_watermark: cycle,
                        },
                    ),
                    0,
                )
                .unwrap();

            assert_eq!(client.snapshot_entries.capacity(), capacity);

            client.require_recovery().unwrap();
        }
    }
}
