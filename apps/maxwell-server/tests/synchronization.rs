//! Integration tests for downstream market synchronization.

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, Instrument, InstrumentConfig, InstrumentId,
    MarketEvent, MonotonicTimestamp, OrderAdded, OrderId, Price, Quantity, SequenceNumber,
    SessionId as FeedSessionId, Side, StreamId,
};
use maxwell_gateway::{ClientRequest, ServerMessage, SessionId, SessionState};
use maxwell_publisher::SubscriptionFilter;

use maxwell_server::config::ServerConfig;
use maxwell_server::downstream::{ConnectionQueue, DeliveryError, DownstreamMessage};
use maxwell_server::error::ServerError;
use maxwell_server::server::Server;
use maxwell_server::snapshot::{SnapshotEntry, SnapshotOutcome};

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

fn instrument(id: InstrumentId) -> Instrument {
    Instrument::new(InstrumentConfig {
        id,
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

fn add(sequence: u64, instrument_id: InstrumentId, order_id: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed(), SequenceNumber::new(sequence)),
            instrument_id,
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

fn server_with_config(config: ServerConfig) -> Server {
    let mut server = Server::new(config, feed()).unwrap();

    // Deliberately register instruments out of order.
    server
        .register_instrument(instrument(InstrumentId::new(2)))
        .unwrap();

    server
        .register_instrument(instrument(InstrumentId::new(1)))
        .unwrap();

    server.start_empty(timestamp(0)).unwrap();

    server
}

fn running_server() -> Server {
    server_with_config(ServerConfig::default())
}

fn subscribe(server: &mut Server, filter: SubscriptionFilter<InstrumentId>, now: u64) {
    server.connect(session(), timestamp(now)).unwrap();

    server
        .handle_request(
            session(),
            ClientRequest::Subscribe { filter },
            timestamp(now + 1),
        )
        .unwrap();
}

// ============================================================
// Snapshot consistency
// ============================================================

#[test]
fn snapshot_contains_instrument_metadata_and_resting_orders() {
    let mut server = running_server();

    let first = InstrumentId::new(1);
    let second = InstrumentId::new(2);

    server
        .ingest_event(add(1, first, 100), timestamp(1))
        .unwrap();

    subscribe(&mut server, SubscriptionFilter::all(), 2);

    let mut queue = ConnectionQueue::new(session(), 16).unwrap();

    let outcome = server
        .synchronize_session(session(), &mut queue, timestamp(4))
        .unwrap();

    assert_eq!(
        outcome,
        SnapshotOutcome::Activated {
            publisher_watermark: 1,
            entry_count: 3,
        },
    );

    let begin = queue.pop_front().unwrap();

    assert_eq!(begin.transport_sequence, 1);

    assert_eq!(
        begin.message,
        DownstreamMessage::SnapshotBegin {
            publisher_watermark: 1,
            entry_count: 3,
        },
    );

    let metadata = queue.pop_front().unwrap();

    match metadata.message {
        DownstreamMessage::SnapshotEntry(SnapshotEntry::Instrument {
            instrument,
            feed: snapshot_feed,
            last_sequence,
        }) => {
            assert_eq!(instrument.id(), first);
            assert_eq!(snapshot_feed, feed());

            assert_eq!(last_sequence, Some(SequenceNumber::ONE),);
        }

        other => panic!("unexpected message: {other:?}"),
    }

    let order = queue.pop_front().unwrap();

    match order.message {
        DownstreamMessage::SnapshotEntry(SnapshotEntry::Order {
            instrument_id,
            order,
        }) => {
            assert_eq!(instrument_id, first);
            assert_eq!(order.order_id(), OrderId::new(100));
            assert_eq!(order.quantity(), Quantity::new(100));
            assert_eq!(order.price(), Price::new(10_000));
        }

        other => panic!("unexpected message: {other:?}"),
    }

    // Empty instruments must retain their metadata.
    let empty_book = queue.pop_front().unwrap();

    match empty_book.message {
        DownstreamMessage::SnapshotEntry(SnapshotEntry::Instrument {
            instrument,
            last_sequence,
            ..
        }) => {
            assert_eq!(instrument.id(), second);
            assert_eq!(last_sequence, None);
        }

        other => panic!("unexpected message: {other:?}"),
    }

    let end = queue.pop_front().unwrap();

    assert_eq!(end.transport_sequence, 5);

    assert_eq!(
        end.message,
        DownstreamMessage::SnapshotEnd {
            publisher_watermark: 1,
        },
    );

    assert!(queue.is_empty());
}

#[test]
fn snapshot_orders_are_deterministically_sorted() {
    let mut server = running_server();
    let id = InstrumentId::new(1);

    for (sequence, order_id) in [(1, 300), (2, 100), (3, 200)] {
        server
            .ingest_event(add(sequence, id, order_id), timestamp(sequence))
            .unwrap();
    }

    subscribe(&mut server, SubscriptionFilter::all(), 4);

    let mut queue = ConnectionQueue::new(session(), 16).unwrap();

    server
        .synchronize_session(session(), &mut queue, timestamp(6))
        .unwrap();

    let mut order_ids = Vec::new();

    while let Some(frame) = queue.pop_front() {
        if let DownstreamMessage::SnapshotEntry(SnapshotEntry::Order { order, .. }) = frame.message
        {
            order_ids.push(order.order_id());
        }
    }

    assert_eq!(
        order_ids,
        vec![OrderId::new(100), OrderId::new(200), OrderId::new(300),],
    );
}

// ============================================================
// Snapshot and incremental ordering
// ============================================================

#[test]
fn incrementals_follow_snapshot_transaction() {
    let mut server = running_server();
    let id = InstrumentId::new(1);

    server.ingest_event(add(1, id, 100), timestamp(1)).unwrap();

    subscribe(&mut server, SubscriptionFilter::all(), 2);

    let mut queue = ConnectionQueue::new(session(), 16).unwrap();

    server
        .synchronize_session(session(), &mut queue, timestamp(4))
        .unwrap();

    let event = add(2, id, 200);

    server.ingest_event(event, timestamp(5)).unwrap();

    server
        .service_session(session(), &mut queue, timestamp(6))
        .unwrap();

    let mut snapshot_finished = false;
    let mut incremental_count = 0;
    let mut expected_transport_sequence = 1;

    while let Some(frame) = queue.pop_front() {
        assert_eq!(frame.transport_sequence, expected_transport_sequence,);

        expected_transport_sequence += 1;

        match frame.message {
            DownstreamMessage::SnapshotEnd {
                publisher_watermark,
            } => {
                assert_eq!(publisher_watermark, 1);

                snapshot_finished = true;
            }

            DownstreamMessage::Gateway(ServerMessage::Incremental(publication)) => {
                assert!(snapshot_finished);
                assert_eq!(publication.sequence, 2);
                assert_eq!(publication.payload, event);

                incremental_count += 1;
            }

            _ => {}
        }
    }

    assert!(snapshot_finished);
    assert_eq!(incremental_count, 1);
}

// ============================================================
// Subscription filtering
// ============================================================

#[test]
fn snapshot_contains_only_subscribed_instruments() {
    let mut server = running_server();

    let selected = InstrumentId::new(1);
    let excluded = InstrumentId::new(2);

    server
        .ingest_event(add(1, selected, 100), timestamp(1))
        .unwrap();

    server
        .ingest_event(add(2, excluded, 200), timestamp(2))
        .unwrap();

    let filter = SubscriptionFilter::for_instruments(&[selected], 1).unwrap();

    subscribe(&mut server, filter, 3);

    let mut queue = ConnectionQueue::new(session(), 16).unwrap();

    let outcome = server
        .synchronize_session(session(), &mut queue, timestamp(5))
        .unwrap();

    assert_eq!(
        outcome,
        SnapshotOutcome::Activated {
            publisher_watermark: 2,
            entry_count: 2,
        },
    );

    let mut entry_count = 0;

    while let Some(frame) = queue.pop_front() {
        if let DownstreamMessage::SnapshotEntry(entry) = frame.message {
            assert_eq!(entry.instrument_id(), selected);

            entry_count += 1;
        }
    }

    assert_eq!(entry_count, 2);
}

#[test]
fn filtered_global_sequences_do_not_create_transport_gaps() {
    let mut server = running_server();

    let selected = InstrumentId::new(1);
    let excluded = InstrumentId::new(2);

    let filter = SubscriptionFilter::for_instruments(&[selected], 1).unwrap();

    subscribe(&mut server, filter, 1);

    let mut queue = ConnectionQueue::new(session(), 16).unwrap();

    server
        .synchronize_session(session(), &mut queue, timestamp(3))
        .unwrap();

    while queue.pop_front().is_some() {}

    let first_transport_sequence = queue.next_sequence().unwrap();

    server
        .ingest_event(add(1, selected, 100), timestamp(4))
        .unwrap();

    server
        .ingest_event(add(2, excluded, 200), timestamp(5))
        .unwrap();

    server
        .ingest_event(add(3, selected, 300), timestamp(6))
        .unwrap();

    server
        .service_session(session(), &mut queue, timestamp(7))
        .unwrap();

    assert_eq!(queue.len(), 2);

    let mut global_sequences = Vec::new();

    for offset in 0..2 {
        let frame = queue.pop_front().unwrap();

        assert_eq!(frame.transport_sequence, first_transport_sequence + offset,);

        match frame.message {
            DownstreamMessage::Gateway(ServerMessage::Incremental(publication)) => {
                global_sequences.push(publication.sequence);
            }

            other => panic!("unexpected message: {other:?}"),
        }
    }

    assert_eq!(global_sequences, vec![1, 3]);
    assert_eq!(server.status().publication_watermark, 3);
}

// ============================================================
// Snapshot capacity
// ============================================================

#[test]
fn snapshot_capacity_failure_does_not_activate_session() {
    let mut config = ServerConfig::default();

    config.snapshot.max_entries = 1;

    let mut server = server_with_config(config);

    subscribe(&mut server, SubscriptionFilter::all(), 1);

    let mut queue = ConnectionQueue::new(session(), 16).unwrap();

    // Two registered instruments require two metadata
    // records, even though both books are empty.
    assert!(matches!(
        server.synchronize_session(session(), &mut queue, timestamp(3),),
        Err(ServerError::SnapshotCapacityExceeded {
            limit: 1,
            required: 2,
        }),
    ));

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
fn snapshot_transport_rejection_is_atomic() {
    let mut server = running_server();

    subscribe(&mut server, SubscriptionFilter::all(), 1);

    // Two instrument records plus begin and end
    // require four available transport slots.
    let mut queue = ConnectionQueue::new(session(), 3).unwrap();

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
fn snapshot_retry_captures_latest_market_watermark() {
    let mut server = running_server();

    subscribe(&mut server, SubscriptionFilter::all(), 1);

    let mut queue = ConnectionQueue::new(session(), 5).unwrap();

    // Occupy two transport slots, leaving insufficient
    // capacity for the initial four-frame snapshot.
    for _ in 0..2 {
        queue
            .try_accept(
                session(),
                DownstreamMessage::Gateway(ServerMessage::Heartbeat {
                    timestamp_ns: 2,
                    publisher_watermark: 0,
                }),
            )
            .unwrap();
    }

    assert_eq!(
        server
            .synchronize_session(session(), &mut queue, timestamp(3),)
            .unwrap(),
        SnapshotOutcome::Rejected(DeliveryError::Backpressure,),
    );

    while queue.pop_front().is_some() {}

    server
        .ingest_event(add(1, InstrumentId::new(1), 100), timestamp(4))
        .unwrap();

    // The retry must capture the updated market rather
    // than reuse the previously rejected snapshot.
    let outcome = server
        .synchronize_session(session(), &mut queue, timestamp(5))
        .unwrap();

    assert_eq!(
        outcome,
        SnapshotOutcome::Activated {
            publisher_watermark: 1,
            entry_count: 3,
        },
    );

    assert_eq!(queue.len(), 5);

    let begin = queue.pop_front().unwrap();

    assert_eq!(begin.transport_sequence, 3);

    assert_eq!(
        begin.message,
        DownstreamMessage::SnapshotBegin {
            publisher_watermark: 1,
            entry_count: 3,
        },
    );
}
