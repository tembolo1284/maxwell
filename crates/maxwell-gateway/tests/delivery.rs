//! Integration tests for incremental delivery and backpressure.

use maxwell_gateway::{
    ClientRequest, Gateway, GatewayConfig, GatewaySink, ServerMessage, SessionId, SessionState,
};

use maxwell_publisher::{BackpressurePolicy, EventKind, PublisherConfig, SubscriptionFilter};

type TestGateway = Gateway<u32, u64>;
type TestMessage = ServerMessage<u32, u64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkError {
    WouldBlock,
}

#[derive(Default)]
struct RecordingSink {
    messages: Vec<(SessionId, TestMessage)>,
    capacity: usize,
}

impl RecordingSink {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            messages: Vec::new(),
            capacity,
        }
    }
}

impl GatewaySink<u32, u64> for RecordingSink {
    type Error = SinkError;

    fn try_send(
        &mut self,
        session_id: SessionId,
        message: &TestMessage,
    ) -> Result<(), Self::Error> {
        if self.messages.len() >= self.capacity {
            return Err(SinkError::WouldBlock);
        }

        self.messages.push((session_id, *message));

        Ok(())
    }
}

fn gateway(queue_capacity: usize, budget: usize) -> TestGateway {
    Gateway::new(GatewayConfig {
        publisher: PublisherConfig {
            max_subscribers: 8,
            queue_capacity,
            max_instruments_per_subscription: 16,
            backpressure_policy: BackpressurePolicy::RequireResync,
        },
        heartbeat_interval_ns: 50,
        session_timeout_ns: 150,
        max_messages_per_service: budget,
    })
    .unwrap()
}

fn connect(gateway: &mut TestGateway, value: u64, filter: SubscriptionFilter<u32>) -> SessionId {
    let id = SessionId::new(value);

    gateway.connect(id, 100).unwrap();

    gateway
        .handle_request(id, ClientRequest::Subscribe { filter }, 100)
        .unwrap();

    gateway
        .activate_snapshot(id, gateway.publisher_watermark(), 100)
        .unwrap();

    id
}

fn publish(gateway: &mut TestGateway, value: u64) {
    gateway
        .publish(Some(100), EventKind::BookUpdate, value * 1000, value)
        .unwrap();
}

#[test]
fn delivers_incremental_publications_in_order() {
    let mut gateway = gateway(16, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    for value in 1..=10 {
        publish(&mut gateway, value);
    }

    let mut sink = RecordingSink::with_capacity(16);

    let report = gateway.service(id, &mut sink, 110).unwrap();

    assert_eq!(report.incremental_sent, 10);
    assert_eq!(report.remaining, 0);
    assert_eq!(report.control_sent, 0);
    assert!(!report.is_blocked());

    for (index, (session_id, message)) in sink.messages.iter().enumerate() {
        assert_eq!(*session_id, id);

        match message {
            ServerMessage::Incremental(publication) => {
                assert_eq!(publication.sequence, index as u64 + 1);

                assert_eq!(publication.payload, index as u64 + 1);
            }

            _ => panic!("unexpected control message"),
        }
    }
}

#[test]
fn enforces_service_budget() {
    let mut gateway = gateway(16, 3);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    for value in 1..=8 {
        publish(&mut gateway, value);
    }

    let mut sink = RecordingSink::with_capacity(16);

    let first = gateway.service(id, &mut sink, 110).unwrap();
    let second = gateway.service(id, &mut sink, 110).unwrap();
    let third = gateway.service(id, &mut sink, 110).unwrap();

    assert_eq!(first.incremental_sent, 3);
    assert_eq!(first.remaining, 5);

    assert_eq!(second.incremental_sent, 3);
    assert_eq!(second.remaining, 2);

    assert_eq!(third.incremental_sent, 2);
    assert_eq!(third.remaining, 0);

    assert_eq!(sink.messages.len(), 8);
}

#[test]
fn sink_failure_preserves_pending_publications() {
    let mut gateway = gateway(16, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    for value in 1..=5 {
        publish(&mut gateway, value);
    }

    let mut sink = RecordingSink::with_capacity(2);

    let first = gateway.service(id, &mut sink, 110).unwrap();

    assert_eq!(first.incremental_sent, 2);
    assert_eq!(first.remaining, 3);
    assert_eq!(first.error, Some(SinkError::WouldBlock));

    sink.capacity = 5;

    let second = gateway.service(id, &mut sink, 120).unwrap();

    assert_eq!(second.incremental_sent, 3);
    assert_eq!(second.remaining, 0);
    assert!(!second.is_blocked());

    let sequences: Vec<_> = sink
        .messages
        .iter()
        .filter_map(|(_, message)| match message {
            ServerMessage::Incremental(publication) => Some(publication.sequence),

            _ => None,
        })
        .collect();

    assert_eq!(sequences, vec![1, 2, 3, 4, 5]);
    assert_eq!(gateway.stats().sink_failures, 1);
}

#[test]
fn publisher_overflow_transitions_session_to_resync() {
    let mut gateway = gateway(2, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    publish(&mut gateway, 1);
    publish(&mut gateway, 2);
    publish(&mut gateway, 3);

    assert_eq!(
        gateway.session(id).unwrap().state(),
        SessionState::ResyncRequired
    );

    assert_eq!(gateway.stats().resync_transitions, 1);

    let mut sink = RecordingSink::with_capacity(16);

    let report = gateway.service(id, &mut sink, 110).unwrap();

    assert_eq!(report.incremental_sent, 0);
    assert_eq!(report.control_sent, 1);

    assert_eq!(
        sink.messages,
        vec![(
            id,
            ServerMessage::ResyncRequired {
                publisher_watermark: 3,
            },
        )]
    );
}

#[test]
fn resync_notification_is_retried_until_accepted() {
    let mut gateway = gateway(1, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    publish(&mut gateway, 1);
    publish(&mut gateway, 2);

    let mut sink = RecordingSink::with_capacity(0);

    let report = gateway.service(id, &mut sink, 110).unwrap();

    assert!(report.is_blocked());

    assert!(!gateway.session(id).unwrap().resync_notified());

    sink.capacity = 10;

    let report = gateway.service(id, &mut sink, 120).unwrap();

    assert_eq!(report.control_sent, 1);

    assert!(gateway.session(id).unwrap().resync_notified());

    let report = gateway.service(id, &mut sink, 130).unwrap();

    assert_eq!(report.control_sent, 0);
    assert_eq!(gateway.stats().resync_notifications_sent, 1);
}

#[test]
fn healthy_session_continues_after_slow_session_overflow() {
    let mut gateway = gateway(1, 16);

    let slow = connect(&mut gateway, 1, SubscriptionFilter::all());

    let fast = connect(&mut gateway, 2, SubscriptionFilter::all());

    let mut sink = RecordingSink::with_capacity(16);

    publish(&mut gateway, 1);

    gateway.service(fast, &mut sink, 110).unwrap();

    publish(&mut gateway, 2);

    assert_eq!(
        gateway.session(slow).unwrap().state(),
        SessionState::ResyncRequired
    );

    assert!(gateway.session(fast).unwrap().is_active());

    let report = gateway.service(fast, &mut sink, 120).unwrap();

    assert_eq!(report.incremental_sent, 1);
    assert_eq!(gateway.stats().incremental_messages_sent, 2);
}

#[test]
fn snapshot_activation_restores_incremental_delivery() {
    let mut gateway = gateway(1, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    publish(&mut gateway, 1);
    publish(&mut gateway, 2);

    let mut sink = RecordingSink::with_capacity(16);

    gateway.service(id, &mut sink, 110).unwrap();

    // The application establishes a new snapshot at watermark 2.
    gateway.activate_snapshot(id, 2, 120).unwrap();

    publish(&mut gateway, 3);

    let report = gateway.service(id, &mut sink, 130).unwrap();

    assert_eq!(report.incremental_sent, 1);
    assert!(gateway.session(id).unwrap().is_active());

    match &sink.messages.last().unwrap().1 {
        ServerMessage::Incremental(publication) => {
            assert_eq!(publication.sequence, 3);
            assert_eq!(publication.payload, 3);
        }

        _ => panic!("expected incremental publication"),
    }
}

#[test]
fn client_can_request_resynchronization() {
    let mut gateway = gateway(8, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    publish(&mut gateway, 1);

    gateway
        .handle_request(id, ClientRequest::RequestResync, 110)
        .unwrap();

    assert_eq!(
        gateway.session(id).unwrap().state(),
        SessionState::ResyncRequired
    );

    assert_eq!(gateway.publisher_stats().pending_deliveries(), 0);
}

#[test]
fn idle_session_receives_heartbeat() {
    let mut gateway = gateway(8, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    let mut sink = RecordingSink::with_capacity(16);

    let first = gateway.service(id, &mut sink, 149).unwrap();

    assert_eq!(first.total_sent(), 0);

    let second = gateway.service(id, &mut sink, 150).unwrap();

    assert_eq!(second.control_sent, 1);

    assert_eq!(
        sink.messages,
        vec![(
            id,
            ServerMessage::Heartbeat {
                timestamp_ns: 150,
                publisher_watermark: 0,
            },
        )]
    );
}

#[test]
fn incremental_delivery_suppresses_idle_heartbeat() {
    let mut gateway = gateway(8, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    publish(&mut gateway, 1);

    let mut sink = RecordingSink::with_capacity(16);

    let report = gateway.service(id, &mut sink, 150).unwrap();

    assert_eq!(report.incremental_sent, 1);
    assert_eq!(report.control_sent, 0);
    assert_eq!(gateway.stats().heartbeats_sent, 0);
}

#[test]
fn expired_session_cannot_deliver_pending_data() {
    let mut gateway = gateway(8, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    publish(&mut gateway, 1);

    let mut sink = RecordingSink::with_capacity(16);

    let report = gateway.service(id, &mut sink, 250).unwrap();

    assert!(report.expired);
    assert_eq!(report.total_sent(), 0);
    assert!(sink.messages.is_empty());
}

#[test]
fn failed_heartbeat_is_retried() {
    let mut gateway = gateway(8, 16);

    let id = connect(&mut gateway, 1, SubscriptionFilter::all());

    let mut sink = RecordingSink::with_capacity(0);

    let first = gateway.service(id, &mut sink, 150).unwrap();

    assert!(first.is_blocked());
    assert_eq!(gateway.stats().heartbeats_sent, 0);

    sink.capacity = 1;

    let second = gateway.service(id, &mut sink, 151).unwrap();

    assert_eq!(second.control_sent, 1);
    assert_eq!(gateway.stats().heartbeats_sent, 1);
}
