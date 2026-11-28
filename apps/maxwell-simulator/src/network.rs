//! UDP delivery for the simulator.
//!
//! The network runner sends each simulated event as one native
//! Maxwell datagram. On the same socket it answers
//! retransmission requests from bounded history and announces
//! its sequence watermark with heartbeats.
//!
//! Everything runs on the calling thread and the socket is
//! nonblocking. A datagram the socket cannot take is reported
//! to the simulator as backpressure and offered again later.
//!
//! Delivery faults configured for the simulator apply to the
//! live stream only. Retransmitted events bypass them.

use std::net::SocketAddr;
use std::thread;
use std::time::{Duration, Instant};

use maxwell_core::{EventEnvelope, FeedId, MonotonicTimestamp, SequenceNumber};
use maxwell_protocol::constants::MAX_MESSAGE_SIZE;
use maxwell_protocol::{CONTROL_SIZE, ControlMessage, Encoder, Heartbeat, RetransmitRequest};
use maxwell_transport::{
    DatagramReceiver, DatagramSender, ReceiveBuffer, UdpConfig, UdpUnicast, WriteOutcome,
};

use crate::error::{SimulatorError, SimulatorResult};
use crate::simulator::{DeliveryOutcome, EventSink, Simulator, SimulatorState};

// ============================================================
// Defaults
// ============================================================

/// Interval between watermark heartbeats.
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(50);

/// Wait for further requests after the last event.
pub const DEFAULT_LINGER: Duration = Duration::from_secs(2);

/// Most control datagrams read per iteration.
pub const DEFAULT_REQUEST_BUDGET: usize = 16;

/// Pause taken when an iteration makes no progress.
const IDLE_PAUSE: Duration = Duration::from_millis(1);

// ============================================================
// Configuration
// ============================================================

/// Network delivery configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkConfig {
    /// Local address the producer socket binds to.
    pub bind: SocketAddr,

    /// Consumer address that live events are sent to.
    pub target: SocketAddr,

    pub udp: UdpConfig,
    pub heartbeat_interval: Duration,

    /// How long to wait for further requests once the last
    /// event has been sent.
    ///
    /// Each received request restarts the wait. The runner
    /// returns sooner if the consumer reports that it has
    /// applied the complete stream.
    pub linger: Duration,

    pub request_budget: usize,
}

impl NetworkConfig {
    /// Creates a configuration with default tuning.
    #[must_use]
    pub fn new(bind: SocketAddr, target: SocketAddr) -> Self {
        Self {
            bind,
            target,
            udp: UdpConfig::default(),
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
            linger: DEFAULT_LINGER,
            request_budget: DEFAULT_REQUEST_BUDGET,
        }
    }

    /// Validates the delivery parameters.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfig` for a zero interval or budget.
    pub fn validate(&self) -> SimulatorResult<()> {
        if self.heartbeat_interval.is_zero() {
            return Err(SimulatorError::InvalidConfig(
                "heartbeat interval must be positive",
            ));
        }

        if self.request_budget == 0 {
            return Err(SimulatorError::InvalidConfig(
                "request budget must be positive",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Statistics
// ============================================================

/// Cumulative network delivery counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NetworkStats {
    /// Live event datagrams sent.
    pub events_sent: u64,

    /// Live sends refused by the socket and retried later.
    pub send_blocked: u64,

    /// Heartbeats sent.
    pub heartbeats_sent: u64,

    /// Retransmission requests received for this feed.
    pub requests_received: u64,

    /// Requests for events no longer held in history.
    pub requests_unavailable: u64,

    /// Event datagrams sent in answer to requests.
    pub events_retransmitted: u64,

    /// Inbound datagrams that were not usable requests.
    pub rejected: u64,

    /// The consumer reported the complete stream applied.
    pub consumer_complete: bool,
}

// ============================================================
// Datagram sink
// ============================================================

/// Event sink that sends one datagram per event.
pub struct UdpEventSink {
    socket: UdpUnicast,
    target: SocketAddr,
    encoder: Encoder,
    frame: [u8; MAX_MESSAGE_SIZE],
    sent: u64,
    blocked: u64,
}

impl UdpEventSink {
    /// Binds a nonblocking producer socket.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket cannot be bound or
    /// switched to nonblocking mode.
    pub fn bind(bind: SocketAddr, target: SocketAddr, udp: UdpConfig) -> SimulatorResult<Self> {
        let mut socket = UdpUnicast::bind(bind, udp)?;

        socket.set_nonblocking(true)?;

        Ok(Self {
            socket,
            target,
            encoder: Encoder::new(),
            frame: [0; MAX_MESSAGE_SIZE],
            sent: 0,
            blocked: 0,
        })
    }

    /// Returns the address the socket is bound to.
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system cannot report it.
    pub fn local_addr(&self) -> SimulatorResult<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    /// Encodes and sends one event to `destination`.
    ///
    /// Returns whether the socket accepted the datagram.
    fn send_event(
        &mut self,
        event: &EventEnvelope,
        destination: SocketAddr,
    ) -> SimulatorResult<bool> {
        let length = self.encoder.encode_event(*event, &mut self.frame)?;

        let outcome = self.socket.send_to(&self.frame[..length], destination)?;

        Ok(matches!(outcome, WriteOutcome::Written(_)))
    }

    fn send_control(
        &mut self,
        message: ControlMessage,
        destination: SocketAddr,
    ) -> SimulatorResult<bool> {
        let mut frame = [0u8; CONTROL_SIZE];

        let length = message.encode(&mut frame)?;

        let outcome = self.socket.send_to(&frame[..length], destination)?;

        Ok(matches!(outcome, WriteOutcome::Written(_)))
    }
}

impl EventSink for UdpEventSink {
    fn try_send(
        &mut self,
        event: &EventEnvelope,
        _received_at: MonotonicTimestamp,
    ) -> SimulatorResult<DeliveryOutcome> {
        if self.send_event(event, self.target)? {
            self.sent += 1;

            Ok(DeliveryOutcome::Accepted)
        } else {
            self.blocked += 1;

            Ok(DeliveryOutcome::Blocked)
        }
    }
}

// ============================================================
// Network runner
// ============================================================

/// Drives a simulator over a UDP socket.
pub struct NetworkRunner {
    config: NetworkConfig,
    feed: FeedId,
    sink: UdpEventSink,
    inbound: ReceiveBuffer,
    advertised: Option<SequenceNumber>,
    stats: NetworkStats,
}

impl NetworkRunner {
    /// Binds the producer socket for `feed`.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid or
    /// the socket cannot be bound.
    pub fn bind(config: NetworkConfig, feed: FeedId) -> SimulatorResult<Self> {
        config.validate()?;

        let sink = UdpEventSink::bind(config.bind, config.target, config.udp)?;
        let inbound = sink.socket.create_receive_buffer()?;

        Ok(Self {
            config,
            feed,
            sink,
            inbound,
            advertised: None,
            stats: NetworkStats::default(),
        })
    }

    /// Returns the address the producer socket is bound to.
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system cannot report it.
    pub fn local_addr(&self) -> SimulatorResult<SocketAddr> {
        self.sink.local_addr()
    }

    #[must_use]
    pub const fn stats(&self) -> NetworkStats {
        let mut stats = self.stats;

        stats.events_sent = self.sink.sent;
        stats.send_blocked = self.sink.blocked;

        stats
    }

    // --------------------------------------------------------
    // Heartbeats
    // --------------------------------------------------------

    /// Advances the advertised watermark when it is safe.
    ///
    /// The generated watermark may run ahead of delivery while
    /// events wait in the outbound or delayed queues. Announcing
    /// it then would make the consumer request events that are
    /// about to arrive. The watermark is therefore advanced only
    /// when nothing generated is still waiting to be sent.
    fn refresh_watermark(&mut self, simulator: &Simulator) {
        if simulator.generated() != 0
            && simulator.pending_delivery() == 0
            && simulator.pending_delayed() == 0
        {
            self.advertised = Some(simulator.watermark());
        }
    }

    fn send_heartbeat(&mut self, simulator: &Simulator) -> SimulatorResult<()> {
        let message = ControlMessage::Heartbeat(Heartbeat {
            feed: self.feed,
            last_sequence: self.advertised,
            is_final: simulator.state() == SimulatorState::Completed,
        });

        if self.sink.send_control(message, self.config.target)? {
            self.stats.heartbeats_sent += 1;
        }

        Ok(())
    }

    // --------------------------------------------------------
    // Requests
    // --------------------------------------------------------

    /// Answers one retransmission request from history.
    ///
    /// An unavailable range is counted and left unanswered.
    /// A reply cut short by a full socket is also left as is.
    /// In both cases the consumer's own timeout decides what
    /// happens next.
    fn answer(
        &mut self,
        simulator: &mut Simulator,
        request: RetransmitRequest,
        requester: SocketAddr,
    ) -> SimulatorResult<()> {
        self.stats.requests_received += 1;

        let range = match simulator.retransmission(request.range.start(), request.range.end()) {
            Ok(range) => range,

            Err(error) if error.is_history_unavailable() => {
                self.stats.requests_unavailable += 1;

                return Ok(());
            }

            Err(SimulatorError::InvalidHistoryRange { .. }) => {
                self.stats.rejected += 1;

                return Ok(());
            }

            Err(error) => return Err(error),
        };

        for event in range {
            if !self.sink.send_event(event, requester)? {
                break;
            }

            self.stats.events_retransmitted += 1;
        }

        Ok(())
    }

    /// Reads and handles inbound control datagrams.
    ///
    /// Returns whether any datagram was read.
    fn service_requests(&mut self, simulator: &mut Simulator) -> SimulatorResult<bool> {
        let mut progressed = false;

        for _ in 0..self.config.request_budget {
            let Some(datagram) = self.sink.socket.receive_into(&mut self.inbound)? else {
                break;
            };

            progressed = true;

            let message = match ControlMessage::decode(self.inbound.as_slice()) {
                Ok(message) if message.feed() == self.feed => message,

                _ => {
                    self.stats.rejected += 1;
                    continue;
                }
            };

            match message {
                ControlMessage::RetransmitRequest(request) => {
                    self.answer(simulator, request, datagram.source)?;
                }

                ControlMessage::Complete(_) => {
                    self.stats.consumer_complete = true;
                }

                ControlMessage::Heartbeat(_) => {
                    self.stats.rejected += 1;
                }
            }
        }

        Ok(progressed)
    }

    // --------------------------------------------------------
    // Execution
    // --------------------------------------------------------

    /// Runs the simulation to completion over the socket.
    ///
    /// After the last event the runner keeps answering
    /// requests until the consumer reports completion or no
    /// request arrives for one linger period.
    ///
    /// # Errors
    ///
    /// Returns an error if the simulator fails or the socket
    /// reports a failure other than backpressure.
    pub fn run(&mut self, simulator: &mut Simulator) -> SimulatorResult<NetworkStats> {
        if simulator.feed() != self.feed {
            return Err(SimulatorError::InvalidConfig(
                "network feed differs from simulator feed",
            ));
        }

        if simulator.state() == SimulatorState::Initialized {
            simulator.start()?;
        }

        let mut last_heartbeat = Instant::now();
        let mut quiet_since: Option<Instant> = None;

        loop {
            let mut progressed = false;

            if quiet_since.is_none() {
                let report = simulator.service(&mut self.sink)?;

                progressed = report.made_progress();

                self.refresh_watermark(simulator);

                if report.is_completed() {
                    quiet_since = Some(Instant::now());

                    // Announce the final watermark at once.
                    self.send_heartbeat(simulator)?;
                    last_heartbeat = Instant::now();
                }
            }

            let requests_before = self.stats.requests_received;

            progressed |= self.service_requests(simulator)?;

            if quiet_since.is_some() && self.stats.requests_received != requests_before {
                quiet_since = Some(Instant::now());
            }

            if last_heartbeat.elapsed() >= self.config.heartbeat_interval {
                self.send_heartbeat(simulator)?;
                last_heartbeat = Instant::now();
            }

            if let Some(quiet_since) = quiet_since
                && (self.stats.consumer_complete || quiet_since.elapsed() >= self.config.linger)
            {
                return Ok(self.stats());
            }

            if !progressed {
                thread::sleep(IDLE_PAUSE);
            }
        }
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{SequenceNumber, SequenceRange, SessionId, StreamId};
    use maxwell_protocol::Decoder;

    use crate::scenario::Scenario;
    use crate::simulator::RuntimeLimits;

    fn feed() -> FeedId {
        FeedId::new(StreamId::new(1), SessionId::new(1))
    }

    fn local() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    fn consumer() -> UdpUnicast {
        let mut socket = UdpUnicast::bind(local(), UdpConfig::default()).unwrap();

        socket.set_nonblocking(false).unwrap();

        socket
            .socket()
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        socket
    }

    fn simulator(event_count: u64) -> Simulator {
        let mut config = Scenario::Smoke.config().unwrap();

        config.run.event_count = event_count;

        Simulator::new(config, feed(), RuntimeLimits::default()).unwrap()
    }

    fn runner(target: SocketAddr, linger: Duration) -> NetworkRunner {
        let mut config = NetworkConfig::new(local(), target);

        config.linger = linger;

        NetworkRunner::bind(config, feed()).unwrap()
    }

    #[test]
    fn configuration_validation() {
        let valid = NetworkConfig::new(local(), local());

        valid.validate().unwrap();

        let mut config = valid;
        config.heartbeat_interval = Duration::ZERO;
        assert!(config.validate().is_err());

        let mut config = valid;
        config.request_budget = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_mismatched_feed() {
        let config = NetworkConfig::new(local(), local());

        let other = FeedId::new(StreamId::new(9), SessionId::new(9));

        let mut runner = NetworkRunner::bind(config, other).unwrap();
        let mut simulator = simulator(10);

        assert!(matches!(
            runner.run(&mut simulator),
            Err(SimulatorError::InvalidConfig(_)),
        ));
    }

    #[test]
    fn sends_every_event_then_a_final_heartbeat() {
        let mut consumer = consumer();
        let target = consumer.local_addr().unwrap();

        let mut runner = runner(target, Duration::from_millis(10));
        let mut simulator = simulator(50);

        let stats = runner.run(&mut simulator).unwrap();

        assert_eq!(stats.events_sent, 50);
        assert!(stats.heartbeats_sent >= 1);
        assert!(!stats.consumer_complete);

        let decoder = Decoder::new();
        let mut buffer = consumer.create_receive_buffer().unwrap();
        let mut next = 1u64;
        let mut final_watermark = None;

        while final_watermark.is_none() {
            consumer.receive_into(&mut buffer).unwrap().unwrap();

            if ControlMessage::matches(buffer.as_slice()) {
                let ControlMessage::Heartbeat(heartbeat) =
                    ControlMessage::decode(buffer.as_slice()).unwrap()
                else {
                    panic!("producer sent a non-heartbeat control message");
                };

                assert_eq!(heartbeat.feed, feed());

                if heartbeat.is_final {
                    final_watermark = heartbeat.last_sequence;
                }
            } else {
                let event = decoder.decode_event(buffer.as_slice()).unwrap();

                assert_eq!(event.sequence().raw(), next);
                next += 1;
            }
        }

        assert_eq!(next, 51);
        assert_eq!(final_watermark, Some(SequenceNumber::new(50)));
    }

    #[test]
    fn answers_retransmission_and_stops_on_completion() {
        let mut consumer = consumer();
        let target = consumer.local_addr().unwrap();

        let mut runner = runner(target, Duration::from_secs(30));
        let producer = runner.local_addr().unwrap();
        let mut simulator = simulator(20);

        // Queue a request and a completion notice before the
        // run. The request cannot be served until the events
        // exist, so the first one is counted as unavailable or
        // served depending on timing. Sending it again after
        // the final heartbeat makes the outcome deterministic.
        let request = ControlMessage::RetransmitRequest(RetransmitRequest {
            feed: feed(),
            request_id: 1,
            range: SequenceRange::new(SequenceNumber::new(5), SequenceNumber::new(8)).unwrap(),
        });

        let complete = ControlMessage::Complete(maxwell_protocol::Complete {
            feed: feed(),
            last_sequence: SequenceNumber::new(20),
        });

        let handle = thread::spawn(move || {
            let decoder = Decoder::new();
            let mut buffer = consumer.create_receive_buffer().unwrap();
            let mut frame = [0u8; CONTROL_SIZE];

            // Wait for the final heartbeat.
            loop {
                consumer.receive_into(&mut buffer).unwrap().unwrap();

                if let Ok(ControlMessage::Heartbeat(heartbeat)) =
                    ControlMessage::decode(buffer.as_slice())
                    && heartbeat.is_final
                {
                    break;
                }
            }

            request.encode(&mut frame).unwrap();
            consumer.send_to(&frame, producer).unwrap();

            let mut replayed = Vec::new();

            while replayed.len() < 4 {
                consumer.receive_into(&mut buffer).unwrap().unwrap();

                if !ControlMessage::matches(buffer.as_slice()) {
                    replayed.push(
                        decoder
                            .decode_event(buffer.as_slice())
                            .unwrap()
                            .sequence()
                            .raw(),
                    );
                }
            }

            complete.encode(&mut frame).unwrap();
            consumer.send_to(&frame, producer).unwrap();

            replayed
        });

        let started = Instant::now();
        let stats = runner.run(&mut simulator).unwrap();

        assert_eq!(handle.join().unwrap(), vec![5, 6, 7, 8]);

        assert!(stats.consumer_complete);
        assert_eq!(stats.requests_received, 1);
        assert_eq!(stats.requests_unavailable, 0);
        assert_eq!(stats.events_retransmitted, 4);

        // Completion ends the linger period early.
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn ignores_unusable_inbound_datagrams() {
        let mut consumer = consumer();
        let target = consumer.local_addr().unwrap();

        let mut runner = runner(target, Duration::from_millis(300));
        let producer = runner.local_addr().unwrap();
        let mut simulator = simulator(5);

        // Garbage, a foreign feed, and a heartbeat.
        consumer
            .send_to(b"not a control datagram", producer)
            .unwrap();

        let mut frame = [0u8; CONTROL_SIZE];

        ControlMessage::Complete(maxwell_protocol::Complete {
            feed: FeedId::new(StreamId::new(2), SessionId::new(2)),
            last_sequence: SequenceNumber::ONE,
        })
        .encode(&mut frame)
        .unwrap();

        consumer.send_to(&frame, producer).unwrap();

        ControlMessage::Heartbeat(Heartbeat {
            feed: feed(),
            last_sequence: None,
            is_final: false,
        })
        .encode(&mut frame)
        .unwrap();

        consumer.send_to(&frame, producer).unwrap();

        let stats = runner.run(&mut simulator).unwrap();

        assert_eq!(stats.rejected, 3);
        assert_eq!(stats.requests_received, 0);
        assert!(!stats.consumer_complete);
        assert_eq!(stats.events_sent, 5);
    }
}
