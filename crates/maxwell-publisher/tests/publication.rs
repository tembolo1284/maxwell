//! Integration tests for publication, filtering and delivery.

use maxwell_publisher::{
    BackpressurePolicy, EventKind, Publication, PublicationSink, Publisher, PublisherConfig,
    PublisherError, SubscriberId, SubscriberState, SubscriptionFilter,
};

/// Representative fixed-point market data payload.
///
/// The actual application will supply Maxwell's existing
/// protocol and fixed-point types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarketData {
    price_ticks: i64,
    quantity: u64,
}

type TestPublisher = Publisher<u32, MarketData>;
type TestPublication = Publication<u32, MarketData>;

fn make_publisher() -> TestPublisher {
    Publisher::new(PublisherConfig {
        max_subscribers: 8,
        queue_capacity: 16,
        max_instruments_per_subscription: 16,
        backpressure_policy: BackpressurePolicy::RequireResync,
    })
    .unwrap()
}

fn subscribe(
    publisher: &mut TestPublisher,
    id: u64,
    filter: SubscriptionFilter<u32>,
) -> SubscriberId {
    let id = SubscriberId::new(id);

    publisher.subscribe(id, filter).unwrap();

    publisher.activate(id, publisher.last_sequence()).unwrap();

    id
}

fn payload(price_ticks: i64, quantity: u64) -> MarketData {
    MarketData {
        price_ticks,
        quantity,
    }
}

#[derive(Default)]
struct RecordingSink {
    received: Vec<TestPublication>,
}

impl PublicationSink<u32, MarketData> for RecordingSink {
    type Error = ();

    fn try_send(&mut self, publication: &TestPublication) -> Result<(), Self::Error> {
        self.received.push(*publication);
        Ok(())
    }
}

#[test]
fn publishes_identical_events_to_multiple_subscribers() {
    let mut publisher = make_publisher();

    let first = subscribe(&mut publisher, 1, SubscriptionFilter::all());

    let second = subscribe(&mut publisher, 2, SubscriptionFilter::all());

    let report = publisher
        .publish(Some(100), EventKind::Trade, 1_000_000, payload(125_000, 50))
        .unwrap();

    assert_eq!(report.sequence, 1);
    assert_eq!(report.matched, 2);
    assert_eq!(report.enqueued, 2);
    assert_eq!(report.resync_required, 0);

    let a = publisher.pop(first).unwrap().unwrap();
    let b = publisher.pop(second).unwrap().unwrap();

    assert_eq!(a, b);
    assert_eq!(a.instrument, Some(100));
    assert_eq!(a.payload.price_ticks, 125_000);
    assert_eq!(a.payload.quantity, 50);
}

#[test]
fn routes_events_by_instrument() {
    let mut publisher = make_publisher();

    let first = subscribe(
        &mut publisher,
        1,
        SubscriptionFilter::for_instruments(&[100], 16).unwrap(),
    );

    let second = subscribe(
        &mut publisher,
        2,
        SubscriptionFilter::for_instruments(&[200], 16).unwrap(),
    );

    publisher
        .publish(Some(100), EventKind::BookUpdate, 1000, payload(100, 10))
        .unwrap();

    publisher
        .publish(Some(200), EventKind::BookUpdate, 2000, payload(200, 20))
        .unwrap();

    let a = publisher.pop(first).unwrap().unwrap();
    let b = publisher.pop(second).unwrap().unwrap();

    assert_eq!(a.instrument, Some(100));
    assert_eq!(b.instrument, Some(200));

    assert!(publisher.pop(first).unwrap().is_none());
    assert!(publisher.pop(second).unwrap().is_none());
}

#[test]
fn routes_events_by_kind() {
    let mut publisher = make_publisher();

    let trades = subscribe(
        &mut publisher,
        1,
        SubscriptionFilter::all().with_event_kinds(&[EventKind::Trade]),
    );

    let quotes = subscribe(
        &mut publisher,
        2,
        SubscriptionFilter::all().with_event_kinds(&[EventKind::Quote]),
    );

    publisher
        .publish(Some(100), EventKind::Trade, 1000, payload(100, 10))
        .unwrap();

    publisher
        .publish(Some(100), EventKind::Quote, 2000, payload(101, 20))
        .unwrap();

    assert_eq!(
        publisher.pop(trades).unwrap().unwrap().kind,
        EventKind::Trade
    );

    assert_eq!(
        publisher.pop(quotes).unwrap().unwrap().kind,
        EventKind::Quote
    );
}

#[test]
fn filtered_sequence_gaps_are_preserved() {
    let mut publisher = make_publisher();

    let id = subscribe(
        &mut publisher,
        1,
        SubscriptionFilter::for_instruments(&[100], 16).unwrap(),
    );

    for instrument in [100, 200, 100] {
        publisher
            .publish(Some(instrument), EventKind::Trade, 1000, payload(100, 1))
            .unwrap();
    }

    let first = publisher.pop(id).unwrap().unwrap();
    let second = publisher.pop(id).unwrap().unwrap();

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 3);
}

#[test]
fn publishes_global_control_events() {
    let mut publisher = make_publisher();

    let enabled = subscribe(&mut publisher, 1, SubscriptionFilter::all());

    let disabled = subscribe(
        &mut publisher,
        2,
        SubscriptionFilter::all().with_global_events(false),
    );

    let report = publisher
        .publish(None, EventKind::Heartbeat, 1000, payload(0, 0))
        .unwrap();

    assert_eq!(report.matched, 1);

    assert_eq!(
        publisher.pop(enabled).unwrap().unwrap().kind,
        EventKind::Heartbeat
    );

    assert!(publisher.pop(disabled).unwrap().is_none());
}

#[test]
fn drains_publications_in_fifo_order() {
    let mut publisher = make_publisher();

    let id = subscribe(&mut publisher, 1, SubscriptionFilter::all());

    for value in 1..=10 {
        publisher
            .publish(
                Some(100),
                EventKind::BookUpdate,
                value,
                payload(value as i64, value),
            )
            .unwrap();
    }

    let mut sink = RecordingSink::default();

    let first = publisher.drain(id, &mut sink, 4).unwrap();

    assert_eq!(first.sent, 4);
    assert_eq!(first.remaining, 6);
    assert!(!first.is_blocked());

    let second = publisher.drain(id, &mut sink, 16).unwrap();

    assert_eq!(second.sent, 6);
    assert_eq!(second.remaining, 0);

    let sequences: Vec<_> = sink
        .received
        .iter()
        .map(|publication| publication.sequence)
        .collect();

    assert_eq!(sequences, (1u64..=10).collect::<Vec<_>>());
}

#[test]
fn zero_drain_budget_preserves_queue() {
    let mut publisher = make_publisher();

    let id = subscribe(&mut publisher, 1, SubscriptionFilter::all());

    publisher
        .publish(Some(100), EventKind::Trade, 1000, payload(100, 10))
        .unwrap();

    let mut sink = RecordingSink::default();

    let report = publisher.drain(id, &mut sink, 0).unwrap();

    assert_eq!(report.sent, 0);
    assert_eq!(report.remaining, 1);
    assert!(sink.received.is_empty());
}

#[test]
fn peek_does_not_consume_publication() {
    let mut publisher = make_publisher();

    let id = subscribe(&mut publisher, 1, SubscriptionFilter::all());

    publisher
        .publish(Some(100), EventKind::Trade, 1000, payload(100, 10))
        .unwrap();

    let first = *publisher.peek(id).unwrap().unwrap();
    let second = *publisher.peek(id).unwrap().unwrap();

    assert_eq!(first, second);
    assert_eq!(publisher.stats().deliveries_dequeued, 0);

    assert_eq!(publisher.pop(id).unwrap(), Some(first));
    assert_eq!(publisher.stats().deliveries_dequeued, 1);
}

#[test]
fn new_subscriber_requires_snapshot_activation() {
    let mut publisher = make_publisher();
    let id = SubscriberId::new(1);

    publisher.subscribe(id, SubscriptionFilter::all()).unwrap();

    assert_eq!(
        publisher.subscriber(id).unwrap().state(),
        SubscriberState::PendingSnapshot
    );

    assert_eq!(
        publisher.pop(id),
        Err(PublisherError::InvalidSubscriberState { subscriber_id: id })
    );

    publisher
        .publish(Some(100), EventKind::BookUpdate, 1000, payload(100, 10))
        .unwrap();

    publisher.activate(id, 1).unwrap();

    publisher
        .publish(Some(100), EventKind::BookUpdate, 2000, payload(101, 20))
        .unwrap();

    let publication = publisher.pop(id).unwrap().unwrap();

    assert_eq!(publication.sequence, 2);
    assert_eq!(publication.payload.price_ticks, 101);
}

#[test]
fn statistics_reconcile_with_queue_occupancy() {
    let mut publisher = make_publisher();

    let first = subscribe(&mut publisher, 1, SubscriptionFilter::all());

    let second = subscribe(&mut publisher, 2, SubscriptionFilter::all());

    for value in 1..=5 {
        publisher
            .publish(Some(100), EventKind::Trade, value, payload(100, value))
            .unwrap();
    }

    publisher.pop(first).unwrap();
    publisher.pop(first).unwrap();

    publisher.unsubscribe(second).unwrap();

    let stats = publisher.stats();

    assert_eq!(stats.publications_accepted, 5);
    assert_eq!(stats.deliveries_enqueued, 10);
    assert_eq!(stats.deliveries_dequeued, 2);
    assert_eq!(stats.deliveries_discarded, 5);
    assert_eq!(stats.pending_deliveries(), 3);
    assert_eq!(stats.queue_high_watermark, 5);
    assert_eq!(stats.subscribers_registered, 2);
    assert_eq!(stats.subscribers_removed, 1);

    assert_eq!(publisher.subscriber(first).unwrap().queue_len(), 3);
}
