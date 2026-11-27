//! Deterministic gateway replay and bounded-resource tests.

use maxwell_gateway::{
    ClientRequest, Gateway, GatewayConfig, GatewaySink, GatewayStats, ServerMessage, Session,
    SessionId,
};

use maxwell_publisher::{
    BackpressurePolicy, EventKind, PublisherConfig, PublisherStats, SubscriptionFilter,
};

type TestGateway = Gateway<u32, u64>;
type TestMessage = ServerMessage<u32, u64>;

#[derive(Default)]
struct RecordingSink {
    messages: Vec<(SessionId, TestMessage)>,
}

impl GatewaySink<u32, u64> for RecordingSink {
    type Error = ();

    fn try_send(
        &mut self,
        session_id: SessionId,
        message: &TestMessage,
    ) -> Result<(), Self::Error> {
        self.messages.push((session_id, *message));

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ReplayResult {
    messages: Vec<(SessionId, TestMessage)>,
    sessions: Vec<Session>,
    gateway_stats: GatewayStats,
    publisher_stats: PublisherStats,
    watermark: u64,
}

fn config() -> GatewayConfig {
    GatewayConfig {
        publisher: PublisherConfig {
            max_subscribers: 8,
            queue_capacity: 64,
            max_instruments_per_subscription: 16,
            backpressure_policy: BackpressurePolicy::RequireResync,
        },
        heartbeat_interval_ns: 100,
        session_timeout_ns: 1000,
        max_messages_per_service: 64,
    }
}

fn register(gateway: &mut TestGateway, value: u64, filter: SubscriptionFilter<u32>) {
    let id = SessionId::new(value);

    gateway.connect(id, 0).unwrap();

    gateway
        .handle_request(id, ClientRequest::Subscribe { filter }, 0)
        .unwrap();

    gateway.activate_snapshot(id, 0, 0).unwrap();
}

fn next_random(state: &mut u64) -> u64 {
    let mut value = *state;

    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;

    *state = value;

    value
}

fn replay(seed: u64, registration_order: &[u64]) -> ReplayResult {
    let mut gateway = TestGateway::new(config()).unwrap();
    let mut sink = RecordingSink::default();

    for value in registration_order {
        let filter = match value {
            1 => SubscriptionFilter::all(),

            2 => SubscriptionFilter::for_instruments(&[100], 16).unwrap(),

            3 => SubscriptionFilter::all().with_event_kinds(&[EventKind::Trade]),

            _ => SubscriptionFilter::none(),
        };

        register(&mut gateway, *value, filter);
    }

    let mut random_state = seed;

    for index in 1u64..=10_000 {
        let random = next_random(&mut random_state);

        let instrument = if random % 2 == 0 { 100 } else { 200 };

        let kind = if random % 3 == 0 {
            EventKind::Trade
        } else {
            EventKind::BookUpdate
        };

        gateway
            .publish(Some(instrument), kind, index * 1000, random)
            .unwrap();

        if index % 100 == 0 {
            for value in [1, 2, 3, 4] {
                gateway
                    .handle_request(SessionId::new(value), ClientRequest::Heartbeat, index)
                    .unwrap();
            }
        }

        if index % 32 == 0 {
            for value in [1, 2, 3, 4] {
                gateway
                    .service(SessionId::new(value), &mut sink, index)
                    .unwrap();
            }
        }
    }

    for value in [1, 2, 3, 4] {
        gateway
            .service(SessionId::new(value), &mut sink, 10_000)
            .unwrap();
    }

    ReplayResult {
        messages: sink.messages,
        sessions: gateway.sessions().cloned().collect(),
        gateway_stats: *gateway.stats(),
        publisher_stats: *gateway.publisher_stats(),
        watermark: gateway.publisher_watermark(),
    }
}

#[test]
fn identical_inputs_produce_identical_gateway_outputs() {
    let first = replay(0x1234_5678_9ABC_DEF0, &[1, 2, 3, 4]);

    let second = replay(0x1234_5678_9ABC_DEF0, &[1, 2, 3, 4]);

    assert_eq!(first, second);
    assert_eq!(first.watermark, 10_000);
}

#[test]
fn registration_order_does_not_affect_replay() {
    let first = replay(12345, &[1, 2, 3, 4]);
    let second = replay(12345, &[4, 3, 2, 1]);

    assert_eq!(first, second);
}

#[test]
fn different_inputs_produce_different_outputs() {
    let first = replay(12345, &[1, 2, 3, 4]);
    let second = replay(67890, &[1, 2, 3, 4]);

    assert_ne!(first.messages, second.messages);
}

#[test]
fn replay_preserves_per_session_publication_order() {
    let result = replay(12345, &[1, 2, 3, 4]);

    let mut last_sequences = [0u64; 5];

    for (id, message) in &result.messages {
        if let ServerMessage::Incremental(publication) = message {
            let index = id.get() as usize;

            assert!(publication.sequence > last_sequences[index]);

            last_sequences[index] = publication.sequence;
        }
    }

    assert_eq!(last_sequences[1], 10_000);
    assert_eq!(last_sequences[4], 0);
}

#[test]
fn replay_does_not_lose_accepted_deliveries() {
    let result = replay(12345, &[1, 2, 3, 4]);

    assert_eq!(result.publisher_stats.publications_accepted, 10_000);

    assert_eq!(result.publisher_stats.deliveries_discarded, 0);

    assert_eq!(result.publisher_stats.deliveries_missed, 0);

    assert_eq!(result.publisher_stats.pending_deliveries(), 0);

    assert_eq!(
        result.gateway_stats.incremental_messages_sent,
        result.publisher_stats.deliveries_enqueued
    );

    assert_eq!(result.gateway_stats.resync_transitions, 0);

    assert_eq!(result.gateway_stats.sessions_expired, 0);
}

#[test]
fn session_churn_respects_resource_limits() {
    let mut gateway = TestGateway::new(config()).unwrap();

    for index in 1u64..=10_000 {
        let id = SessionId::new(index);

        gateway.connect(id, index).unwrap();

        gateway
            .handle_request(
                id,
                ClientRequest::Subscribe {
                    filter: SubscriptionFilter::all(),
                },
                index,
            )
            .unwrap();

        gateway
            .activate_snapshot(id, gateway.publisher_watermark(), index)
            .unwrap();

        gateway
            .publish(Some(100), EventKind::Trade, index, index)
            .unwrap();

        gateway.disconnect(id).unwrap();

        assert_eq!(gateway.session_count(), 0);

        assert_eq!(gateway.publisher_stats().pending_deliveries(), 0);
    }

    let stats = gateway.stats();

    assert_eq!(stats.sessions_connected, 10_000);
    assert_eq!(stats.sessions_disconnected, 10_000);
    assert_eq!(stats.subscriptions_created, 10_000);
    assert_eq!(stats.subscriptions_removed, 10_000);
    assert_eq!(stats.session_high_watermark, 1);

    assert_eq!(gateway.publisher_stats().deliveries_discarded, 10_000);
}
