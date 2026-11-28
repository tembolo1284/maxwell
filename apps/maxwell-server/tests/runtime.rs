//! Integration tests for the UDP server runtime.
//!
//! A hand-driven producer socket plays the upstream role, so
//! every datagram the runtime sends and receives is visible
//! to the test.

use std::io;
use std::net::SocketAddr;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use maxwell_core::{
    EventEnvelope, EventHeader, EventId, FeedId, InstrumentId, MarketEvent, OrderAdded, OrderId,
    Price, Quantity, SequenceNumber, SequenceRange, SessionId, Side, StreamId,
};
use maxwell_protocol::constants::MAX_MESSAGE_SIZE;
use maxwell_protocol::{
    CONTROL_SIZE, Complete, ControlMessage, Encoder, Heartbeat, RetransmitRequest,
};
use maxwell_transport::{DatagramReceiver, DatagramSender, ReceiveBuffer, UdpConfig, UdpUnicast};

use maxwell_server::config::ServerConfig;
use maxwell_server::error::{ServerError, ServerResult};
use maxwell_server::runtime::{MarketSpec, RunSummary, Runtime, RuntimeConfig, StopReason};
use maxwell_server::state::ServerState;

// ============================================================
// Fixtures
// ============================================================

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(1))
}

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn market() -> MarketSpec {
    MarketSpec {
        instrument_count: 1,
        max_orders_per_instrument: 64,
        ..MarketSpec::default()
    }
}

fn add(feed: FeedId, sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventHeader::new(
            EventId::new(feed, SequenceNumber::new(sequence)),
            InstrumentId::new(1),
            None,
        ),
        MarketEvent::OrderAdded(OrderAdded {
            order_id: OrderId::new(sequence),
            side: Side::Buy,
            price: Price::new(1_000_000),
            quantity: Quantity::new(10),
        }),
    )
}

fn order_count(runtime: &Runtime) -> usize {
    runtime
        .server()
        .pipeline()
        .books()
        .book(InstrumentId::new(1))
        .unwrap()
        .order_count()
}

// ============================================================
// Hand-driven producer
// ============================================================

struct Producer {
    socket: UdpUnicast,
    buffer: ReceiveBuffer,
    encoder: Encoder,
    server: SocketAddr,
}

impl Producer {
    fn new(server: SocketAddr) -> Self {
        let mut socket = UdpUnicast::bind(loopback(), UdpConfig::default()).unwrap();

        socket.set_nonblocking(false).unwrap();

        socket
            .socket()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let buffer = socket.create_receive_buffer().unwrap();

        Self {
            socket,
            buffer,
            encoder: Encoder::new(),
            server,
        }
    }

    fn address(&self) -> SocketAddr {
        self.socket.local_addr().unwrap()
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        self.socket.send_to(bytes, self.server).unwrap();
    }

    fn send_event(&mut self, event: EventEnvelope) {
        let mut frame = [0u8; MAX_MESSAGE_SIZE];

        let length = self.encoder.encode_event(event, &mut frame).unwrap();

        self.send_raw(&frame[..length]);
    }

    fn send_control(&mut self, message: ControlMessage) {
        let mut frame = [0u8; CONTROL_SIZE];

        message.encode(&mut frame).unwrap();

        self.send_raw(&frame);
    }

    fn send_heartbeat(&mut self, last_sequence: u64, is_final: bool) {
        self.send_control(ControlMessage::Heartbeat(Heartbeat {
            feed: feed(),
            last_sequence: (last_sequence != 0).then(|| SequenceNumber::new(last_sequence)),
            is_final,
        }));
    }

    /// Waits for the next control datagram from the server.
    fn receive(&mut self) -> ControlMessage {
        self.socket
            .receive_into(&mut self.buffer)
            .unwrap()
            .expect("server sent nothing within the timeout");

        ControlMessage::decode(self.buffer.as_slice()).unwrap()
    }

    fn expect_request(&mut self, first: u64, last: u64) -> RetransmitRequest {
        let ControlMessage::RetransmitRequest(request) = self.receive() else {
            panic!("expected a retransmission request");
        };

        assert_eq!(request.feed, feed());

        assert_eq!(
            request.range,
            SequenceRange::new(SequenceNumber::new(first), SequenceNumber::new(last)).unwrap(),
        );

        request
    }
}

// ============================================================
// Server thread
// ============================================================

struct ServerThread {
    address: SocketAddr,
    handle: JoinHandle<(Runtime, ServerResult<RunSummary>)>,
}

impl ServerThread {
    fn start(configure: impl FnOnce(&mut RuntimeConfig)) -> Self {
        let mut config = RuntimeConfig::new(loopback(), feed());

        config.exit_on_final = true;
        config.idle_timeout = Some(Duration::from_secs(10));

        configure(&mut config);

        let mut server_config = ServerConfig::default();

        market().size_books(&mut server_config).unwrap();

        let mut runtime =
            Runtime::bind(config, server_config, &market().instruments().unwrap()).unwrap();

        let address = runtime.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let result = runtime.run(&mut io::sink());

            (runtime, result)
        });

        Self { address, handle }
    }

    fn join(self) -> (Runtime, ServerResult<RunSummary>) {
        self.handle.join().unwrap()
    }
}

// ============================================================
// Ordinary delivery
// ============================================================

#[test]
fn contiguous_feed_completes_and_notifies_producer() {
    let server = ServerThread::start(|_| {});
    let mut producer = Producer::new(server.address);

    for sequence in 1..=5 {
        producer.send_event(add(feed(), sequence));
    }

    producer.send_heartbeat(5, true);

    assert_eq!(
        producer.receive(),
        ControlMessage::Complete(Complete {
            feed: feed(),
            last_sequence: SequenceNumber::new(5),
        }),
    );

    let (runtime, result) = server.join();
    let summary = result.unwrap();

    assert_eq!(summary.reason, StopReason::FeedComplete);
    assert!(summary.synchronized);
    assert_eq!(summary.next_expected, SequenceNumber::new(6));
    assert_eq!(summary.publication_watermark, 5);

    assert_eq!(summary.stats.datagrams, 6);
    assert_eq!(summary.stats.events, 5);
    assert_eq!(summary.stats.heartbeats, 1);
    assert_eq!(summary.stats.retransmit_requests, 0);

    assert_eq!(order_count(&runtime), 5);
    assert_eq!(runtime.server().state(), ServerState::Stopped);
}

#[test]
fn empty_final_feed_completes() {
    let server = ServerThread::start(|_| {});
    let mut producer = Producer::new(server.address);

    producer.send_heartbeat(0, true);

    let (runtime, result) = server.join();
    let summary = result.unwrap();

    assert_eq!(summary.reason, StopReason::FeedComplete);
    assert_eq!(summary.next_expected, SequenceNumber::ONE);
    assert_eq!(order_count(&runtime), 0);
}

#[test]
fn idle_feed_stops_after_timeout() {
    let server = ServerThread::start(|config| {
        config.idle_timeout = Some(Duration::from_millis(100));
    });

    let mut producer = Producer::new(server.address);

    producer.send_event(add(feed(), 1));

    let (runtime, result) = server.join();
    let summary = result.unwrap();

    assert_eq!(summary.reason, StopReason::Idle);
    assert_eq!(summary.stats.events, 1);
    assert_eq!(order_count(&runtime), 1);
}

// ============================================================
// Recovery
// ============================================================

#[test]
fn gap_is_requested_and_healed() {
    let server = ServerThread::start(|_| {});
    let mut producer = Producer::new(server.address);

    producer.send_event(add(feed(), 1));
    producer.send_event(add(feed(), 4));

    producer.expect_request(2, 3);

    producer.send_event(add(feed(), 2));
    producer.send_event(add(feed(), 3));
    producer.send_heartbeat(4, true);

    assert!(matches!(producer.receive(), ControlMessage::Complete(_)));

    let (runtime, result) = server.join();
    let summary = result.unwrap();

    assert_eq!(summary.next_expected, SequenceNumber::new(5));
    assert_eq!(summary.stats.buffered, 1);
    assert_eq!(summary.stats.retransmit_requests, 1);
    assert_eq!(summary.stats.resynchronizations, 1);
    assert_eq!(order_count(&runtime), 4);
}

#[test]
fn heartbeat_reveals_trailing_loss() {
    let server = ServerThread::start(|_| {});
    let mut producer = Producer::new(server.address);

    for sequence in 1..=3 {
        producer.send_event(add(feed(), sequence));
    }

    // Events four and five were lost. No later event exists
    // to expose the gap, so only the watermark can.
    producer.send_heartbeat(5, true);

    producer.expect_request(4, 5);

    producer.send_event(add(feed(), 4));
    producer.send_event(add(feed(), 5));

    assert!(matches!(producer.receive(), ControlMessage::Complete(_)));

    let (runtime, result) = server.join();
    let summary = result.unwrap();

    assert_eq!(summary.reason, StopReason::FeedComplete);
    assert_eq!(summary.next_expected, SequenceNumber::new(6));
    assert_eq!(order_count(&runtime), 5);
}

#[test]
fn unanswered_request_is_retried_then_fails() {
    let server = ServerThread::start(|_| {});
    let mut producer = Producer::new(server.address);

    producer.send_event(add(feed(), 3));

    // Every attempt asks for the same missing range.
    let first = producer.expect_request(1, 2);
    let second = producer.expect_request(1, 2);

    assert_ne!(first.request_id, second.request_id);

    // The datagram feed has no snapshot source, so exhausting
    // retransmission stops the runtime.
    let (runtime, result) = server.join();

    assert!(matches!(result, Err(ServerError::RecoveryUnavailable)));
    assert!(!runtime.server().is_synchronized());
    assert_eq!(order_count(&runtime), 0);
    assert!(runtime.stats().retransmit_requests >= 2);
}

// ============================================================
// Invalid input
// ============================================================

#[test]
fn malformed_datagrams_are_counted_and_skipped() {
    let server = ServerThread::start(|_| {});
    let mut producer = Producer::new(server.address);

    producer.send_event(add(feed(), 1));

    // Not a protocol message at all.
    producer.send_raw(b"garbage");

    // Control magic with an invalid length.
    producer.send_raw(b"MC short");

    // Valid event for a different feed.
    let other = FeedId::new(StreamId::new(9), SessionId::new(9));
    producer.send_event(add(other, 2));

    // Heartbeat for a different feed.
    producer.send_control(ControlMessage::Heartbeat(Heartbeat {
        feed: other,
        last_sequence: Some(SequenceNumber::new(100)),
        is_final: true,
    }));

    // A control kind only the server sends.
    producer.send_control(ControlMessage::Complete(Complete {
        feed: feed(),
        last_sequence: SequenceNumber::ONE,
    }));

    producer.send_event(add(feed(), 2));
    producer.send_heartbeat(2, true);

    let (runtime, result) = server.join();
    let summary = result.unwrap();

    assert_eq!(summary.reason, StopReason::FeedComplete);
    assert_eq!(summary.stats.datagrams, 8);
    assert_eq!(summary.stats.events, 2);
    assert_eq!(summary.stats.rejected, 5);
    assert_eq!(summary.stats.heartbeats, 1);
    assert_eq!(summary.stats.retransmit_requests, 0);
    assert_eq!(order_count(&runtime), 2);
}

#[test]
fn datagrams_from_other_sources_are_ignored() {
    // The configured producer address is reserved first, so
    // the server can be told which source to trust.
    let trusted_socket = UdpUnicast::bind(loopback(), UdpConfig::default()).unwrap();
    let trusted_address = trusted_socket.local_addr().unwrap();

    drop(trusted_socket);

    let server = ServerThread::start(|config| {
        config.producer = Some(trusted_address);
        config.idle_timeout = Some(Duration::from_millis(200));
    });

    let mut stranger = Producer::new(server.address);

    assert_ne!(stranger.address(), trusted_address);

    stranger.send_event(add(feed(), 1));
    stranger.send_heartbeat(1, true);

    let (runtime, result) = server.join();
    let summary = result.unwrap();

    assert_eq!(summary.reason, StopReason::Idle);
    assert_eq!(summary.stats.datagrams, 2);
    assert_eq!(summary.stats.foreign, 2);
    assert_eq!(summary.stats.events, 0);
    assert_eq!(summary.next_expected, SequenceNumber::ONE);
    assert_eq!(order_count(&runtime), 0);
}
