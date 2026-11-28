//! Simulator to server tests over real UDP sockets.
//!
//! The simulator and the server run on separate threads, and
//! in one test as separate operating system processes. They
//! share nothing but a loopback UDP path.
//!
//! Each test compares the complete resting order state the
//! server reconstructed with the simulator's authoritative
//! market.

use std::io;
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use maxwell_core::{FeedId, SessionId, StreamId};

use maxwell_server::config::ServerConfig;
use maxwell_server::error::ServerResult;
use maxwell_server::runtime::{
    DEFAULT_REQUEST_BATCH, MarketSpec, RunSummary, Runtime, RuntimeConfig, StopReason,
};
use maxwell_server::server::Server;
use maxwell_server::state::ServerState;

use maxwell_simulator::config::{PacingMode, SimulatorConfig};
use maxwell_simulator::market::Market;
use maxwell_simulator::network::{NetworkConfig, NetworkRunner, NetworkStats};
use maxwell_simulator::scenario::Scenario;
use maxwell_simulator::simulator::{NullSink, RuntimeLimits, Simulator};

// ============================================================
// Fixtures
// ============================================================

const EVENT_RATE: u64 = 40_000;

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(1))
}

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// Scenario configuration paced for network delivery.
fn config(scenario: Scenario, event_count: u64) -> SimulatorConfig {
    let mut config = scenario.config().unwrap();

    config.run.event_count = event_count;
    config.pacing.mode = PacingMode::RealTime;
    config.pacing.events_per_second = EVENT_RATE;
    config.pacing.event_interval_ns = 1_000_000_000 / EVENT_RATE;

    config.validate().unwrap();

    config
}

/// Small generation batches avoid bursts on the socket.
fn limits() -> RuntimeLimits {
    RuntimeLimits {
        generation_budget: 8,
        ..RuntimeLimits::default()
    }
}

/// Server market parameters for a simulator configuration.
fn market_spec(config: &SimulatorConfig) -> MarketSpec {
    MarketSpec {
        instrument_count: config.market.instrument_count,
        max_orders_per_instrument: config.market.max_orders_per_instrument,
        reference_price: config.market.reference_price,
        tick_size: config.market.tick_size,
        lot_size: config.market.lot_size,
        max_price_distance_ticks: config.market.max_price_distance_ticks,
        max_order_quantity_lots: config.market.max_order_quantity_lots,
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
    /// Binds a server on an ephemeral loopback port and runs
    /// it on its own thread until the feed is complete.
    fn start(config: &SimulatorConfig) -> Self {
        let market = market_spec(config);

        let mut server_config = ServerConfig::default();

        market.size_books(&mut server_config).unwrap();
        server_config.recovery.max_request_batch = DEFAULT_REQUEST_BATCH;

        let mut runtime_config = RuntimeConfig::new(loopback(), feed());

        runtime_config.exit_on_final = true;
        runtime_config.idle_timeout = Some(Duration::from_secs(20));

        let mut runtime = Runtime::bind(
            runtime_config,
            server_config,
            &market.instruments().unwrap(),
        )
        .unwrap();

        let address = runtime.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let result = runtime.run(&mut io::sink());

            (runtime, result)
        });

        Self { address, handle }
    }

    fn finish(self) -> (Runtime, RunSummary) {
        let (runtime, result) = self.handle.join().unwrap();

        (runtime, result.unwrap())
    }
}

// ============================================================
// State comparison
// ============================================================

/// Compares every resting order in the server's books with
/// the simulator's authoritative market.
fn assert_market_matches(market: &Market, server: &Server) {
    assert!(server.is_synchronized());

    assert_eq!(
        market.next_expected(),
        server.recovery().next_expected(),
        "upstream sequence cursors differ",
    );

    let books = server.pipeline().books();

    assert_eq!(books.instrument_count(), market.instrument_count());

    let mut reconstructed_orders = 0;

    for authoritative in market.instruments() {
        let reconstructed = books
            .book(authoritative.id())
            .expect("missing reconstructed instrument");

        assert_eq!(authoritative.instrument(), reconstructed.instrument());

        assert_eq!(
            authoritative.last_sequence(),
            reconstructed.last_sequence(),
            "instrument sequence mismatch",
        );

        assert_eq!(
            authoritative.len(),
            reconstructed.order_count(),
            "resting order count mismatch",
        );

        for expected in authoritative.orders() {
            let actual = reconstructed
                .orders()
                .get(expected.order_id)
                .expect("missing reconstructed order");

            assert_eq!(actual.side(), expected.side, "order side mismatch");
            assert_eq!(actual.price(), expected.price, "order price mismatch");

            assert_eq!(
                actual.quantity(),
                expected.quantity,
                "order quantity mismatch",
            );
        }

        reconstructed.validate_consistency().unwrap();

        assert!(!reconstructed.is_crossed());

        reconstructed_orders += reconstructed.order_count();
    }

    assert_eq!(reconstructed_orders, market.total_orders());

    market.validate_consistency().unwrap();
}

// ============================================================
// Thread-to-thread runs
// ============================================================

struct Outcome {
    simulator: Simulator,
    network: NetworkStats,
    runtime: Runtime,
    summary: RunSummary,
}

/// Runs one simulation against a server on another thread and
/// checks the reconstructed market.
fn run(scenario: Scenario, event_count: u64) -> Outcome {
    let config = config(scenario, event_count);
    let server = ServerThread::start(&config);

    let mut simulator = Simulator::new(config, feed(), limits()).unwrap();

    let mut network = NetworkConfig::new(loopback(), server.address);

    network.linger = Duration::from_secs(10);

    let mut runner = NetworkRunner::bind(network, feed()).unwrap();

    let network = runner.run(&mut simulator).unwrap();
    let (runtime, summary) = server.finish();

    assert_eq!(summary.reason, StopReason::FeedComplete);
    assert!(summary.synchronized);
    assert_eq!(summary.next_expected.raw(), event_count + 1);
    assert_eq!(summary.stats.rejected, 0);
    assert_eq!(summary.stats.foreign, 0);
    assert_eq!(runtime.server().state(), ServerState::Stopped);

    assert!(network.consumer_complete);
    assert_eq!(network.requests_unavailable, 0);

    simulator.validate_consistency().unwrap();

    assert_market_matches(simulator.market(), runtime.server());

    Outcome {
        simulator,
        network,
        runtime,
        summary,
    }
}

#[test]
fn clean_feed_is_reconstructed_without_recovery() {
    let outcome = run(Scenario::Smoke, 2_000);

    assert_eq!(outcome.network.events_sent, 2_000);
    assert_eq!(outcome.summary.stats.duplicates, 0);

    // The loopback path itself may drop a datagram under load,
    // so recovery is permitted but must then have succeeded.
    assert_eq!(
        outcome.summary.stats.retransmit_requests,
        outcome.network.requests_received,
    );

    assert_eq!(outcome.simulator.watermark().raw(), 2_000);
    assert!(outcome.runtime.stats().heartbeats >= 1);
}

#[test]
fn dropped_events_are_recovered_by_retransmission() {
    let outcome = run(Scenario::PacketLoss, 5_000);

    // The injector drops live events before they reach the
    // socket, so fewer than the full stream was sent live.
    assert!(outcome.network.events_sent < 5_000);

    assert!(outcome.summary.stats.retransmit_requests > 0);
    assert!(outcome.network.events_retransmitted > 0);
    assert!(outcome.summary.stats.resynchronizations > 0);
}

#[test]
fn duplicated_events_are_applied_once() {
    let outcome = run(Scenario::Duplicates, 5_000);

    assert!(outcome.network.events_sent > 5_000);
    assert!(outcome.summary.stats.duplicates > 0);
}

#[test]
fn reordered_events_are_applied_in_sequence() {
    let outcome = run(Scenario::OutOfOrder, 5_000);

    assert!(outcome.summary.stats.buffered > 0);
    assert!(outcome.summary.stats.resynchronizations > 0);
}

#[test]
fn combined_faults_are_recovered() {
    let outcome = run(Scenario::MixedFaults, 5_000);

    assert!(outcome.summary.stats.retransmit_requests > 0);
    assert!(outcome.summary.stats.duplicates > 0);
    assert!(outcome.summary.stats.buffered > 0);
}

#[test]
fn heavy_faults_are_recovered() {
    let outcome = run(Scenario::RecoveryStress, 5_000);

    assert!(outcome.summary.stats.retransmit_requests > 0);
    assert!(outcome.network.events_retransmitted > 0);
}

// ============================================================
// Process-to-process run
// ============================================================

/// The simulator executable feeds a server in this process.
///
/// The executable's final market cannot be inspected, so an
/// identically configured simulator supplies the reference
/// state. Generation is deterministic for a given seed.
#[test]
fn simulator_process_feeds_server() {
    const EVENTS: u64 = 3_000;

    let reference_config = {
        let mut config = Scenario::MixedFaults.config().unwrap();

        config.run.event_count = EVENTS;
        config.run.seed = 7;
        config.market.instrument_count = 3;
        config.market.max_orders_per_instrument = 128;
        config
    };

    let server = ServerThread::start(&reference_config);

    let output = Command::new(env!("CARGO_BIN_EXE_maxwell-simulator"))
        .args(["--scenario", "mixed-faults"])
        .args(["--events", &EVENTS.to_string()])
        .args(["--seed", "7"])
        .args(["--instruments", "3"])
        .args(["--max-orders", "128"])
        .args(["--pacing", "real-time"])
        .args(["--rate", &EVENT_RATE.to_string()])
        .args(["--target", &server.address.to_string()])
        .args(["--bind", "127.0.0.1:0"])
        .args(["--linger-ms", "10000"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "simulator failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    assert!(stdout.contains("Consumer complete:      true"), "{stdout}");
    assert!(stdout.contains("Simulation completed successfully."));

    let (runtime, summary) = server.finish();

    assert_eq!(summary.reason, StopReason::FeedComplete);
    assert_eq!(summary.next_expected.raw(), EVENTS + 1);
    assert!(summary.stats.retransmit_requests > 0);

    let mut reference = Simulator::new(reference_config, feed(), RuntimeLimits::default()).unwrap();

    let report = reference.run_until_blocked(&mut NullSink).unwrap();

    assert!(report.is_completed());

    assert_market_matches(reference.market(), runtime.server());
}
