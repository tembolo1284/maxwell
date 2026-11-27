//! Integration tests for publication and transport backpressure.

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

// ============================================================
// Fixtures
// ============================================================

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), FeedSessionId::new(1))
}

fn session() -> SessionId {
    SessionId::new(1)
}

fn instrument_id() -> InstrumentId {
    InstrumentId::new(1)
}

fn timestamp(ns: u64) -> MonotonicTimestamp {
    MonotonicTimestamp::new(ns)
}

fn instrument() -> Instrument {
    Instrument::new(InstrumentConfig {
        id: instrument_id(),
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
            instrument_id(),
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

fn server_with_policy(policy: BackpressurePolicy) -> Server {
    let mut config = ServerConfig::default();

    config.gateway.publisher.queue_capacity = 1;
    config.gateway.publisher.backpressure_policy = policy;

    let mut server = Server::new(config, feed()).unwrap();

    server.register_instrument(instrument()).unwrap();
    server.start_empty(timestamp(0)).unwrap();

    server
}

fn activate(server: &mut Server, id: SessionId, now: u64) -> ConnectionQueue {
    server.connect(id, timestamp(now)).unwrap();

    server
        .handle_request(
            id,
            ClientRequest::Subscribe {
                filter: SubscriptionFilter::all(),
            },
            timestamp(now + 1),
        )
        .unwrap();

    let mut queue = ConnectionQueue::new(id, 16).unwrap();

    server
        .synchronize_session(id, &mut queue, timestamp(now + 2))
        .unwrap();

    while queue.pop_front().is_some() {}

    queue
}

fn order_count(server: &Server) -> usize {
    server
        .pipeline()
        .books()
        .book(instrument_id())
        .unwrap()
        .order_count()
}

// ============================================================
// RejectPublication policy
// ============================================================

#[test]
fn rejected_publication_retains_applied_book_mutation() {
    let mut server = server_with_policy(BackpressurePolicy::RejectPublication);

    let _queue = activate(&mut server, session(), 1);

    assert_eq!(
        server.ingest_event(add(1), timestamp(4)).unwrap(),
        RecoveryOutcome::Published { sequence: 1 },
    );

    assert_eq!(
        server.ingest_event(add(2), timestamp(5)).unwrap(),
        RecoveryOutcome::PublicationPending,
    );

    assert_eq!(order_count(&server), 2);

    assert_eq!(server.status().next_expected, SequenceNumber::new(3),);

    assert_eq!(server.status().publication_watermark, 1,);

    assert!(server.status().publication_pending);
    assert!(server.is_synchronized());
    assert!(!server.is_publishable());
}

#[test]
fn pending_publication_blocks_new_upstream_input() {
    let mut server = server_with_policy(BackpressurePolicy::RejectPublication);

    let _queue = activate(&mut server, session(), 1);

    server.ingest_event(add(1), timestamp(4)).unwrap();
    server.ingest_event(add(2), timestamp(5)).unwrap();

    assert_eq!(
        server.ingest_event(add(3), timestamp(6)).unwrap(),
        RecoveryOutcome::InputBlocked,
    );

    assert_eq!(order_count(&server), 2);

    assert_eq!(server.status().next_expected, SequenceNumber::new(3),);

    assert_eq!(server.status().publication_watermark, 1,);
}

#[test]
fn repeated_publication_rejection_does_not_mutate_book() {
    let mut server = server_with_policy(BackpressurePolicy::RejectPublication);

    let _queue = activate(&mut server, session(), 1);

    server.ingest_event(add(1), timestamp(4)).unwrap();
    server.ingest_event(add(2), timestamp(5)).unwrap();

    let before = server
        .pipeline()
        .books()
        .capture_snapshot(instrument_id())
        .unwrap();

    for _ in 0..10 {
        assert_eq!(
            server.retry_publication().unwrap(),
            PublicationOutcome::Blocked,
        );
    }

    let after = server
        .pipeline()
        .books()
        .capture_snapshot(instrument_id())
        .unwrap();

    assert_eq!(before, after);

    assert_eq!(server.status().next_expected, SequenceNumber::new(3),);

    assert_eq!(server.status().publication_watermark, 1,);
}

#[test]
fn draining_subscriber_allows_publication_retry() {
    let mut server = server_with_policy(BackpressurePolicy::RejectPublication);

    let mut queue = activate(&mut server, session(), 1);

    server.ingest_event(add(1), timestamp(4)).unwrap();
    server.ingest_event(add(2), timestamp(5)).unwrap();

    let report = server
        .service_session(session(), &mut queue, timestamp(6))
        .unwrap();

    assert_eq!(report.incremental_sent, 1);

    assert_eq!(
        server.retry_publication().unwrap(),
        PublicationOutcome::Published { sequence: 2 },
    );

    assert_eq!(order_count(&server), 2);
    assert!(!server.status().publication_pending);
    assert!(server.is_publishable());

    assert_eq!(server.status().next_expected, SequenceNumber::new(3),);

    assert_eq!(server.status().publication_watermark, 2,);

    server
        .service_session(session(), &mut queue, timestamp(7))
        .unwrap();

    assert_eq!(queue.len(), 2);

    let first = queue.pop_front().unwrap();
    let second = queue.pop_front().unwrap();

    assert!(matches!(
        first.message,
        DownstreamMessage::Gateway(ServerMessage::Incremental(_)),
    ));

    assert!(matches!(
        second.message,
        DownstreamMessage::Gateway(ServerMessage::Incremental(_)),
    ));

    assert_eq!(second.transport_sequence, first.transport_sequence + 1,);

    assert_eq!(
        server.retry_publication().unwrap(),
        PublicationOutcome::Idle,
    );
}

#[test]
fn blocked_input_can_be_resubmitted_after_retry() {
    let mut server = server_with_policy(BackpressurePolicy::RejectPublication);

    let mut queue = activate(&mut server, session(), 1);

    server.ingest_event(add(1), timestamp(4)).unwrap();
    server.ingest_event(add(2), timestamp(5)).unwrap();

    let third = add(3);

    assert_eq!(
        server.ingest_event(third, timestamp(6)).unwrap(),
        RecoveryOutcome::InputBlocked,
    );

    server
        .service_session(session(), &mut queue, timestamp(7))
        .unwrap();

    server.retry_publication().unwrap();

    server
        .service_session(session(), &mut queue, timestamp(8))
        .unwrap();

    assert_eq!(
        server.ingest_event(third, timestamp(9)).unwrap(),
        RecoveryOutcome::Published { sequence: 3 },
    );

    assert_eq!(order_count(&server), 3);

    assert_eq!(server.status().next_expected, SequenceNumber::new(4),);
}

#[test]
fn pending_publication_prevents_snapshot_capture() {
    let mut server = server_with_policy(BackpressurePolicy::RejectPublication);

    let _first = activate(&mut server, session(), 1);

    server.ingest_event(add(1), timestamp(4)).unwrap();
    server.ingest_event(add(2), timestamp(5)).unwrap();

    let second = SessionId::new(2);

    server.connect(second, timestamp(6)).unwrap();

    server
        .handle_request(
            second,
            ClientRequest::Subscribe {
                filter: SubscriptionFilter::all(),
            },
            timestamp(7),
        )
        .unwrap();

    let mut queue = ConnectionQueue::new(second, 16).unwrap();

    assert!(matches!(
        server.synchronize_session(second, &mut queue, timestamp(8),),
        Err(ServerError::SnapshotUnavailable),
    ));

    assert!(queue.is_empty());
}

// ============================================================
// RequireResync policy
// ============================================================

#[test]
fn slow_subscriber_is_invalidated_without_blocking_market() {
    let mut server = server_with_policy(BackpressurePolicy::RequireResync);

    let mut queue = activate(&mut server, session(), 1);

    server.ingest_event(add(1), timestamp(4)).unwrap();

    assert_eq!(
        server.ingest_event(add(2), timestamp(5)).unwrap(),
        RecoveryOutcome::Published { sequence: 2 },
    );

    assert_eq!(order_count(&server), 2);
    assert!(server.is_publishable());
    assert!(!server.status().publication_pending);

    assert_eq!(
        server
            .pipeline()
            .gateway()
            .session(session())
            .unwrap()
            .state(),
        SessionState::ResyncRequired,
    );

    server
        .service_session(session(), &mut queue, timestamp(6))
        .unwrap();

    assert!(matches!(
        queue.pop_front().unwrap().message,
        DownstreamMessage::Gateway(ServerMessage::ResyncRequired { .. }),
    ));
}

#[test]
fn slow_subscriber_does_not_invalidate_healthy_subscriber() {
    let mut server = server_with_policy(BackpressurePolicy::RequireResync);

    let slow = SessionId::new(1);
    let healthy = SessionId::new(2);

    let _slow_queue = activate(&mut server, slow, 1);
    let mut healthy_queue = activate(&mut server, healthy, 4);

    server.ingest_event(add(1), timestamp(7)).unwrap();

    server
        .service_session(healthy, &mut healthy_queue, timestamp(8))
        .unwrap();

    server.ingest_event(add(2), timestamp(9)).unwrap();

    let gateway = server.pipeline().gateway();

    assert_eq!(
        gateway.session(slow).unwrap().state(),
        SessionState::ResyncRequired,
    );

    assert!(gateway.session(healthy).unwrap().is_active());

    server
        .service_session(healthy, &mut healthy_queue, timestamp(10))
        .unwrap();

    assert_eq!(healthy_queue.len(), 2);
    assert_eq!(order_count(&server), 2);
    assert!(server.is_publishable());
}

// ============================================================
// Transport-level backpressure
// ============================================================

#[test]
fn rejected_transport_delivery_preserves_publication() {
    let mut server = server_with_policy(BackpressurePolicy::RejectPublication);

    let mut queue = activate(&mut server, session(), 1);

    // Fill the transport queue independently of the
    // publisher queue.
    while queue.remaining_capacity() != 0 {
        queue
            .try_accept(
                session(),
                DownstreamMessage::Gateway(ServerMessage::Heartbeat {
                    timestamp_ns: 3,
                    publisher_watermark: 0,
                }),
            )
            .unwrap();
    }

    server.ingest_event(add(1), timestamp(4)).unwrap();

    let before = queue.next_sequence();

    let report = server
        .service_session(session(), &mut queue, timestamp(5))
        .unwrap();

    assert!(report.is_blocked());
    assert_eq!(report.incremental_sent, 0);
    assert_eq!(queue.next_sequence(), before);

    while queue.pop_front().is_some() {}

    let report = server
        .service_session(session(), &mut queue, timestamp(6))
        .unwrap();

    assert_eq!(report.incremental_sent, 1);
    assert!(!report.is_blocked());

    let frame = queue.pop_front().unwrap();

    assert_eq!(Some(frame.transport_sequence), before);

    assert!(matches!(
        frame.message,
        DownstreamMessage::Gateway(ServerMessage::Incremental(_)),
    ));

    assert_eq!(order_count(&server), 1);
}
