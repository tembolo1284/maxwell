//! Integration tests for deterministic publication and replay.

use maxwell_publisher::{
    BackpressurePolicy, EventKind, Publication, PublishReport, Publisher, PublisherConfig,
    PublisherError, PublisherStats, SubscriberId, SubscriberState, SubscriptionFilter,
};

type TestPublisher = Publisher<u32, u64>;
type TestPublication = Publication<u32, u64>;

#[derive(Debug, PartialEq, Eq)]
struct SubscriberResult {
    id: SubscriberId,
    state: SubscriberState,
    publications: Vec<TestPublication>,
}

#[derive(Debug, PartialEq, Eq)]
struct RunResult {
    reports: Vec<PublishReport>,
    subscribers: Vec<SubscriberResult>,
    stats: PublisherStats,
    last_sequence: u64,
}

fn make_publisher(policy: BackpressurePolicy, queue_capacity: usize) -> TestPublisher {
    Publisher::new(PublisherConfig {
        max_subscribers: 16,
        queue_capacity,
        max_instruments_per_subscription: 16,
        backpressure_policy: policy,
    })
    .unwrap()
}

fn register(publisher: &mut TestPublisher, id: u64, filter: SubscriptionFilter<u32>) {
    let id = SubscriberId::new(id);

    publisher.subscribe(id, filter).unwrap();

    publisher.activate(id, publisher.last_sequence()).unwrap();
}

fn run_scenario(registration_order: &[u64]) -> RunResult {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 32);

    for id in registration_order {
        let filter = match id {
            1 => SubscriptionFilter::all(),

            2 => SubscriptionFilter::for_instruments(&[100], 16).unwrap(),

            3 => SubscriptionFilter::all().with_event_kinds(&[EventKind::Trade]),

            4 => SubscriptionFilter::for_instruments(&[200], 16).unwrap(),

            _ => SubscriptionFilter::none(),
        };

        register(&mut publisher, *id, filter);
    }

    let mut reports = Vec::new();

    for index in 0u64..100 {
        let instrument = if index % 2 == 0 { 100 } else { 200 };

        let kind = if index % 3 == 0 {
            EventKind::Trade
        } else {
            EventKind::BookUpdate
        };

        let report = publisher
            .publish(Some(instrument), kind, index * 1000, index)
            .unwrap();

        reports.push(report);
    }

    let subscribers = publisher
        .subscribers()
        .map(|subscriber| SubscriberResult {
            id: subscriber.id(),
            state: subscriber.state(),
            publications: subscriber.pending().copied().collect(),
        })
        .collect();

    RunResult {
        reports,
        subscribers,
        stats: *publisher.stats(),
        last_sequence: publisher.last_sequence(),
    }
}

#[test]
fn identical_inputs_produce_identical_outputs() {
    let first = run_scenario(&[1, 2, 3, 4]);
    let second = run_scenario(&[1, 2, 3, 4]);

    assert_eq!(first, second);
}

#[test]
fn registration_order_does_not_change_results() {
    let first = run_scenario(&[1, 2, 3, 4]);
    let second = run_scenario(&[4, 3, 2, 1]);
    let third = run_scenario(&[2, 4, 1, 3]);

    assert_eq!(first, second);
    assert_eq!(second, third);
}

#[test]
fn backpressure_error_selects_lowest_matching_subscriber_id() {
    let mut publisher = make_publisher(BackpressurePolicy::RejectPublication, 1);

    for id in [9, 3, 7, 1] {
        register(&mut publisher, id, SubscriptionFilter::all());
    }

    publisher
        .publish(Some(100), EventKind::Trade, 1000, 1)
        .unwrap();

    let result = publisher.publish(Some(100), EventKind::Trade, 2000, 2);

    assert_eq!(
        result,
        Err(PublisherError::Backpressure {
            subscriber_id: SubscriberId::new(1),
        })
    );
}

#[test]
fn publication_order_does_not_depend_on_timestamps() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 8);

    register(&mut publisher, 1, SubscriptionFilter::all());

    let timestamps = [3000, 1000, 1000, 2000];

    for (index, timestamp) in timestamps.iter().enumerate() {
        publisher
            .publish(Some(100), EventKind::Trade, *timestamp, index as u64)
            .unwrap();
    }

    let id = SubscriberId::new(1);

    for (index, timestamp) in timestamps.iter().enumerate() {
        let publication = publisher.pop(id).unwrap().unwrap();

        assert_eq!(publication.sequence, index as u64 + 1);
        assert_eq!(publication.timestamp_ns, *timestamp);
        assert_eq!(publication.payload, index as u64);
    }
}

#[test]
fn unsubscribe_and_resubscribe_preserves_sorted_order() {
    let mut publisher = make_publisher(BackpressurePolicy::RequireResync, 8);

    for id in [5, 1, 3, 2, 4] {
        register(&mut publisher, id, SubscriptionFilter::all());
    }

    publisher.unsubscribe(SubscriberId::new(3)).unwrap();
    publisher.unsubscribe(SubscriberId::new(1)).unwrap();

    register(&mut publisher, 3, SubscriptionFilter::all());

    register(&mut publisher, 1, SubscriptionFilter::all());

    let ids: Vec<_> = publisher
        .subscribers()
        .map(|subscriber| subscriber.id().get())
        .collect();

    assert_eq!(ids, vec![1, 2, 3, 4, 5]);
}

/// Dependency-free deterministic pseudo-random generator.
///
/// Used only for test data generation.
fn next_random(state: &mut u64) -> u64 {
    let mut value = *state;

    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;

    *state = value;
    value
}

fn run_replay(seed: u64) -> Vec<TestPublication> {
    let mut publisher = make_publisher(BackpressurePolicy::RejectPublication, 64);

    register(&mut publisher, 1, SubscriptionFilter::all());

    let id = SubscriberId::new(1);
    let mut random_state = seed;
    let mut output = Vec::new();

    for index in 0u64..10_000 {
        let random = next_random(&mut random_state);

        let instrument = (random % 32) as u32;

        let kind = match random % 3 {
            0 => EventKind::Trade,
            1 => EventKind::Quote,
            _ => EventKind::BookUpdate,
        };

        publisher
            .publish(Some(instrument), kind, index * 1000, random)
            .unwrap();

        if index % 32 == 31 {
            while let Some(publication) = publisher.pop(id).unwrap() {
                output.push(publication);
            }
        }
    }

    while let Some(publication) = publisher.pop(id).unwrap() {
        output.push(publication);
    }

    assert_eq!(publisher.last_sequence(), 10_000);
    assert_eq!(publisher.stats().publications_rejected, 0);
    assert_eq!(publisher.stats().pending_deliveries(), 0);

    output
}

#[test]
fn deterministic_replay_of_ten_thousand_events() {
    let first = run_replay(0x1234_5678_9ABC_DEF0);
    let second = run_replay(0x1234_5678_9ABC_DEF0);

    assert_eq!(first.len(), 10_000);
    assert_eq!(first, second);

    for (index, publication) in first.iter().enumerate() {
        assert_eq!(publication.sequence, index as u64 + 1);
    }
}

#[test]
fn different_replay_inputs_produce_different_outputs() {
    let first = run_replay(12345);
    let second = run_replay(67890);

    assert_ne!(first, second);
}

#[test]
fn repeated_publication_respects_memory_bounds() {
    let mut publisher = make_publisher(BackpressurePolicy::RejectPublication, 32);

    for id in 1..=8 {
        register(&mut publisher, id, SubscriptionFilter::all());
    }

    for sequence in 1u64..=10_000 {
        let report = publisher
            .publish(Some(100), EventKind::BookUpdate, sequence * 1000, sequence)
            .unwrap();

        assert_eq!(report.enqueued, 8);

        if sequence % 32 == 0 {
            for id in 1..=8 {
                let id = SubscriberId::new(id);

                while publisher.pop(id).unwrap().is_some() {}
            }
        }

        for subscriber in publisher.subscribers() {
            assert!(subscriber.queue_len() <= subscriber.queue_capacity());
        }
    }

    for id in 1..=8 {
        let id = SubscriberId::new(id);

        while publisher.pop(id).unwrap().is_some() {}
    }

    let stats = publisher.stats();

    assert_eq!(stats.publications_accepted, 10_000);
    assert_eq!(stats.deliveries_enqueued, 80_000);
    assert_eq!(stats.deliveries_dequeued, 80_000);
    assert_eq!(stats.deliveries_discarded, 0);
    assert_eq!(stats.deliveries_missed, 0);
    assert_eq!(stats.pending_deliveries(), 0);
    assert_eq!(stats.queue_high_watermark, 32);
}
