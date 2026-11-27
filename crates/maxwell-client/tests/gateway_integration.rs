//! End-to-end logical gateway-to-client integration tests.

use maxwell_client::{
    Client, ClientConfig, ClientState, InboundFrame, InboundMessage, MarketDataHandler,
    RecoveryReason,
};

use maxwell_gateway::{
    ClientRequest, Gateway, GatewayConfig, GatewaySink, ServerMessage, SessionId,
};

use maxwell_publisher::{
    BackpressurePolicy, BoundedQueue, EventKind, Publication, PublisherConfig, SubscriptionFilter,
};

type TestGateway = Gateway<u32, u64>;
type TestClient = Client<u32, u64, u64, RecordingHandler>;
type TestFrame = InboundFrame<u32, u64, u64>;
type TestMessage = InboundMessage<u32, u64, u64>;

#[derive(Debug, Default, PartialEq, Eq)]
struct RecordingHandler {
    valid: bool,
    snapshot: Vec<u64>,
    publications: Vec<Publication<u32, u64>>,
    invalidations: Vec<RecoveryReason>,
}

impl MarketDataHandler<u32, u64, u64> for RecordingHandler {
    type Error = ();

    fn install_snapshot(&mut self, _watermark: u64, entries: &[u64]) -> Result<(), Self::Error> {
        self.snapshot = entries.to_vec();
        self.publications.clear();
        self.valid = true;

        Ok(())
    }

    fn apply_incremental(
        &mut self,
        publication: &Publication<u32, u64>,
    ) -> Result<(), Self::Error> {
        if !self.valid {
            return Err(());
        }

        self.publications.push(*publication);

        Ok(())
    }

    fn on_invalidated(&mut self, reason: RecoveryReason) {
        self.valid = false;
        self.invalidations.push(reason);
    }

    fn on_disconnected(&mut self) {
        self.valid = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportError {
    WouldBlock,
    WrongSession,
    SequenceExhausted,
}

struct TestTransport {
    session_id: SessionId,
    last_sequence: u64,
    frames: BoundedQueue<TestFrame>,
}

impl TestTransport {
    fn new(session_id: SessionId, capacity: usize) -> Self {
        Self {
            session_id,
            last_sequence: 0,
            frames: BoundedQueue::new(capacity).unwrap(),
        }
    }

    fn enqueue(&mut self, message: TestMessage) -> Result<(), TransportError> {
        let sequence = self
            .last_sequence
            .checked_add(1)
            .ok_or(TransportError::SequenceExhausted)?;

        let frame = InboundFrame::new(sequence, message);

        self.frames
            .try_push(frame)
            .map_err(|_| TransportError::WouldBlock)?;

        self.last_sequence = sequence;

        Ok(())
    }

    fn deliver(&mut self, client: &mut TestClient, now_ns: u64) -> usize {
        let mut delivered = 0;

        while let Some(frame) = self.frames.pop() {
            client.receive(frame, now_ns).unwrap();
            delivered += 1;
        }

        delivered
    }
}

impl GatewaySink<u32, u64> for TestTransport {
    type Error = TransportError;

    fn try_send(
        &mut self,
        session_id: SessionId,
        message: &ServerMessage<u32, u64>,
    ) -> Result<(), Self::Error> {
        if session_id != self.session_id {
            return Err(TransportError::WrongSession);
        }

        self.enqueue(InboundMessage::Gateway(*message))
    }
}

fn gateway(queue_capacity: usize) -> TestGateway {
    Gateway::new(GatewayConfig {
        publisher: PublisherConfig {
            max_subscribers: 8,
            queue_capacity,
            max_instruments_per_subscription: 16,
            backpressure_policy: BackpressurePolicy::RequireResync,
        },
        heartbeat_interval_ns: 50,
        session_timeout_ns: 150,
        max_messages_per_service: 64,
    })
    .unwrap()
}

fn client() -> TestClient {
    Client::new(
        ClientConfig {
            heartbeat_interval_ns: 50,
            server_timeout_ns: 150,
            max_snapshot_entries: 64,
        },
        RecordingHandler::default(),
    )
    .unwrap()
}

fn connect(
    gateway: &mut TestGateway,
    client: &mut TestClient,
    id: SessionId,
    filter: SubscriptionFilter<u32>,
) {
    gateway.connect(id, 0).unwrap();
    client.connect(0).unwrap();

    gateway
        .handle_request(id, ClientRequest::Subscribe { filter }, 0)
        .unwrap();

    client.record_outbound(0).unwrap();
}

/// Simulates the server application's snapshot transaction.
///
/// The complete snapshot is accepted into the same ordered
/// transport used for subsequent incremental publications.
fn synchronize(
    gateway: &mut TestGateway,
    client: &mut TestClient,
    transport: &mut TestTransport,
    entries: &[u64],
    now_ns: u64,
) {
    let watermark = gateway.publisher_watermark();

    assert!(transport.frames.remaining_capacity() >= entries.len() + 2);

    transport
        .enqueue(InboundMessage::SnapshotBegin {
            publisher_watermark: watermark,
            entry_count: entries.len(),
        })
        .unwrap();

    for entry in entries {
        transport
            .enqueue(InboundMessage::SnapshotEntry(*entry))
            .unwrap();
    }

    transport
        .enqueue(InboundMessage::SnapshotEnd {
            publisher_watermark: watermark,
        })
        .unwrap();

    gateway
        .activate_snapshot(transport.session_id, watermark, now_ns)
        .unwrap();

    transport.deliver(client, now_ns);
}

#[test]
fn gateway_to_client_publication_pipeline() {
    let mut gateway = gateway(16);
    let mut client = client();

    let id = SessionId::new(1);
    let mut transport = TestTransport::new(id, 32);

    connect(&mut gateway, &mut client, id, SubscriptionFilter::all());

    synchronize(
        &mut gateway,
        &mut client,
        &mut transport,
        &[100, 200, 300],
        0,
    );

    for value in 1u64..=10 {
        gateway
            .publish(Some(100), EventKind::BookUpdate, value * 1000, value)
            .unwrap();
    }

    let report = gateway.service(id, &mut transport, 10).unwrap();

    assert_eq!(report.incremental_sent, 10);
    assert_eq!(transport.deliver(&mut client, 10), 10);

    assert!(client.is_active());
    assert_eq!(client.handler().snapshot, vec![100, 200, 300]);
    assert_eq!(client.handler().publications.len(), 10);
    assert_eq!(client.publisher_watermark(), Some(10));

    for (index, publication) in client.handler().publications.iter().enumerate() {
        assert_eq!(publication.sequence, index as u64 + 1);
        assert_eq!(publication.payload, index as u64 + 1);
    }
}

#[test]
fn filtered_gateway_stream_remains_contiguous_at_transport_layer() {
    let mut gateway = gateway(16);
    let mut client = client();

    let id = SessionId::new(1);
    let mut transport = TestTransport::new(id, 32);

    connect(
        &mut gateway,
        &mut client,
        id,
        SubscriptionFilter::for_instruments(&[100], 16).unwrap(),
    );

    synchronize(&mut gateway, &mut client, &mut transport, &[], 0);

    for sequence in 1u64..=10 {
        let instrument = if sequence % 2 == 0 { 100 } else { 200 };

        gateway
            .publish(Some(instrument), EventKind::Trade, sequence, sequence)
            .unwrap();
    }

    gateway.service(id, &mut transport, 10).unwrap();
    transport.deliver(&mut client, 10);

    let sequences: Vec<_> = client
        .handler()
        .publications
        .iter()
        .map(|publication| publication.sequence)
        .collect();

    assert_eq!(sequences, vec![2, 4, 6, 8, 10]);
    assert_eq!(client.stats().transport_gaps, 0);
    assert_eq!(client.last_transport_sequence(), 7);
    assert!(client.is_active());
}

#[test]
fn bounded_transport_backpressure_preserves_sequences() {
    let mut gateway = gateway(16);
    let mut client = client();

    let id = SessionId::new(1);
    let mut transport = TestTransport::new(id, 2);

    connect(&mut gateway, &mut client, id, SubscriptionFilter::all());

    synchronize(&mut gateway, &mut client, &mut transport, &[], 0);

    for value in 1u64..=5 {
        gateway
            .publish(Some(100), EventKind::Trade, value, value)
            .unwrap();
    }

    let first = gateway.service(id, &mut transport, 10).unwrap();

    assert_eq!(first.incremental_sent, 2);
    assert_eq!(first.remaining, 3);
    assert_eq!(first.error, Some(TransportError::WouldBlock));

    transport.deliver(&mut client, 10);

    let second = gateway.service(id, &mut transport, 20).unwrap();

    assert_eq!(second.incremental_sent, 2);
    assert_eq!(second.remaining, 1);

    transport.deliver(&mut client, 20);

    let third = gateway.service(id, &mut transport, 30).unwrap();

    assert_eq!(third.incremental_sent, 1);
    assert_eq!(third.remaining, 0);

    transport.deliver(&mut client, 30);

    assert_eq!(client.stats().incrementals_applied, 5);
    assert_eq!(client.stats().transport_gaps, 0);
    assert_eq!(client.last_transport_sequence(), 7);
    assert!(client.is_active());
}

#[test]
fn publisher_overflow_recovers_through_new_snapshot() {
    let mut gateway = gateway(2);
    let mut client = client();

    let id = SessionId::new(1);
    let mut transport = TestTransport::new(id, 16);

    connect(&mut gateway, &mut client, id, SubscriptionFilter::all());

    synchronize(&mut gateway, &mut client, &mut transport, &[10], 0);

    for value in 1u64..=3 {
        gateway
            .publish(Some(100), EventKind::BookUpdate, value, value)
            .unwrap();
    }

    gateway.service(id, &mut transport, 10).unwrap();
    transport.deliver(&mut client, 10);

    assert_eq!(client.state(), ClientState::RecoveryRequired);

    assert!(!client.handler().valid);

    let request = client.pending_recovery_request().unwrap();

    gateway.handle_request(id, request, 20).unwrap();
    client.mark_recovery_request_sent(20).unwrap();

    synchronize(&mut gateway, &mut client, &mut transport, &[30], 20);

    gateway
        .publish(Some(100), EventKind::BookUpdate, 4, 4)
        .unwrap();

    gateway.service(id, &mut transport, 30).unwrap();
    transport.deliver(&mut client, 30);

    assert!(client.is_active());
    assert_eq!(client.publisher_watermark(), Some(4));
    assert_eq!(client.handler().snapshot, vec![30]);
    assert_eq!(client.handler().publications.len(), 1);
    assert_eq!(client.stats().snapshots_completed, 2);
}

#[test]
fn bidirectional_heartbeats_preserve_session() {
    let mut gateway = gateway(16);
    let mut client = client();

    let id = SessionId::new(1);
    let mut transport = TestTransport::new(id, 16);

    connect(&mut gateway, &mut client, id, SubscriptionFilter::all());

    synchronize(&mut gateway, &mut client, &mut transport, &[], 0);

    for now in (50u64..=1000).step_by(50) {
        assert!(!client.tick(now).unwrap());

        if let Some(request) = client.pending_heartbeat() {
            gateway.handle_request(id, request, now).unwrap();
            client.mark_heartbeat_sent(now).unwrap();
        }

        gateway.service(id, &mut transport, now).unwrap();
        transport.deliver(&mut client, now);
    }

    assert!(client.is_active());
    assert!(gateway.session(id).unwrap().is_active());

    assert_eq!(client.stats().heartbeats_sent, 20);
    assert_eq!(client.stats().heartbeats_received, 20);
    assert_eq!(gateway.stats().sessions_expired, 0);
    assert_eq!(client.stats().connection_timeouts, 0);
}
