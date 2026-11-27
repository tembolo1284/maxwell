//! End-to-end server pipeline integration tests.
//!
//! These tests exercise the public server API using
//! deterministic timestamps and in-memory downstream
//! transport queues.

use maxwell_core::{
    FeedId, MonotonicTimestamp, SequenceNumber, SessionId as FeedSessionId, StreamId,
};
use maxwell_gateway::{ClientRequest, SessionId, SessionState};
use maxwell_publisher::SubscriptionFilter;

use maxwell_server::config::ServerConfig;
use maxwell_server::downstream::{ConnectionQueue, DeliveryError, DownstreamMessage};
use maxwell_server::error::ServerError;
use maxwell_server::pipeline::PublicationOutcome;
use maxwell_server::server::Server;
use maxwell_server::snapshot::SnapshotOutcome;
use maxwell_server::state::ServerState;

// ============================================================
// Test fixtures
// ============================================================

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), FeedSessionId::new(1))
}

fn session() -> SessionId {
    SessionId::new(1)
}

fn timestamp(ns: u64) -> MonotonicTimestamp {
    MonotonicTimestamp::new(ns)
}

fn new_server() -> Server {
    Server::new(ServerConfig::default(), feed()).unwrap()
}

fn running_server() -> Server {
    let mut server = new_server();

    server.start_empty(timestamp(0)).unwrap();

    server
}

fn subscribed_server() -> Server {
    let mut server = running_server();

    server.connect(session(), timestamp(1)).unwrap();

    server
        .handle_request(
            session(),
            ClientRequest::Subscribe {
                filter: SubscriptionFilter::all(),
            },
            timestamp(2),
        )
        .unwrap();

    server
}

// ============================================================
// Initialization
// ============================================================

#[test]
fn server_starts_uninitialized() {
    let server = new_server();

    assert_eq!(server.state(), ServerState::Initialized,);

    assert!(!server.is_synchronized());
    assert!(!server.is_publishable());
    assert!(!server.status().publication_pending);
}

#[test]
fn authoritative_empty_baseline_is_publishable() {
    let server = running_server();

    let status = server.status();

    assert_eq!(status.lifecycle, ServerState::Running,);

    assert!(status.synchronized);
    assert!(server.is_publishable());

    assert_eq!(status.next_expected, SequenceNumber::ONE,);

    assert_eq!(status.publication_watermark, 0,);

    assert_eq!(status.connected_sessions, 0);
    assert_eq!(status.registered_instruments, 0);
}

#[test]
fn recovery_startup_is_not_publishable() {
    let mut server = new_server();

    server.start_recovering(timestamp(0)).unwrap();

    assert_eq!(server.state(), ServerState::Running,);

    assert!(!server.is_synchronized());
    assert!(!server.is_publishable());
}

#[test]
fn repeated_startup_is_rejected() {
    let mut server = running_server();

    assert!(server.start_empty(timestamp(1)).is_err());

    assert_eq!(server.state(), ServerState::Running,);
}

// ============================================================
// Session management
// ============================================================

#[test]
fn initialized_server_rejects_connections() {
    let mut server = new_server();

    assert!(matches!(
        server.connect(session(), timestamp(0)),
        Err(ServerError::InvalidState(_)),
    ));
}

#[test]
fn downstream_connection_is_registered() {
    let mut server = running_server();

    server.connect(session(), timestamp(1)).unwrap();

    assert_eq!(server.status().connected_sessions, 1,);

    assert!(server.pipeline().gateway().session(session()).is_some());
}

#[test]
fn downstream_disconnect_removes_session() {
    let mut server = subscribed_server();

    server.disconnect(session()).unwrap();

    assert_eq!(server.status().connected_sessions, 0,);

    assert!(server.pipeline().gateway().session(session()).is_none());
}

#[test]
fn subscription_requires_snapshot_activation() {
    let server = subscribed_server();

    let downstream = server.pipeline().gateway().session(session()).unwrap();

    assert_eq!(downstream.state(), SessionState::AwaitingSnapshot,);

    assert!(!downstream.is_active());
}

// ============================================================
// Snapshot synchronization
// ============================================================

#[test]
fn empty_market_snapshot_activates_subscriber() {
    let mut server = subscribed_server();

    let mut queue = ConnectionQueue::new(session(), 8).unwrap();

    let outcome = server
        .synchronize_session(session(), &mut queue, timestamp(3))
        .unwrap();

    assert_eq!(
        outcome,
        SnapshotOutcome::Activated {
            publisher_watermark: 0,
            entry_count: 0,
        },
    );

    assert!(
        server
            .pipeline()
            .gateway()
            .session(session())
            .unwrap()
            .is_active()
    );

    assert_eq!(queue.len(), 2);

    let begin = queue.pop_front().unwrap();

    assert_eq!(begin.transport_sequence, 1);

    assert_eq!(
        begin.message,
        DownstreamMessage::SnapshotBegin {
            publisher_watermark: 0,
            entry_count: 0,
        },
    );

    let end = queue.pop_front().unwrap();

    assert_eq!(end.transport_sequence, 2);

    assert_eq!(
        end.message,
        DownstreamMessage::SnapshotEnd {
            publisher_watermark: 0,
        },
    );

    assert_eq!(queue.next_sequence(), Some(3));
}

#[test]
fn rejected_snapshot_does_not_activate_subscriber() {
    let mut server = subscribed_server();

    let mut queue = ConnectionQueue::new(session(), 1).unwrap();

    let outcome = server
        .synchronize_session(session(), &mut queue, timestamp(3))
        .unwrap();

    assert_eq!(
        outcome,
        SnapshotOutcome::Rejected(DeliveryError::Backpressure,),
    );

    assert!(queue.is_empty());
    assert_eq!(queue.next_sequence(), Some(1));

    assert_eq!(
        server
            .pipeline()
            .gateway()
            .session(session())
            .unwrap()
            .state(),
        SessionState::AwaitingSnapshot,
    );
}

#[test]
fn unsubscribed_session_cannot_capture_snapshot() {
    let mut server = running_server();

    server.connect(session(), timestamp(1)).unwrap();

    let mut queue = ConnectionQueue::new(session(), 8).unwrap();

    assert!(
        server
            .synchronize_session(session(), &mut queue, timestamp(2),)
            .is_err()
    );

    assert!(queue.is_empty());
}

#[test]
fn unsynchronized_market_cannot_activate_subscriber() {
    let mut server = new_server();

    server.start_recovering(timestamp(0)).unwrap();

    server.connect(session(), timestamp(1)).unwrap();

    server
        .handle_request(
            session(),
            ClientRequest::Subscribe {
                filter: SubscriptionFilter::all(),
            },
            timestamp(2),
        )
        .unwrap();

    let mut queue = ConnectionQueue::new(session(), 8).unwrap();

    assert!(matches!(
        server.synchronize_session(session(), &mut queue, timestamp(3),),
        Err(ServerError::SnapshotUnavailable),
    ));

    assert!(queue.is_empty());
}

// ============================================================
// Publication state
// ============================================================

#[test]
fn retry_without_pending_publication_is_idle() {
    let mut server = running_server();

    assert_eq!(
        server.retry_publication().unwrap(),
        PublicationOutcome::Idle,
    );

    assert_eq!(server.status().publication_watermark, 0,);

    assert_eq!(server.status().next_expected, SequenceNumber::ONE,);
}

// ============================================================
// Clock behavior
// ============================================================

#[test]
fn monotonic_timestamp_regression_is_rejected() {
    let mut server = running_server();

    server.poll(timestamp(100)).unwrap();

    assert!(matches!(
        server.poll(timestamp(99)),
        Err(ServerError::InvalidState(_)),
    ));

    assert_eq!(server.state(), ServerState::Running,);
}

#[test]
fn equal_timestamps_are_permitted() {
    let mut server = running_server();

    server.poll(timestamp(100)).unwrap();
    server.poll(timestamp(100)).unwrap();

    assert_eq!(server.state(), ServerState::Running,);
}

// ============================================================
// Metrics
// ============================================================

#[test]
fn server_exports_operational_metrics() {
    let mut server = running_server();

    server.poll(timestamp(1)).unwrap();

    let mut output = Vec::new();

    server.metrics().write_prometheus(&mut output).unwrap();

    let output = String::from_utf8(output).unwrap();

    assert!(output.contains("maxwell_service_iterations_total 1"));

    assert!(output.contains("maxwell_synchronized 1"));

    assert!(output.contains("maxwell_server_lifecycle 1"));
}

// ============================================================
// Basic shutdown
// ============================================================

#[test]
fn empty_server_can_shutdown_cleanly() {
    let mut server = running_server();

    server.begin_shutdown(timestamp(1)).unwrap();

    assert_eq!(server.state(), ServerState::Draining,);

    assert!(!server.is_publishable());

    server.finish_shutdown(timestamp(2)).unwrap();

    assert_eq!(server.state(), ServerState::Stopped,);
}

#[test]
fn draining_server_rejects_new_connections() {
    let mut server = running_server();

    server.begin_shutdown(timestamp(1)).unwrap();

    assert!(server.connect(session(), timestamp(2)).is_err());
}

#[test]
fn shutdown_requires_downstream_disconnection() {
    let mut server = subscribed_server();

    server.begin_shutdown(timestamp(3)).unwrap();

    assert!(server.finish_shutdown(timestamp(4)).is_err());

    server.disconnect(session()).unwrap();

    server.finish_shutdown(timestamp(5)).unwrap();

    assert_eq!(server.state(), ServerState::Stopped,);
}
