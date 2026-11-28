//! Socket-driven server runtime.
//!
//! The runtime owns one UDP socket and one [`Server`]. It turns
//! received datagrams into server calls, sends retransmission
//! requests to the producer, and decides when to stop.
//!
//! Everything runs on the calling thread. The socket uses a
//! short read timeout so that timers are serviced while the
//! feed is quiet.
//!
//! # Datagram feed contract
//!
//! - One datagram carries one native market event or one feed
//!   control message.
//! - Retransmitted events arrive as ordinary event datagrams
//!   and are admitted through ordinary sequencing.
//! - Snapshots are not available over this transport. A gap
//!   that retransmission cannot heal stops the runtime.

use std::io::{self, Write};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use maxwell_core::{
    FeedId, Instrument, InstrumentConfig, InstrumentId, MonotonicTimestamp, Price, Quantity,
    SequenceNumber, SessionId, StreamId,
};
use maxwell_protocol::{CONTROL_SIZE, Complete, ControlMessage, Heartbeat, RetransmitRequest};
use maxwell_recovery::{RecoveryRequest, RequestId};
use maxwell_transport::{
    DatagramReceiver, DatagramSender, ReceiveBuffer, UdpConfig, UdpUnicast, WriteOutcome,
};

use crate::config::ServerConfig;
use crate::error::{ServerError, ServerResult};
use crate::recovery::RecoveryOutcome;
use crate::server::Server;
use crate::state::ServerState;

// ============================================================
// Defaults
// ============================================================

/// Longest wait for a datagram before timers are serviced.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Most datagrams consumed between two timer services.
pub const DEFAULT_RECEIVE_BUDGET: usize = 64;

/// Most events requested by one retransmission request.
///
/// The recovery default suits a stream transport. A datagram
/// reply of that size would overflow ordinary socket buffers.
pub const DEFAULT_REQUEST_BATCH: u64 = 128;

// ============================================================
// Market specification
// ============================================================

/// Instrument definitions shared with the producer.
///
/// Instrument reference data does not travel on the feed, so
/// both sides derive it from the same parameters. The defaults
/// and the price band rule match the Maxwell simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketSpec {
    pub instrument_count: usize,
    pub max_orders_per_instrument: usize,
    pub reference_price: i64,
    pub tick_size: i64,
    pub lot_size: u64,
    pub max_price_distance_ticks: u32,
    pub max_order_quantity_lots: u32,
}

impl Default for MarketSpec {
    fn default() -> Self {
        Self {
            instrument_count: 10,
            max_orders_per_instrument: 1_024,
            reference_price: 1_000_000,
            tick_size: 100,
            lot_size: 1,
            max_price_distance_ticks: 100,
            max_order_quantity_lots: 100,
        }
    }
}

impl MarketSpec {
    /// Builds the instrument with the given one-based index.
    fn instrument(&self, index: usize) -> ServerResult<Instrument> {
        const INVALID: ServerError = ServerError::InvalidConfig("invalid market specification");

        let id = u32::try_from(index).map_err(|_| INVALID)?;

        let distance = self
            .tick_size
            .checked_mul(i64::from(self.max_price_distance_ticks))
            .ok_or(INVALID)?;

        let min_price = self.reference_price.checked_sub(distance).ok_or(INVALID)?;
        let max_price = self.reference_price.checked_add(distance).ok_or(INVALID)?;

        let max_quantity = self
            .lot_size
            .checked_mul(u64::from(self.max_order_quantity_lots))
            .ok_or(INVALID)?;

        Instrument::new(InstrumentConfig {
            id: InstrumentId::new(id),
            price_scale: 4,
            quantity_scale: 0,
            tick_size: Price::new(self.tick_size),
            lot_size: Quantity::new(self.lot_size),
            min_price: Price::new(min_price),
            max_price: Price::new(max_price),
            min_quantity: Quantity::new(self.lot_size),
            max_quantity: Quantity::new(max_quantity),
        })
        .map_err(|_| INVALID)
    }

    /// Builds every instrument, numbered from one.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfig` if the parameters do not describe
    /// a valid instrument, or `AllocationFailed`.
    pub fn instruments(&self) -> ServerResult<Vec<Instrument>> {
        if self.instrument_count == 0 || self.max_orders_per_instrument == 0 {
            return Err(ServerError::InvalidConfig(
                "market needs at least one instrument and one order",
            ));
        }

        let mut instruments = Vec::new();

        instruments
            .try_reserve_exact(self.instrument_count)
            .map_err(|_| ServerError::AllocationFailed)?;

        for index in 1..=self.instrument_count {
            instruments.push(self.instrument(index)?);
        }

        Ok(instruments)
    }

    /// Sizes the book limits in `config` for this market.
    ///
    /// Maxwell uses dense price ladders, so the level capacity
    /// must cover the whole price band.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfig` for an invalid specification.
    pub fn size_books(&self, config: &mut ServerConfig) -> ServerResult<()> {
        let levels = self
            .instrument(1)?
            .price_level_count()
            .and_then(|count| usize::try_from(count).ok())
            .ok_or(ServerError::InvalidConfig("price band is too large"))?;

        config.book.max_instruments = self.instrument_count;
        config.book.max_orders = self.max_orders_per_instrument;
        config.book.max_price_levels = levels;

        Ok(())
    }
}

// ============================================================
// Runtime configuration
// ============================================================

/// Network and stopping configuration for one feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// Local address the feed socket binds to.
    pub listen: SocketAddr,

    /// Producer address for retransmission requests.
    ///
    /// When set, datagrams from any other address are ignored.
    /// When unset, the source of the latest accepted datagram
    /// is used.
    pub producer: Option<SocketAddr>,

    pub feed: FeedId,
    pub udp: UdpConfig,
    pub poll_interval: Duration,
    pub receive_budget: usize,

    /// Stop once the producer's final watermark is applied.
    pub exit_on_final: bool,

    /// Stop after this long without any datagram.
    pub idle_timeout: Option<Duration>,

    /// Interval between status lines written by [`Runtime::run`].
    pub status_interval: Option<Duration>,
}

impl RuntimeConfig {
    /// Creates a configuration with default tuning.
    #[must_use]
    pub fn new(listen: SocketAddr, feed: FeedId) -> Self {
        Self {
            listen,
            producer: None,
            feed,
            udp: UdpConfig::default(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            receive_budget: DEFAULT_RECEIVE_BUDGET,
            exit_on_final: false,
            idle_timeout: None,
            status_interval: None,
        }
    }

    /// Validates the runtime parameters.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfig` for a zero interval or budget.
    pub fn validate(&self) -> ServerResult<()> {
        if self.poll_interval.is_zero() {
            return Err(ServerError::InvalidConfig(
                "poll interval must be greater than zero",
            ));
        }

        if self.receive_budget == 0 {
            return Err(ServerError::InvalidConfig(
                "receive budget must be greater than zero",
            ));
        }

        if self.idle_timeout.is_some_and(|timeout| timeout.is_zero()) {
            return Err(ServerError::InvalidConfig(
                "idle timeout must be greater than zero",
            ));
        }

        if self
            .status_interval
            .is_some_and(|interval| interval.is_zero())
        {
            return Err(ServerError::InvalidConfig(
                "status interval must be greater than zero",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Runtime observation
// ============================================================

/// Cumulative counters for one runtime.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStats {
    /// Datagrams read from the socket.
    pub datagrams: u64,

    /// Event datagrams the server consumed.
    pub events: u64,

    /// Consumed events that were already applied.
    pub duplicates: u64,

    /// Consumed events held for a gap ahead of them.
    pub buffered: u64,

    /// Heartbeats accepted from the producer.
    pub heartbeats: u64,

    /// Datagrams that failed decoding or validation.
    pub rejected: u64,

    /// Datagrams from an address other than the producer.
    pub foreign: u64,

    /// Events dropped because publication was blocked.
    pub blocked: u64,

    /// Retransmission requests sent to the producer.
    pub retransmit_requests: u64,

    /// Times the server regained a synchronized baseline.
    pub resynchronizations: u64,
}

/// Reason a runtime stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The producer's final watermark was fully applied.
    FeedComplete,

    /// No datagram arrived within the idle timeout.
    Idle,
}

impl StopReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FeedComplete => "feed complete",
            Self::Idle => "idle timeout",
        }
    }
}

/// Final observation of a stopped runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunSummary {
    pub reason: StopReason,
    pub stats: RuntimeStats,
    pub synchronized: bool,
    pub next_expected: SequenceNumber,
    pub publication_watermark: u64,
    pub elapsed: Duration,
}

// ============================================================
// Runtime
// ============================================================

/// Single-threaded UDP runtime for one upstream feed.
pub struct Runtime {
    config: RuntimeConfig,
    server: Server,
    socket: UdpUnicast,
    buffer: ReceiveBuffer,
    control: [u8; CONTROL_SIZE],
    started: Instant,
    last_datagram: Instant,
    producer: Option<SocketAddr>,
    final_watermark: Option<Option<SequenceNumber>>,
    deferred_request: Option<RequestId>,
    stats: RuntimeStats,
}

impl Runtime {
    /// Binds the feed socket and starts the server from an
    /// empty market at sequence one.
    ///
    /// # Errors
    ///
    /// Returns an error if a configuration is invalid, an
    /// instrument cannot be registered, or the socket cannot
    /// be bound.
    pub fn bind(
        config: RuntimeConfig,
        server_config: ServerConfig,
        instruments: &[Instrument],
    ) -> ServerResult<Self> {
        config.validate()?;

        let mut server = Server::new(server_config, config.feed)?;

        for instrument in instruments {
            server.register_instrument(*instrument)?;
        }

        let mut socket = UdpUnicast::bind(config.listen, config.udp)?;

        // Block for at most one poll interval per receive, so
        // a quiet feed does not spin the processor.
        socket.set_nonblocking(false)?;

        socket
            .socket()
            .set_read_timeout(Some(config.poll_interval))?;

        let buffer = socket.create_receive_buffer()?;
        let started = Instant::now();

        server.start_empty(MonotonicTimestamp::new(0))?;

        Ok(Self {
            config,
            server,
            socket,
            buffer,
            control: [0; CONTROL_SIZE],
            started,
            last_datagram: started,
            producer: config.producer,
            final_watermark: None,
            deferred_request: None,
            stats: RuntimeStats::default(),
        })
    }

    // --------------------------------------------------------
    // Observation
    // --------------------------------------------------------

    #[must_use]
    pub const fn server(&self) -> &Server {
        &self.server
    }

    #[must_use]
    pub const fn stats(&self) -> RuntimeStats {
        self.stats
    }

    /// Returns the address the feed socket is bound to.
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system cannot report it.
    pub fn local_addr(&self) -> ServerResult<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    fn now(&self) -> MonotonicTimestamp {
        let elapsed = self.started.elapsed().as_nanos();

        MonotonicTimestamp::new(u64::try_from(elapsed).unwrap_or(u64::MAX))
    }

    fn summary(&self, reason: StopReason) -> RunSummary {
        let status = self.server.status();

        RunSummary {
            reason,
            stats: self.stats,
            synchronized: status.synchronized,
            next_expected: status.next_expected,
            publication_watermark: status.publication_watermark,
            elapsed: self.started.elapsed(),
        }
    }

    // --------------------------------------------------------
    // Datagram admission
    // --------------------------------------------------------

    /// Applies the source address policy to one datagram.
    fn accept_source(&mut self, source: SocketAddr) -> bool {
        match self.config.producer {
            Some(expected) if expected != source => {
                self.stats.foreign += 1;
                false
            }
            _ => true,
        }
    }

    /// A rejected datagram is counted and skipped unless the
    /// failure stopped the server.
    fn reject(&mut self, error: ServerError) -> ServerResult<()> {
        if self.server.state() == ServerState::Failed {
            return Err(error);
        }

        self.stats.rejected += 1;

        Ok(())
    }

    fn handle_control(&mut self, source: SocketAddr, now: MonotonicTimestamp) -> ServerResult<()> {
        let message = match ControlMessage::decode(self.buffer.as_slice()) {
            Ok(message) => message,
            Err(error) => return self.reject(error.into()),
        };

        // Only the producer's heartbeat is meaningful here.
        let ControlMessage::Heartbeat(Heartbeat {
            feed,
            last_sequence,
            is_final,
        }) = message
        else {
            self.stats.rejected += 1;
            return Ok(());
        };

        if feed != self.config.feed {
            self.stats.rejected += 1;
            return Ok(());
        }

        self.stats.heartbeats += 1;
        self.producer = Some(source);

        if let Some(last_sequence) = last_sequence {
            self.server.observe_upstream_watermark(last_sequence, now)?;
        }

        if is_final {
            self.final_watermark = Some(last_sequence);
        }

        Ok(())
    }

    /// Decides when a retransmission request leaves.
    ///
    /// A gap first seen behind a newly arrived event may be
    /// nothing more than two datagrams that swapped places, so
    /// the request waits for one more datagram. A gap reached
    /// by replaying buffered events is already old and is
    /// requested at once, which keeps recovery ahead of a
    /// lossy feed.
    fn schedule_recovery(&mut self, outcome: RecoveryOutcome) -> ServerResult<()> {
        let Some(request) = self.server.recovery().pending_request() else {
            self.deferred_request = None;
            return Ok(());
        };

        let seen_before = self.deferred_request == Some(request.id());

        if seen_before || outcome == RecoveryOutcome::Replayed {
            return self.dispatch_recovery();
        }

        self.deferred_request = Some(request.id());

        Ok(())
    }

    fn handle_event(&mut self, source: SocketAddr, now: MonotonicTimestamp) -> ServerResult<()> {
        let mut outcome = self.server.ingest_payload(self.buffer.as_slice(), now);

        if matches!(outcome, Ok(RecoveryOutcome::InputBlocked)) {
            // A pending publication holds new input back. Try
            // to clear it once before giving the datagram up.
            self.server.retry_publication()?;

            outcome = self.server.ingest_payload(self.buffer.as_slice(), now);
        }

        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => return self.reject(error),
        };

        if outcome == RecoveryOutcome::InputBlocked {
            // The datagram cannot be retained. Recovery will
            // request the event again once the gap is visible.
            self.stats.blocked += 1;
            return Ok(());
        }

        self.stats.events += 1;
        self.producer = Some(source);

        match outcome {
            RecoveryOutcome::Duplicate => self.stats.duplicates += 1,
            RecoveryOutcome::Buffered => self.stats.buffered += 1,
            RecoveryOutcome::Resynchronized => self.stats.resynchronizations += 1,
            _ => {}
        }

        self.schedule_recovery(outcome)
    }

    /// Reads datagrams until the budget is spent or the socket
    /// stays quiet for one poll interval.
    fn receive(&mut self) -> ServerResult<()> {
        for _ in 0..self.config.receive_budget {
            let Some(datagram) = self.socket.receive_into(&mut self.buffer)? else {
                break;
            };

            self.stats.datagrams += 1;
            self.last_datagram = Instant::now();

            if !self.accept_source(datagram.source) {
                continue;
            }

            let now = self.now();

            if ControlMessage::matches(self.buffer.as_slice()) {
                self.handle_control(datagram.source, now)?;
            } else {
                self.handle_event(datagram.source, now)?;
            }
        }

        Ok(())
    }

    // --------------------------------------------------------
    // Recovery requests
    // --------------------------------------------------------

    fn send_control(
        &mut self,
        message: ControlMessage,
        destination: SocketAddr,
    ) -> ServerResult<bool> {
        let length = message.encode(&mut self.control)?;

        let outcome = self.socket.send_to(&self.control[..length], destination)?;

        Ok(matches!(outcome, WriteOutcome::Written(_)))
    }

    /// Sends the pending recovery request, if any.
    ///
    /// A request that cannot be sent yet stays pending and is
    /// attempted again on the next step.
    fn dispatch_recovery(&mut self) -> ServerResult<()> {
        let Some(request) = self.server.recovery().pending_request() else {
            return Ok(());
        };

        let request = match request {
            RecoveryRequest::Retransmission(request) => request,
            RecoveryRequest::Snapshot(_) => {
                // The datagram feed has no snapshot source.
                return Err(ServerError::RecoveryUnavailable);
            }
        };

        let Some(producer) = self.producer else {
            return Ok(());
        };

        let message = ControlMessage::RetransmitRequest(RetransmitRequest {
            feed: request.feed,
            request_id: request.id.raw(),
            range: request.range,
        });

        if self.send_control(message, producer)? {
            self.server.acknowledge_recovery_request(request.id)?;
            self.stats.retransmit_requests += 1;
            self.deferred_request = None;
        }

        Ok(())
    }

    // --------------------------------------------------------
    // Stopping
    // --------------------------------------------------------

    /// Reports whether the producer's final watermark has been
    /// applied with nothing left to recover.
    fn feed_complete(&self) -> bool {
        let Some(final_watermark) = self.final_watermark else {
            return false;
        };

        if !self.server.is_synchronized() {
            return false;
        }

        final_watermark.is_none_or(|last| self.server.status().next_expected > last)
    }

    fn stop_reason(&self) -> Option<StopReason> {
        if self.config.exit_on_final && self.feed_complete() {
            return Some(StopReason::FeedComplete);
        }

        match self.config.idle_timeout {
            Some(timeout) if self.last_datagram.elapsed() >= timeout => Some(StopReason::Idle),
            _ => None,
        }
    }

    /// Tells the producer that the complete stream was applied.
    ///
    /// The notice is a courtesy that lets the producer stop
    /// waiting for requests. Its loss is harmless.
    fn notify_complete(&mut self) {
        let (Some(producer), Some(Some(last_sequence))) = (self.producer, self.final_watermark)
        else {
            return;
        };

        let message = ControlMessage::Complete(Complete {
            feed: self.config.feed,
            last_sequence,
        });

        let _ = self.send_control(message, producer);
    }

    // --------------------------------------------------------
    // Execution
    // --------------------------------------------------------

    /// Runs one iteration: receive, service timers, and send
    /// any pending recovery request.
    ///
    /// Returns the reason to stop once a stop condition holds.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket fails, the server enters
    /// its failed state, or recovery needs a snapshot.
    pub fn step(&mut self) -> ServerResult<Option<StopReason>> {
        self.receive()?;

        let now = self.now();

        self.server.poll(now)?;
        self.dispatch_recovery()?;

        Ok(self.stop_reason())
    }

    fn write_status(&self, log: &mut dyn Write) -> io::Result<()> {
        let status = self.server.status();
        let stats = self.stats;

        writeln!(
            log,
            "status elapsed_ms={} lifecycle={} synchronized={} next_expected={} \
             datagrams={} events={} duplicates={} rejected={} retransmit_requests={}",
            self.started.elapsed().as_millis(),
            status.lifecycle,
            status.synchronized,
            status.next_expected.raw(),
            stats.datagrams,
            stats.events,
            stats.duplicates,
            stats.rejected,
            stats.retransmit_requests,
        )
    }

    /// Runs until a stop condition holds, then shuts the
    /// server down.
    ///
    /// Status lines go to `log` at the configured interval.
    ///
    /// # Errors
    ///
    /// Returns the first error from [`Self::step`], from
    /// writing a status line, or from shutdown.
    pub fn run(&mut self, log: &mut dyn Write) -> ServerResult<RunSummary> {
        let mut last_status = Instant::now();

        let reason = loop {
            if let Some(reason) = self.step()? {
                break reason;
            }

            if let Some(interval) = self.config.status_interval
                && last_status.elapsed() >= interval
            {
                self.write_status(log)?;
                last_status = Instant::now();
            }
        };

        if reason == StopReason::FeedComplete {
            self.notify_complete();
        }

        let summary = self.summary(reason);

        self.server.begin_shutdown(self.now())?;
        self.server.finish_shutdown(self.now())?;

        Ok(summary)
    }
}

// ============================================================
// Command-line options
// ============================================================

/// Options for the `run` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    pub runtime: RuntimeConfig,
    pub market: MarketSpec,
    pub request_batch: u64,
}

fn value<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = String>,
    message: &'static str,
) -> ServerResult<T> {
    args.next()
        .and_then(|value| value.parse().ok())
        .ok_or(ServerError::InvalidConfig(message))
}

impl RunOptions {
    /// Parses the arguments that follow the `run` command.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfig` for an unknown option, a missing
    /// or malformed value, or a missing `--listen` address.
    pub fn parse(args: impl IntoIterator<Item = String>) -> ServerResult<Self> {
        let mut args = args.into_iter();

        let mut listen: Option<SocketAddr> = None;
        let mut producer = None;
        let mut stream_id = 1u32;
        let mut session_id = 1u64;
        let mut exit_on_final = false;
        let mut idle_timeout = None;
        let mut status_interval = None;
        let mut request_batch = DEFAULT_REQUEST_BATCH;
        let mut market = MarketSpec::default();

        while let Some(option) = args.next() {
            let args = &mut args;

            match option.as_str() {
                "--listen" => listen = Some(value(args, "invalid --listen address")?),
                "--producer" => producer = Some(value(args, "invalid --producer address")?),
                "--stream-id" => stream_id = value(args, "invalid --stream-id")?,
                "--session-id" => session_id = value(args, "invalid --session-id")?,
                "--exit-on-final" => exit_on_final = true,
                "--idle-timeout-ms" => {
                    idle_timeout = Some(Duration::from_millis(value(
                        args,
                        "invalid --idle-timeout-ms",
                    )?));
                }
                "--status-interval-ms" => {
                    status_interval = Some(Duration::from_millis(value(
                        args,
                        "invalid --status-interval-ms",
                    )?));
                }
                "--request-batch" => request_batch = value(args, "invalid --request-batch")?,
                "--instruments" => {
                    market.instrument_count = value(args, "invalid --instruments")?;
                }
                "--max-orders" => {
                    market.max_orders_per_instrument = value(args, "invalid --max-orders")?;
                }
                "--reference-price" => {
                    market.reference_price = value(args, "invalid --reference-price")?;
                }
                "--tick-size" => market.tick_size = value(args, "invalid --tick-size")?,
                "--lot-size" => market.lot_size = value(args, "invalid --lot-size")?,
                "--max-price-distance-ticks" => {
                    market.max_price_distance_ticks =
                        value(args, "invalid --max-price-distance-ticks")?;
                }
                "--max-order-quantity-lots" => {
                    market.max_order_quantity_lots =
                        value(args, "invalid --max-order-quantity-lots")?;
                }
                _ => return Err(ServerError::InvalidConfig("unknown run option")),
            }
        }

        let listen = listen.ok_or(ServerError::InvalidConfig("run requires --listen <ADDR>"))?;

        if request_batch == 0 {
            return Err(ServerError::InvalidConfig(
                "request batch must be greater than zero",
            ));
        }

        let feed = FeedId::new(StreamId::new(stream_id), SessionId::new(session_id));

        let mut runtime = RuntimeConfig::new(listen, feed);

        runtime.producer = producer;
        runtime.exit_on_final = exit_on_final;
        runtime.idle_timeout = idle_timeout;
        runtime.status_interval = status_interval;
        runtime.validate()?;

        Ok(Self {
            runtime,
            market,
            request_batch,
        })
    }

    /// Builds the server configuration for these options.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfig` for an invalid market.
    pub fn server_config(&self) -> ServerResult<ServerConfig> {
        let mut config = ServerConfig::default();

        self.market.size_books(&mut config)?;
        config.recovery.max_request_batch = self.request_batch;
        config.validate()?;

        Ok(config)
    }

    /// Binds a runtime for these options.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Runtime::bind`].
    pub fn bind(&self) -> ServerResult<Runtime> {
        let instruments = self.market.instruments()?;

        Runtime::bind(self.runtime, self.server_config()?, &instruments)
    }
}

/// Usage text for the `run` command.
pub const RUN_USAGE: &str = "\
Usage:
  maxwell-server run --listen <ADDR> [OPTIONS]

Feed:
  --listen <ADDR>                 Local UDP address for the feed
  --producer <ADDR>               Producer address. Other sources are ignored.
                                  Default: learned from the feed.
  --stream-id <N>                 Feed stream identifier. Default: 1
  --session-id <N>                Feed session identifier. Default: 1
  --request-batch <N>             Events per retransmission request. Default: 128

Stopping:
  --exit-on-final                 Stop once the producer's final watermark is applied
  --idle-timeout-ms <N>           Stop after N milliseconds without a datagram
  --status-interval-ms <N>        Write a status line every N milliseconds

Market (must match the producer):
  --instruments <N>               Default: 10
  --max-orders <N>                Resting orders per instrument. Default: 1024
  --reference-price <N>           Default: 1000000
  --tick-size <N>                 Default: 100
  --lot-size <N>                  Default: 1
  --max-price-distance-ticks <N>  Default: 100
  --max-order-quantity-lots <N>   Default: 100";

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> ServerResult<RunOptions> {
        RunOptions::parse(args.iter().map(ToString::to_string))
    }

    #[test]
    fn market_defaults_describe_valid_instruments() {
        let market = MarketSpec::default();
        let instruments = market.instruments().unwrap();

        assert_eq!(instruments.len(), 10);
        assert_eq!(instruments[0].id(), InstrumentId::new(1));
        assert_eq!(instruments[9].id(), InstrumentId::new(10));
        assert_eq!(instruments[0].min_price(), Price::new(990_000));
        assert_eq!(instruments[0].max_price(), Price::new(1_010_000));
    }

    #[test]
    fn market_sizes_book_limits() {
        let market = MarketSpec {
            instrument_count: 3,
            max_orders_per_instrument: 64,
            ..MarketSpec::default()
        };

        let mut config = ServerConfig::default();

        market.size_books(&mut config).unwrap();

        assert_eq!(config.book.max_instruments, 3);
        assert_eq!(config.book.max_orders, 64);
        assert_eq!(config.book.max_price_levels, 201);

        config.validate().unwrap();
    }

    #[test]
    fn market_rejects_invalid_parameters() {
        let empty = MarketSpec {
            instrument_count: 0,
            ..MarketSpec::default()
        };

        assert!(empty.instruments().is_err());

        let overflow = MarketSpec {
            tick_size: i64::MAX,
            ..MarketSpec::default()
        };

        assert!(overflow.instruments().is_err());
    }

    #[test]
    fn parses_minimal_options() {
        let options = parse(&["--listen", "127.0.0.1:9100"]).unwrap();

        assert_eq!(options.runtime.listen, "127.0.0.1:9100".parse().unwrap(),);
        assert_eq!(options.runtime.producer, None);
        assert!(!options.runtime.exit_on_final);
        assert_eq!(options.runtime.idle_timeout, None);
        assert_eq!(options.request_batch, DEFAULT_REQUEST_BATCH);
        assert_eq!(options.market, MarketSpec::default());

        assert_eq!(
            options.runtime.feed,
            FeedId::new(StreamId::new(1), SessionId::new(1)),
        );
    }

    #[test]
    fn parses_all_options() {
        let options = parse(&[
            "--listen",
            "0.0.0.0:9100",
            "--producer",
            "10.0.0.5:9101",
            "--stream-id",
            "3",
            "--session-id",
            "8",
            "--exit-on-final",
            "--idle-timeout-ms",
            "2500",
            "--status-interval-ms",
            "1000",
            "--request-batch",
            "64",
            "--instruments",
            "2",
            "--max-orders",
            "64",
            "--reference-price",
            "500000",
            "--tick-size",
            "50",
            "--lot-size",
            "10",
            "--max-price-distance-ticks",
            "20",
            "--max-order-quantity-lots",
            "5",
        ])
        .unwrap();

        assert_eq!(
            options.runtime.producer,
            Some("10.0.0.5:9101".parse().unwrap()),
        );

        assert_eq!(
            options.runtime.feed,
            FeedId::new(StreamId::new(3), SessionId::new(8)),
        );

        assert!(options.runtime.exit_on_final);

        assert_eq!(
            options.runtime.idle_timeout,
            Some(Duration::from_millis(2_500)),
        );

        assert_eq!(
            options.runtime.status_interval,
            Some(Duration::from_secs(1)),
        );

        assert_eq!(options.request_batch, 64);

        assert_eq!(
            options.market,
            MarketSpec {
                instrument_count: 2,
                max_orders_per_instrument: 64,
                reference_price: 500_000,
                tick_size: 50,
                lot_size: 10,
                max_price_distance_ticks: 20,
                max_order_quantity_lots: 5,
            },
        );

        assert_eq!(
            options.server_config().unwrap().recovery.max_request_batch,
            64,
        );
    }

    #[test]
    fn rejects_invalid_options() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["--listen"]).is_err());
        assert!(parse(&["--listen", "not-an-address"]).is_err());
        assert!(parse(&["--listen", "127.0.0.1:9100", "--unknown"]).is_err());
        assert!(parse(&["--listen", "127.0.0.1:9100", "--request-batch", "0"]).is_err());
        assert!(parse(&["--listen", "127.0.0.1:9100", "--idle-timeout-ms", "0"]).is_err());
        assert!(parse(&["--listen", "127.0.0.1:9100", "--instruments", "x"]).is_err());
    }

    #[test]
    fn runtime_configuration_validation() {
        let feed = FeedId::new(StreamId::new(1), SessionId::new(1));
        let valid = RuntimeConfig::new("127.0.0.1:0".parse().unwrap(), feed);

        valid.validate().unwrap();

        let mut config = valid;
        config.poll_interval = Duration::ZERO;
        assert!(config.validate().is_err());

        let mut config = valid;
        config.receive_budget = 0;
        assert!(config.validate().is_err());

        let mut config = valid;
        config.status_interval = Some(Duration::ZERO);
        assert!(config.validate().is_err());
    }

    #[test]
    fn idle_runtime_stops_and_shuts_down() {
        let options = parse(&[
            "--listen",
            "127.0.0.1:0",
            "--idle-timeout-ms",
            "20",
            "--instruments",
            "1",
            "--max-orders",
            "8",
        ])
        .unwrap();

        let mut runtime = options.bind().unwrap();

        assert_ne!(runtime.local_addr().unwrap().port(), 0);

        let summary = runtime.run(&mut io::sink()).unwrap();

        assert_eq!(summary.reason, StopReason::Idle);
        assert_eq!(summary.stats, RuntimeStats::default());
        assert!(summary.synchronized);
        assert_eq!(summary.next_expected, SequenceNumber::ONE);
        assert_eq!(runtime.server().state(), ServerState::Stopped);
    }
}
