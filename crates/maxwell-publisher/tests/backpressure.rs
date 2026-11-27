//! Integration tests for explicit backpressure and recovery.

use maxwell_publisher::{
    BackpressurePolicy, EventKind, Publication, PublicationSink, Publisher, PublisherConfig,
    PublisherError, SubscriberId, SubscriberState, SubscriptionFilter,
};

type TestPublisher = Publisher<u32, u64>;

fn make_publisher(policy: BackpressurePolicy, queue_capacity: usize) -> TestPublisher {
    Publisher::new(PublisherConfig {
        max_subscribers: 8,
        queue_capacity,
        max_instruments_per_subscription: 16,
        backpressure_policy: policy,
    })
    .unwrap()
}

fn subscribe(publisher: &mut TestPublisher, id: u64) -> SubscriberId {
    let id = SubscriberId::new(id);

    publisher.subscribe(id, SubscriptionFilter::all()).unwrap();

    publisher.activate(id, publisher.last_sequence()).unwrap();

    id
}

fn publish(publisher: &mut TestPublisher, value: u64) {
    publisher
        .publish(Some(100), EventKind::BookUpdate, value * 1000, value)
        .unwrap();
}

#[test]
fn slow_subscriber_does_not_block_healthy_subscriber() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 2);

    let slow = subscribe(&mut publisher, 1);
    let fast = subscribe(&mut publisher, 2);

    publish(&mut publisher, 1);
    publisher.pop(fast).unwrap();

    publish(&mut publisher, 2);
    publisher.pop(fast).unwrap();

    let report = publisher
        .publish(Some(100), EventKind::BookUpdate, 3000, 3)
        .unwrap();

    assert_eq!(report.sequence, 3);
    assert_eq!(report.matched, 2);
    assert_eq!(report.enqueued, 1);
    assert_eq!(report.resync_required, 1);
    assert_eq!(report.discarded, 2);

    assert_eq!(
        publisher.subscriber(slow).unwrap().state(),
        SubscriberState::ResyncRequired
    );

    assert_eq!(publisher.subscriber(slow).unwrap().queue_len(), 0);

    assert_eq!(
        publisher.pop(slow),
        Err(PublisherError::ResyncRequired {
            subscriber_id: slow,
        })
    );

    assert_eq!(publisher.pop(fast).unwrap().unwrap().payload, 3);

    publish(&mut publisher, 4);

    assert_eq!(publisher.pop(fast).unwrap().unwrap().payload, 4);
}

#[test]
fn resync_subscriber_receives_no_incremental_updates() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 1);

    let id = subscribe(&mut publisher, 1);

    publish(&mut publisher, 1);
    publish(&mut publisher, 2);

    for value in 3..=100 {
        publish(&mut publisher, value);
    }

    let subscriber = publisher.subscriber(id).unwrap();

    assert_eq!(subscriber.state(), SubscriberState::ResyncRequired);

    assert!(subscriber.queue_is_empty());
    assert_eq!(publisher.stats().resync_transitions, 1);
    assert_eq!(publisher.last_sequence(), 100);
}

#[test]
fn subscriber_can_resume_after_valid_snapshot() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 1);

    let id = subscribe(&mut publisher, 1);

    publish(&mut publisher, 1);
    publish(&mut publisher, 2);
    publish(&mut publisher, 3);

    assert_eq!(
        publisher.activate(id, 2),
        Err(PublisherError::InvalidWatermark {
            expected: 3,
            received: 2,
        })
    );

    // Represents an externally established snapshot
    // corresponding to publication watermark 3.
    publisher.activate(id, 3).unwrap();

    publish(&mut publisher, 4);

    let publication = publisher.pop(id).unwrap().unwrap();

    assert_eq!(publication.sequence, 4);
    assert_eq!(publication.payload, 4);

    assert_eq!(
        publisher.subscriber(id).unwrap().state(),
        SubscriberState::Active
    );
}

#[test]
fn reject_policy_prevents_partial_fanout() {
    let mut publisher = make_publisher(BackpressurePolicy::RejectPublication, 1);

    // The healthy subscriber has the lower identifier.
    // This ensures the preflight must occur before any enqueue.
    let healthy = subscribe(&mut publisher, 1);
    let blocked = subscribe(&mut publisher, 2);

    publish(&mut publisher, 1);

    publisher.pop(healthy).unwrap();

    let result = publisher.publish(Some(100), EventKind::BookUpdate, 2000, 2);

    assert_eq!(
        result,
        Err(PublisherError::Backpressure {
            subscriber_id: blocked,
        })
    );

    assert_eq!(publisher.last_sequence(), 1);

    assert_eq!(publisher.subscriber(healthy).unwrap().queue_len(), 0);

    assert_eq!(publisher.subscriber(blocked).unwrap().queue_len(), 1);

    assert_eq!(publisher.stats().publications_accepted, 1);
    assert_eq!(publisher.stats().publications_rejected, 1);
    assert_eq!(publisher.stats().deliveries_enqueued, 2);
}

#[test]
fn rejected_publication_can_be_retried_without_duplicates() {
    let mut publisher = make_publisher(BackpressurePolicy::RejectPublication, 1);

    let first = subscribe(&mut publisher, 1);
    let second = subscribe(&mut publisher, 2);

    publish(&mut publisher, 1);

    assert!(
        publisher
            .publish(Some(100), EventKind::BookUpdate, 2000, 2,)
            .is_err()
    );

    publisher.pop(first).unwrap();
    publisher.pop(second).unwrap();

    let report = publisher
        .publish(Some(100), EventKind::BookUpdate, 2000, 2)
        .unwrap();

    assert_eq!(report.sequence, 2);
    assert_eq!(report.enqueued, 2);

    for id in [first, second] {
        let publication = publisher.pop(id).unwrap().unwrap();

        assert_eq!(publication.sequence, 2);
        assert_eq!(publication.payload, 2);
        assert!(publisher.pop(id).unwrap().is_none());
    }
}

#[test]
fn full_nonmatching_subscriber_does_not_reject_publication() {
    let mut publisher = make_publisher(BackpressurePolicy::RejectPublication, 1);

    let first = SubscriberId::new(1);
    let second = SubscriberId::new(2);

    publisher
        .subscribe(
            first,
            SubscriptionFilter::for_instruments(&[100], 16).unwrap(),
        )
        .unwrap();

    publisher
        .subscribe(
            second,
            SubscriptionFilter::for_instruments(&[200], 16).unwrap(),
        )
        .unwrap();

    publisher.activate(first, 0).unwrap();
    publisher.activate(second, 0).unwrap();

    publisher
        .publish(Some(100), EventKind::Trade, 1000, 1)
        .unwrap();

    let report = publisher
        .publish(Some(200), EventKind::Trade, 2000, 2)
        .unwrap();

    assert_eq!(report.sequence, 2);
    assert_eq!(report.enqueued, 1);
    assert_eq!(publisher.stats().publications_rejected, 0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkError {
    WouldBlock,
}

struct LimitedSink {
    remaining_capacity: usize,
    received: Vec<u64>,
}

impl PublicationSink<u32, u64> for LimitedSink {
    type Error = SinkError;

    fn try_send(&mut self, publication: &Publication<u32, u64>) -> Result<(), Self::Error> {
        if self.remaining_capacity == 0 {
            return Err(SinkError::WouldBlock);
        }

        self.remaining_capacity -= 1;
        self.received.push(publication.sequence);

        Ok(())
    }
}

#[test]
fn sink_backpressure_preserves_failed_publication() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 8);

    let id = subscribe(&mut publisher, 1);

    for value in 1..=5 {
        publish(&mut publisher, value);
    }

    let mut sink = LimitedSink {
        remaining_capacity: 2,
        received: Vec::new(),
    };

    let report = publisher.drain(id, &mut sink, 8).unwrap();

    assert_eq!(report.sent, 2);
    assert_eq!(report.remaining, 3);
    assert_eq!(report.error, Some(SinkError::WouldBlock));

    assert_eq!(publisher.peek(id).unwrap().unwrap().sequence, 3);

    assert_eq!(sink.received, vec![1, 2]);

    sink.remaining_capacity = 3;

    let report = publisher.drain(id, &mut sink, 8).unwrap();

    assert_eq!(report.sent, 3);
    assert_eq!(report.remaining, 0);
    assert_eq!(report.error, None);

    assert_eq!(sink.received, vec![1, 2, 3, 4, 5]);

    assert_eq!(publisher.stats().sink_failures, 1);
    assert_eq!(publisher.stats().deliveries_dequeued, 5);
}

#[test]
fn blocked_sink_does_not_consume_queue_entries() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 4);

    let id = subscribe(&mut publisher, 1);

    publish(&mut publisher, 1);

    let mut sink = LimitedSink {
        remaining_capacity: 0,
        received: Vec::new(),
    };

    for _ in 0..10 {
        let report = publisher.drain(id, &mut sink, 4).unwrap();

        assert_eq!(report.sent, 0);
        assert_eq!(report.remaining, 1);
        assert!(report.is_blocked());
    }

    assert_eq!(publisher.stats().sink_failures, 10);
    assert_eq!(publisher.stats().deliveries_dequeued, 0);
    assert_eq!(publisher.stats().pending_deliveries(), 1);
}

#[test]
fn overflow_statistics_account_for_all_deliveries() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 2);

    let id = subscribe(&mut publisher, 1);

    publish(&mut publisher, 1);
    publish(&mut publisher, 2);
    publish(&mut publisher, 3);

    let stats = publisher.stats();

    assert_eq!(stats.publications_accepted, 3);
    assert_eq!(stats.deliveries_enqueued, 2);
    assert_eq!(stats.deliveries_discarded, 2);
    assert_eq!(stats.deliveries_missed, 1);
    assert_eq!(stats.resync_transitions, 1);
    assert_eq!(stats.pending_deliveries(), 0);

    assert_eq!(
        publisher.subscriber(id).unwrap().state(),
        SubscriberState::ResyncRequired
    );
}

#[test]
fn simultaneous_overflow_invalidates_all_affected_subscribers() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 1);

    let first = subscribe(&mut publisher, 1);
    let second = subscribe(&mut publisher, 2);
    let third = subscribe(&mut publisher, 3);

    publish(&mut publisher, 1);

    let report = publisher
        .publish(Some(100), EventKind::BookUpdate, 2000, 2)
        .unwrap();

    assert_eq!(report.matched, 3);
    assert_eq!(report.enqueued, 0);
    assert_eq!(report.resync_required, 3);
    assert_eq!(report.discarded, 3);

    for id in [first, second, third] {
        assert_eq!(
            publisher.subscriber(id).unwrap().state(),
            SubscriberState::ResyncRequired
        );
    }

    assert_eq!(publisher.active_subscriber_count(), 0);
}
