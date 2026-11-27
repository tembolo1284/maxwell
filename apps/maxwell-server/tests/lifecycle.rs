//! Integration tests for server lifecycle and maintenance.

use std::time::Duration;

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, Instrument, InstrumentConfig, InstrumentId,
    MarketEvent, MonotonicTimestamp, OrderAdded, OrderId, Price, Quantity, SequenceNumber,
    SessionId as FeedSessionId, Side, StreamId,
};
use maxwell_gateway::{ClientRequest, ServerMessage, SessionId, SessionState};
use maxwell_publisher::{BackpressurePolicy, SubscriptionFilter};

use maxwell_server::config::ServerConfig;
use maxwell_server::downstream::{ConnectionQueue, DownstreamMessage};
use maxwell_server::error::ServerError;
use maxwell_server::pipeline::PublicationOutcome;
use maxwell_server::recovery::RecoveryOutcome;
use maxwell_server::server::Server;
use maxwell_server::state::ServerState;

// ============================================================
// Fixtures
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

fn instrument() -> Instrument {
    Instrument::new(InstrumentConfig {
        id: InstrumentId::new(1),
        price_scale: 4,
        quantity_scale: 0,
        tick_size: Price::new(5),
        lot_size: Quantity::new(10),
        min_price: Price::new(0),
        max_price: Price::new(100_000),
        min_quantity: Quantity::new(10),
        max_quantity: Quantity::new(1_000_000),
    })
    .unwrap()
}

fn add(sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(), SequenceNumber::new(sequence)),
            InstrumentId::new(1),
            None,
        ),
        MarketEvent::OrderAdded(OrderAdded {
            order_id: OrderId::new(sequence),
            side: Side::Buy,
            price: Price::new(10_000),
            quantity: Quantity::new(100),
        }),
    )
}

fn new_server(config: ServerConfig) -> Server {
    let mut server = Server::new(config, feed()).unwrap();

    server.register_instrument(instrument()).unwrap();

    server
}

fn running_server(config: ServerConfig) -> Server {
    let mut server = new_server(config);

    server.start_empty(timestamp(0)).unwrap();

    server
}

fn activate(server: &mut Server) -> ConnectionQueue {
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

    let mut queue = ConnectionQueue::new(session(), 16).unwrap();

    server
        .synchronize_session(session(), &mut queue, timestamp(3))
        .unwrap();

    while queue.pop_front().is_some() {}

    queue
}

fn short_timeout_config() -> ServerConfig {
    let mut config = ServerConfig::default();

    config.gateway.heartbeat_interval_ns = 10;
    config.gateway.session_timeout_ns = 100;

    config
}

// ============================================================
// Initialization
// ============================================================

#[test]
fn initialized_server_rejects_upstream_events() {
    let mut server = new_server(ServerConfig::default());

    assert!(matches!(
        server.ingest_event(add(1), timestamp(1)),
        Err(ServerError::InvalidState(_)),
    ));

    assert_eq!(server.state(), ServerState::Initialized,);

    assert_eq!(server.status().next_expected, SequenceNumber::ONE,);
}

#[test]
fn instrument_registration_is_restricted_to_initialization() {
    let mut server = running_server(ServerConfig::default());

    assert!(matches!(
        server.register_instrument(instrument()),
        Err(ServerError::InvalidState(_)),
    ));

    assert_eq!(server.status().registered_instruments, 1,);
}

#[test]
fn initialized_server_can_stop_without_starting() {
    let mut server = new_server(ServerConfig::default());

    server.begin_shutdown(timestamp(1)).unwrap();

    assert_eq!(server.state(), ServerState::Stopped,);

    assert!(!server.is_publishable());
}

#[test]
fn stopped_server_cannot_restart() {
    let mut server = new_server(ServerConfig::default());

    server.begin_shutdown(timestamp(1)).unwrap();

    assert!(matches!(
        server.start_empty(timestamp(2)),
        Err(ServerError::InvalidState(_)),
    ));

    assert!(matches!(
        server.start_recovering(timestamp(2)),
        Err(ServerError::InvalidState(_)),
    ));

    assert_eq!(server.state(), ServerState::Stopped,);
}

// ============================================================
// Heartbeats
// ============================================================

#[test]
fn idle_session_receives_heartbeat() {
    let mut server = running_server(short_timeout_config());

    server.connect(session(), timestamp(1)).unwrap();

    let mut queue = ConnectionQueue::new(session(), 4).unwrap();

    let report = server
        .service_session(session(), &mut queue, timestamp(10))
        .unwrap();

    assert_eq!(report.total_sent(), 0);
    assert!(queue.is_empty());

    let report = server
        .service_session(session(), &mut queue, timestamp(11))
        .unwrap();

    assert_eq!(report.control_sent, 1);

    let frame = queue.pop_front().unwrap();

    assert_eq!(frame.transport_sequence, 1);

    assert_eq!(
        frame.message,
        DownstreamMessage::Gateway(ServerMessage::Heartbeat {
            timestamp_ns: 11,
            publisher_watermark: 0,
        },),
    );
}

#[test]
fn heartbeat_interval_restarts_after_successful_delivery() {
    let mut server = running_server(short_timeout_config());

    server.connect(session(), timestamp(1)).unwrap();

    let mut queue = ConnectionQueue::new(session(), 4).unwrap();

    server
        .service_session(session(), &mut queue, timestamp(11))
        .unwrap();

    assert_eq!(queue.len(), 1);

    server
        .service_session(session(), &mut queue, timestamp(20))
        .unwrap();

    assert_eq!(queue.len(), 1);

    server
        .service_session(session(), &mut queue, timestamp(21))
        .unwrap();

    assert_eq!(queue.len(), 2);

    assert_eq!(queue.pop_front().unwrap().transport_sequence, 1,);

    assert_eq!(queue.pop_front().unwrap().transport_sequence, 2,);
}

// ============================================================
// Session expiration
// ============================================================

#[test]
fn idle_session_expires_at_configured_deadline() {
    let mut server = running_server(short_timeout_config());

    server.connect(session(), timestamp(1)).unwrap();

    server.poll(timestamp(100)).unwrap();

    assert!(
        !server
            .pipeline()
            .gateway()
            .session(session())
            .unwrap()
            .is_expired()
    );

    server.poll(timestamp(101)).unwrap();

    assert_eq!(
        server
            .pipeline()
            .gateway()
            .session(session())
            .unwrap()
            .state(),
        SessionState::Expired,
    );
}

#[test]
fn client_heartbeat_extends_session_lifetime() {
    let mut server = running_server(short_timeout_config());

    server.connect(session(), timestamp(1)).unwrap();

    server
        .handle_request(session(), ClientRequest::Heartbeat, timestamp(90))
        .unwrap();

    server.poll(timestamp(101)).unwrap();

    assert!(
        !server
            .pipeline()
            .gateway()
            .session(session())
            .unwrap()
            .is_expired()
    );

    server.poll(timestamp(190)).unwrap();

    assert!(
        server
            .pipeline()
            .gateway()
            .session(session())
            .unwrap()
            .is_expired()
    );
}

#[test]
fn expired_subscriber_cannot_receive_new_incrementals() {
    let mut server = running_server(short_timeout_config());

    let mut queue = activate(&mut server);

    server.poll(timestamp(102)).unwrap();

    assert!(
        server
            .pipeline()
            .gateway()
            .session(session())
            .unwrap()
            .is_expired()
    );

    assert_eq!(
        server.ingest_event(add(1), timestamp(103)).unwrap(),
        RecoveryOutcome::Published { sequence: 1 },
    );

    let report = server
        .service_session(session(), &mut queue, timestamp(104))
        .unwrap();

    assert!(report.expired);
    assert_eq!(report.incremental_sent, 0);
    assert!(queue.is_empty());
}

#[test]
fn expired_session_can_be_explicitly_removed() {
    let mut server = running_server(short_timeout_config());

    server.connect(session(), timestamp(1)).unwrap();

    server.poll(timestamp(101)).unwrap();

    // Expiration and physical disconnection are separate
    // operations in the gateway.
    server.disconnect(session()).unwrap();

    assert_eq!(server.status().connected_sessions, 0,);

    assert!(server.pipeline().gateway().session(session()).is_none());
}

// ============================================================
// Graceful shutdown
// ============================================================

#[test]
fn draining_server_rejects_new_upstream_events() {
    let mut server = running_server(ServerConfig::default());

    server.begin_shutdown(timestamp(1)).unwrap();

    assert_eq!(server.state(), ServerState::Draining,);

    assert!(matches!(
        server.ingest_event(add(1), timestamp(2)),
        Err(ServerError::InvalidState(_)),
    ));

    assert_eq!(server.status().next_expected, SequenceNumber::ONE,);
}

#[test]
fn draining_server_rejects_new_subscriptions() {
    let mut server = running_server(ServerConfig::default());

    server.connect(session(), timestamp(1)).unwrap();

    server.begin_shutdown(timestamp(2)).unwrap();

    assert!(matches!(
        server.handle_request(
            session(),
            ClientRequest::Subscribe {
                filter: SubscriptionFilter::all(),
            },
            timestamp(3),
        ),
        Err(ServerError::InvalidState(_)),
    ));
}

#[test]
fn draining_server_can_service_existing_subscriber() {
    let mut server = running_server(ServerConfig::default());

    let mut queue = activate(&mut server);

    server.ingest_event(add(1), timestamp(4)).unwrap();

    server.begin_shutdown(timestamp(5)).unwrap();

    let report = server
        .service_session(session(), &mut queue, timestamp(6))
        .unwrap();

    assert_eq!(report.incremental_sent, 1);

    let frame = queue.pop_front().unwrap();

    assert!(matches!(
        frame.message,
        DownstreamMessage::Gateway(ServerMessage::Incremental(_)),
    ));

    server.disconnect(session()).unwrap();

    server.finish_shutdown(timestamp(7)).unwrap();

    assert_eq!(server.state(), ServerState::Stopped,);
}

#[test]
fn shutdown_drains_pending_publication_without_reapplying_event() {
    let mut config = ServerConfig::default();

    config.gateway.publisher.queue_capacity = 1;

    config.gateway.publisher.backpressure_policy = BackpressurePolicy::RejectPublication;

    let mut server = running_server(config);

    let mut queue = activate(&mut server);

    server.ingest_event(add(1), timestamp(4)).unwrap();

    assert_eq!(
        server.ingest_event(add(2), timestamp(5)).unwrap(),
        RecoveryOutcome::PublicationPending,
    );

    server.begin_shutdown(timestamp(6)).unwrap();

    assert!(matches!(
        server.finish_shutdown(timestamp(7)),
        Err(ServerError::InvalidState(_)),
    ));

    server
        .service_session(session(), &mut queue, timestamp(8))
        .unwrap();

    assert_eq!(
        server.retry_publication().unwrap(),
        PublicationOutcome::Published { sequence: 2 },
    );

    server
        .service_session(session(), &mut queue, timestamp(9))
        .unwrap();

    assert_eq!(queue.len(), 2);

    while queue.pop_front().is_some() {}

    let book = server
        .pipeline()
        .books()
        .book(InstrumentId::new(1))
        .unwrap();

    assert_eq!(book.order_count(), 2);

    assert_eq!(server.status().next_expected, SequenceNumber::new(3),);

    assert_eq!(server.status().publication_watermark, 2,);

    assert!(!server.status().publication_pending);

    server.disconnect(session()).unwrap();

    server.finish_shutdown(timestamp(10)).unwrap();

    assert_eq!(server.state(), ServerState::Stopped,);
}

#[test]
fn repeated_shutdown_requests_are_idempotent() {
    let mut server = running_server(ServerConfig::default());

    server.begin_shutdown(timestamp(1)).unwrap();
    server.begin_shutdown(timestamp(2)).unwrap();

    assert_eq!(server.state(), ServerState::Draining,);

    server.finish_shutdown(timestamp(3)).unwrap();
    server.finish_shutdown(timestamp(4)).unwrap();

    assert_eq!(server.state(), ServerState::Stopped,);
}

// ============================================================
// Shutdown deadlines
// ============================================================

#[test]
fn shutdown_timeout_transitions_server_to_failed() {
    let mut config = ServerConfig::default();

    config.shutdown.timeout = Duration::from_nanos(100);

    let mut server = running_server(config);

    server.connect(session(), timestamp(1)).unwrap();

    server.begin_shutdown(timestamp(10)).unwrap();

    server.poll(timestamp(109)).unwrap();

    assert_eq!(server.state(), ServerState::Draining,);

    assert!(matches!(
        server.poll(timestamp(110)),
        Err(ServerError::ShutdownTimeout),
    ));

    assert_eq!(server.state(), ServerState::Failed,);

    assert!(!server.is_publishable());
}

#[test]
fn repeated_shutdown_request_does_not_extend_deadline() {
    let mut config = ServerConfig::default();

    config.shutdown.timeout = Duration::from_nanos(100);

    let mut server = running_server(config);

    server.begin_shutdown(timestamp(10)).unwrap();

    server.begin_shutdown(timestamp(50)).unwrap();

    assert!(matches!(
        server.poll(timestamp(110)),
        Err(ServerError::ShutdownTimeout),
    ));

    assert_eq!(server.state(), ServerState::Failed,);
}

#[test]
fn shutdown_can_complete_before_deadline() {
    let mut config = ServerConfig::default();

    config.shutdown.timeout = Duration::from_nanos(100);

    let mut server = running_server(config);

    server.begin_shutdown(timestamp(10)).unwrap();

    server.finish_shutdown(timestamp(109)).unwrap();

    assert_eq!(server.state(), ServerState::Stopped,);
}

// ============================================================
// Terminal states
// ============================================================

#[test]
fn failed_server_rejects_further_upstream_processing() {
    let mut server = running_server(ServerConfig::default());

    server.ingest_event(add(1), timestamp(1)).unwrap();

    let mut duplicate_order = add(1);

    duplicate_order.header.id.sequence = SequenceNumber::new(2);

    assert!(server.ingest_event(duplicate_order, timestamp(2)).is_err());

    assert_eq!(server.state(), ServerState::Failed,);

    assert!(matches!(
        server.ingest_event(add(3), timestamp(3)),
        Err(ServerError::InvalidState(_)),
    ));

    assert!(matches!(
        server.retry_publication(),
        Err(ServerError::InvalidState(_)),
    ));

    assert!(matches!(
        server.start_empty(timestamp(4)),
        Err(ServerError::InvalidState(_)),
    ));
}

#[test]
fn stopped_server_rejects_periodic_service() {
    let mut server = running_server(ServerConfig::default());

    server.begin_shutdown(timestamp(1)).unwrap();
    server.finish_shutdown(timestamp(2)).unwrap();

    assert!(matches!(
        server.poll(timestamp(3)),
        Err(ServerError::InvalidState(_)),
    ));

    assert_eq!(server.state(), ServerState::Stopped,);
}
