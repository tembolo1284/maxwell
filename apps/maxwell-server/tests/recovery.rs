//! Integration tests for upstream sequencing and recovery.

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, Instrument, InstrumentConfig, InstrumentId,
    MarketEvent, MonotonicTimestamp, OrderAdded, OrderId, Price, Quantity, SequenceNumber,
    SessionId as FeedSessionId, Side, StreamId,
};
use maxwell_gateway::{ClientRequest, ServerMessage, SessionId, SessionState};
use maxwell_publisher::SubscriptionFilter;
use maxwell_recovery::RecoveryState;

use maxwell_server::config::ServerConfig;
use maxwell_server::downstream::{ConnectionQueue, DownstreamMessage};
use maxwell_server::error::ServerError;
use maxwell_server::recovery::RecoveryOutcome;
use maxwell_server::server::Server;
use maxwell_server::snapshot::SnapshotOutcome;
use maxwell_server::state::ServerState;

// ============================================================
// Fixtures
// ============================================================

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), FeedSessionId::new(1))
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

fn add(sequence: u64, order_id: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(), SequenceNumber::new(sequence)),
            instrument_id(),
            None,
        ),
        MarketEvent::OrderAdded(OrderAdded {
            order_id: OrderId::new(order_id),
            side: Side::Buy,
            price: Price::new(10_000),
            quantity: Quantity::new(100),
        }),
    )
}

fn running_server() -> Server {
    let mut server = Server::new(ServerConfig::default(), feed()).unwrap();

    server.register_instrument(instrument()).unwrap();
    server.start_empty(timestamp(0)).unwrap();

    server
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
// Contiguous processing
// ============================================================

#[test]
fn contiguous_events_advance_both_watermarks() {
    let mut server = running_server();

    for sequence in 1..=3 {
        let outcome = server
            .ingest_event(add(sequence, sequence), timestamp(sequence))
            .unwrap();

        assert_eq!(outcome, RecoveryOutcome::Published { sequence },);
    }

    assert_eq!(order_count(&server), 3);

    assert_eq!(server.status().next_expected, SequenceNumber::new(4),);

    assert_eq!(server.status().publication_watermark, 3,);

    assert!(server.is_publishable());
}

#[test]
fn duplicate_event_does_not_mutate_or_publish_twice() {
    let mut server = running_server();

    let event = add(1, 100);

    server.ingest_event(event, timestamp(1)).unwrap();

    let outcome = server.ingest_event(event, timestamp(2)).unwrap();

    assert_eq!(outcome, RecoveryOutcome::Duplicate);
    assert_eq!(order_count(&server), 1);

    assert_eq!(server.status().next_expected, SequenceNumber::new(2),);

    assert_eq!(server.status().publication_watermark, 1,);
}

// ============================================================
// Gap detection
// ============================================================

#[test]
fn future_event_is_buffered_without_book_mutation() {
    let mut server = running_server();

    let outcome = server.ingest_event(add(3, 103), timestamp(1)).unwrap();

    assert_eq!(outcome, RecoveryOutcome::Buffered);
    assert_eq!(order_count(&server), 0);

    assert!(!server.is_synchronized());
    assert!(!server.is_publishable());

    assert_eq!(server.status().next_expected, SequenceNumber::ONE,);

    assert_eq!(server.status().publication_watermark, 0,);

    assert!(server.recovery().pending_request().is_some());
}

#[test]
fn missing_event_replays_buffered_events_in_order() {
    let mut server = running_server();

    server.ingest_event(add(1, 101), timestamp(1)).unwrap();

    server.ingest_event(add(3, 103), timestamp(2)).unwrap();

    assert_eq!(order_count(&server), 1);
    assert!(!server.is_publishable());

    let outcome = server.ingest_event(add(2, 102), timestamp(3)).unwrap();

    assert_eq!(outcome, RecoveryOutcome::Resynchronized,);

    assert_eq!(order_count(&server), 3);

    assert_eq!(server.recovery().state(), RecoveryState::Live,);

    assert_eq!(server.status().next_expected, SequenceNumber::new(4),);

    // Only the original live event was published.
    assert_eq!(server.status().publication_watermark, 1,);

    assert!(server.is_publishable());

    let book = server.pipeline().books().book(instrument_id()).unwrap();

    assert_eq!(book.last_sequence(), Some(SequenceNumber::new(3)),);

    book.validate_consistency().unwrap();
}

#[test]
fn recovered_book_matches_contiguous_book() {
    let mut contiguous = running_server();
    let mut recovered = running_server();

    for sequence in 1..=3 {
        contiguous
            .ingest_event(add(sequence, sequence), timestamp(sequence))
            .unwrap();
    }

    for (index, sequence) in [1, 3, 2].into_iter().enumerate() {
        recovered
            .ingest_event(add(sequence, sequence), timestamp(index as u64 + 1))
            .unwrap();
    }

    let expected = contiguous
        .pipeline()
        .books()
        .capture_snapshot(instrument_id())
        .unwrap();

    let actual = recovered
        .pipeline()
        .books()
        .capture_snapshot(instrument_id())
        .unwrap();

    assert_eq!(actual, expected);

    assert_eq!(
        recovered.status().next_expected,
        contiguous.status().next_expected,
    );
}

// ============================================================
// Downstream recovery behavior
// ============================================================

#[test]
fn gap_invalidates_active_subscriber() {
    let mut server = running_server();
    let session = SessionId::new(1);

    server.connect(session, timestamp(1)).unwrap();

    server
        .handle_request(
            session,
            ClientRequest::Subscribe {
                filter: SubscriptionFilter::all(),
            },
            timestamp(2),
        )
        .unwrap();

    let mut queue = ConnectionQueue::new(session, 16).unwrap();

    server
        .synchronize_session(session, &mut queue, timestamp(3))
        .unwrap();

    while queue.pop_front().is_some() {}

    server.ingest_event(add(2, 102), timestamp(4)).unwrap();

    assert_eq!(
        server
            .pipeline()
            .gateway()
            .session(session)
            .unwrap()
            .state(),
        SessionState::ResyncRequired,
    );

    server
        .service_session(session, &mut queue, timestamp(5))
        .unwrap();

    assert!(matches!(
        queue.pop_front().unwrap().message,
        DownstreamMessage::Gateway(ServerMessage::ResyncRequired { .. }),
    ));

    assert!(matches!(
        server.synchronize_session(session, &mut queue, timestamp(6),),
        Err(ServerError::SnapshotUnavailable),
    ));
}

#[test]
fn recovered_subscriber_requires_new_snapshot() {
    let mut server = running_server();
    let session = SessionId::new(1);

    server.connect(session, timestamp(1)).unwrap();

    server
        .handle_request(
            session,
            ClientRequest::Subscribe {
                filter: SubscriptionFilter::all(),
            },
            timestamp(2),
        )
        .unwrap();

    let mut queue = ConnectionQueue::new(session, 16).unwrap();

    server
        .synchronize_session(session, &mut queue, timestamp(3))
        .unwrap();

    while queue.pop_front().is_some() {}

    server.ingest_event(add(2, 102), timestamp(4)).unwrap();

    server.ingest_event(add(1, 101), timestamp(5)).unwrap();

    assert!(server.is_publishable());

    assert_eq!(
        server
            .pipeline()
            .gateway()
            .session(session)
            .unwrap()
            .state(),
        SessionState::ResyncRequired,
    );

    let outcome = server
        .synchronize_session(session, &mut queue, timestamp(6))
        .unwrap();

    assert_eq!(
        outcome,
        SnapshotOutcome::Activated {
            publisher_watermark: 0,
            entry_count: 3,
        },
    );

    assert_eq!(order_count(&server), 2);

    assert!(
        server
            .pipeline()
            .gateway()
            .session(session)
            .unwrap()
            .is_active()
    );
}

// ============================================================
// Application failures
// ============================================================

#[test]
fn failed_book_mutation_does_not_advance_cursor() {
    let mut server = running_server();

    server.ingest_event(add(1, 100), timestamp(1)).unwrap();

    // A second add for an existing order identifier is
    // an invalid book mutation.
    assert!(server.ingest_event(add(2, 100), timestamp(2)).is_err());

    assert_eq!(server.status().next_expected, SequenceNumber::new(2),);

    assert_eq!(server.status().publication_watermark, 1,);

    assert_eq!(order_count(&server), 1);
    assert_eq!(server.state(), ServerState::Failed);
    assert!(!server.is_publishable());
}

#[test]
fn snapshot_recovery_buffers_without_mutating_book() {
    let mut server = Server::new(ServerConfig::default(), feed()).unwrap();

    server.register_instrument(instrument()).unwrap();
    server.start_recovering(timestamp(0)).unwrap();

    let outcome = server.ingest_event(add(1, 100), timestamp(1)).unwrap();

    assert_eq!(outcome, RecoveryOutcome::Buffered);
    assert_eq!(order_count(&server), 0);

    assert_eq!(server.recovery().state(), RecoveryState::AwaitingSnapshot,);

    assert_eq!(server.status().publication_watermark, 0,);

    assert!(!server.is_publishable());
}
