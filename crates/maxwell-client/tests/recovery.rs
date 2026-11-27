//! Integration tests for transport gaps, duplicates,
//! recovery requests and heartbeat monitoring.

use maxwell_client::{
    Client, ClientConfig, ClientError, ClientState, InboundFrame, InboundMessage,
    MarketDataHandler, ReceiveOutcome, RecoveryReason,
};

use maxwell_gateway::{ClientRequest, ServerMessage};
use maxwell_publisher::{EventKind, Publication};

#[derive(Debug, Default)]
struct TestHandler {
    valid: bool,
    applied: Vec<u64>,
    invalidations: Vec<RecoveryReason>,
}

impl MarketDataHandler<u32, u64, u64> for TestHandler {
    type Error = ();

    fn install_snapshot(&mut self, _watermark: u64, _entries: &[u64]) -> Result<(), Self::Error> {
        self.valid = true;
        self.applied.clear();

        Ok(())
    }

    fn apply_incremental(
        &mut self,
        publication: &Publication<u32, u64>,
    ) -> Result<(), Self::Error> {
        if !self.valid {
            return Err(());
        }

        self.applied.push(publication.sequence);

        Ok(())
    }

    fn on_invalidated(&mut self, reason: RecoveryReason) {
        self.valid = false;
        self.invalidations.push(reason);
    }

    fn on_disconnected(&mut self) {
        self.valid = false;
    }
}

type TestClient = Client<u32, u64, u64, TestHandler>;
type TestFrame = InboundFrame<u32, u64, u64>;

fn client() -> TestClient {
    let mut client = Client::new(
        ClientConfig {
            heartbeat_interval_ns: 50,
            server_timeout_ns: 150,
            max_snapshot_entries: 16,
        },
        TestHandler::default(),
    )
    .unwrap();

    client.connect(0).unwrap();

    client
}

fn activate(client: &mut TestClient, watermark: u64) {
    let sequence = client.next_transport_sequence().unwrap();
    let now = client.now_ns();

    client
        .receive(
            InboundFrame::new(
                sequence,
                InboundMessage::SnapshotBegin {
                    publisher_watermark: watermark,
                    entry_count: 0,
                },
            ),
            now,
        )
        .unwrap();

    client
        .receive(
            InboundFrame::new(
                sequence + 1,
                InboundMessage::SnapshotEnd {
                    publisher_watermark: watermark,
                },
            ),
            now,
        )
        .unwrap();
}

fn incremental(transport_sequence: u64, publication_sequence: u64) -> TestFrame {
    InboundFrame::new(
        transport_sequence,
        InboundMessage::Gateway(ServerMessage::Incremental(Publication::new(
            publication_sequence,
            Some(100),
            EventKind::Trade,
            1000,
            publication_sequence,
        ))),
    )
}

fn heartbeat(transport_sequence: u64, watermark: u64) -> TestFrame {
    InboundFrame::new(
        transport_sequence,
        InboundMessage::Gateway(ServerMessage::Heartbeat {
            timestamp_ns: 1000,
            publisher_watermark: watermark,
        }),
    )
}

#[test]
fn transport_duplicate_is_not_applied_twice() {
    let mut client = client();

    activate(&mut client, 100);

    let frame = incremental(3, 101);

    assert_eq!(
        client.receive(frame, 10),
        Ok(ReceiveOutcome::IncrementalApplied)
    );

    assert_eq!(
        client.receive(frame, 20),
        Ok(ReceiveOutcome::DuplicateIgnored)
    );

    assert_eq!(client.handler().applied, vec![101]);
    assert_eq!(client.stats().duplicate_frames, 1);
    assert_eq!(client.last_transport_sequence(), 3);
}

#[test]
fn transport_gap_requires_reconnection() {
    let mut client = client();

    activate(&mut client, 100);

    let reason = RecoveryReason::TransportGap {
        expected: 3,
        received: 4,
    };

    assert_eq!(
        client.receive(incremental(4, 101), 10),
        Ok(ReceiveOutcome::RecoveryRequired(reason))
    );

    assert_eq!(client.state(), ClientState::RecoveryRequired);

    assert!(client.reconnect_required());
    assert!(!client.handler().valid);
    assert_eq!(client.stats().transport_gaps, 1);
    assert_eq!(client.last_transport_sequence(), 2);
}

#[test]
fn late_missing_frame_cannot_revive_connection() {
    let mut client = client();

    activate(&mut client, 100);

    client.receive(incremental(4, 102), 10).unwrap();

    assert!(client.receive(incremental(3, 101), 20).is_err());
    assert!(client.handler().applied.is_empty());
    assert!(client.reconnect_required());
}

#[test]
fn reconnect_restores_sequence_tracking() {
    let mut client = client();

    activate(&mut client, 100);

    client.receive(incremental(4, 102), 10).unwrap();

    client.disconnect();
    client.connect(20).unwrap();

    activate(&mut client, 200);

    assert_eq!(
        client.receive(incremental(3, 201), 30),
        Ok(ReceiveOutcome::IncrementalApplied)
    );

    assert!(client.is_active());
    assert!(!client.reconnect_required());
    assert_eq!(client.handler().applied, vec![201]);
}

#[test]
fn publication_regression_requires_snapshot_recovery() {
    let mut client = client();

    activate(&mut client, 100);

    client.receive(incremental(3, 110), 10).unwrap();

    let reason = RecoveryReason::PublicationRegression {
        previous: 110,
        received: 109,
    };

    assert_eq!(
        client.receive(incremental(4, 109), 20),
        Ok(ReceiveOutcome::RecoveryRequired(reason))
    );

    assert!(!client.reconnect_required());
    assert_eq!(client.stats().publication_regressions, 1);
    assert_eq!(client.handler().applied, vec![110]);
}

#[test]
fn repeated_publication_on_new_frame_is_rejected() {
    let mut client = client();

    activate(&mut client, 100);

    client.receive(incremental(3, 101), 10).unwrap();

    assert_eq!(
        client.receive(incremental(4, 101), 20),
        Ok(ReceiveOutcome::RecoveryRequired(
            RecoveryReason::PublicationRegression {
                previous: 101,
                received: 101,
            },
        ))
    );

    assert_eq!(client.handler().applied, vec![101]);
}

#[test]
fn server_resync_notification_invalidates_baseline() {
    let mut client = client();

    activate(&mut client, 100);

    let reason = RecoveryReason::ServerRequested {
        publisher_watermark: 200,
    };

    let frame = InboundFrame::new(
        3,
        InboundMessage::Gateway(ServerMessage::ResyncRequired {
            publisher_watermark: 200,
        }),
    );

    assert_eq!(
        client.receive(frame, 10),
        Ok(ReceiveOutcome::RecoveryRequired(reason))
    );

    assert!(!client.handler().valid);
    assert!(!client.reconnect_required());
    assert_eq!(client.server_watermark(), Some(200));
}

#[test]
fn recovery_request_remains_pending_until_sent() {
    let mut client = client();

    activate(&mut client, 100);
    client.require_recovery().unwrap();

    assert_eq!(
        client.pending_recovery_request(),
        Some(ClientRequest::RequestResync)
    );

    assert_eq!(
        client.pending_recovery_request(),
        Some(ClientRequest::RequestResync)
    );

    client.mark_recovery_request_sent(10).unwrap();

    assert_eq!(client.pending_recovery_request(), None);
}

#[test]
fn transport_gap_does_not_request_same_connection_recovery() {
    let mut client = client();

    activate(&mut client, 100);

    client.receive(incremental(4, 101), 10).unwrap();

    assert!(client.reconnect_required());
    assert_eq!(client.pending_recovery_request(), None);
}

#[test]
fn heartbeat_watermark_does_not_advance_applied_state() {
    let mut client = client();

    activate(&mut client, 100);

    assert_eq!(
        client.receive(heartbeat(3, 500), 10),
        Ok(ReceiveOutcome::HeartbeatReceived)
    );

    assert_eq!(client.server_watermark(), Some(500));
    assert_eq!(client.publisher_watermark(), Some(100));

    // A previously queued incremental may legitimately
    // have a lower sequence than the server's watermark.
    assert_eq!(
        client.receive(incremental(4, 101), 20),
        Ok(ReceiveOutcome::IncrementalApplied)
    );

    assert_eq!(client.publisher_watermark(), Some(101));
    assert_eq!(client.server_watermark(), Some(500));
}

#[test]
fn heartbeat_extends_connection_lifetime() {
    let mut client = client();

    activate(&mut client, 100);

    client.receive(heartbeat(3, 100), 100).unwrap();

    assert!(!client.tick(249).unwrap());
    assert!(client.tick(250).unwrap());
}

#[test]
fn duplicate_frames_do_not_extend_connection_lifetime() {
    let mut client = client();

    activate(&mut client, 100);

    let frame = heartbeat(3, 100);

    client.receive(frame, 10).unwrap();

    assert_eq!(
        client.receive(frame, 100),
        Ok(ReceiveOutcome::DuplicateIgnored)
    );

    assert_eq!(client.last_rx_ns(), 10);
    assert!(client.tick(160).unwrap());
}

#[test]
fn timeout_invalidates_application_state() {
    let mut client = client();

    activate(&mut client, 100);

    assert!(client.tick(150).unwrap());

    assert_eq!(client.state(), ClientState::TimedOut);
    assert!(!client.handler().valid);
    assert!(client.reconnect_required());

    assert_eq!(
        client.recovery_reason(),
        Some(RecoveryReason::ServerTimeout)
    );
}

#[test]
fn late_heartbeat_cannot_revive_timed_out_connection() {
    let mut client = client();

    activate(&mut client, 100);

    assert!(client.receive(heartbeat(3, 100), 150).is_err());

    assert_eq!(client.state(), ClientState::TimedOut);
    assert_eq!(client.stats().connection_timeouts, 1);
}

#[test]
fn heartbeat_send_is_explicitly_acknowledged() {
    let mut client = client();

    client.tick(50).unwrap();

    assert_eq!(client.pending_heartbeat(), Some(ClientRequest::Heartbeat));

    assert_eq!(client.stats().heartbeats_sent, 0);

    client.mark_heartbeat_sent(50).unwrap();

    assert_eq!(client.pending_heartbeat(), None);
    assert_eq!(client.stats().heartbeats_sent, 1);
}

#[test]
fn invalid_transport_sequence_requires_reconnection() {
    let mut client = client();

    assert_eq!(
        client.receive(heartbeat(0, 0), 10),
        Err(ClientError::InvalidTransportSequence { received: 0 })
    );

    assert!(client.reconnect_required());
    assert_eq!(client.stats().frames_rejected, 1);
}
