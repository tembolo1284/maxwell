//! Integrated Maxwell simulator and server tests.
//!
//! The simulator maintains independent authoritative
//! market state. The server reconstructs that state
//! from delivered upstream events.
//!
//! Tests compare the complete resting order state
//! after clean delivery and recoverable delivery faults.

use maxwell_core::{
    EventEnvelope, FeedId, MonotonicTimestamp, SequenceNumber, SessionId, StreamId,
};

use maxwell_server::config::ServerConfig;
use maxwell_server::recovery::RecoveryOutcome;
use maxwell_server::server::Server;

use maxwell_simulator::config::{BASIS_POINTS, FaultConfig, SimulatorConfig};
use maxwell_simulator::market::Market;
use maxwell_simulator::scenario::Scenario;
use maxwell_simulator::simulator::{
    DeliveryOutcome, EventSink, NullSink, RuntimeLimits, Simulator,
};
use maxwell_simulator::{SimulatorError, SimulatorResult};

// ============================================================
// Fixtures
// ============================================================

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(1))
}

fn config(event_count: u64) -> SimulatorConfig {
    let mut config = Scenario::Smoke.config().unwrap();

    config.run.event_count = event_count;

    config
}

fn simulator(config: SimulatorConfig) -> Simulator {
    Simulator::new(config, feed(), RuntimeLimits::default()).unwrap()
}

fn server(market: &Market) -> Server {
    let mut config = ServerConfig::default();

    // Keep integration test allocations proportional
    // to the simulated workload.
    config.book.max_instruments = market.instrument_count();

    config.book.max_orders = market.config().max_orders_per_instrument;

    // Maxwell uses dense price ladders. Capacity must
    // accommodate the complete instrument price band,
    // not merely the number of resting orders.
    config.book.max_price_levels = market
        .instruments()
        .iter()
        .map(|instrument| {
            usize::try_from(
                instrument
                    .instrument()
                    .price_level_count()
                    .expect("invalid instrument price band"),
            )
            .expect("price level count exceeds usize")
        })
        .max()
        .expect("market must contain instruments");

    let mut server = Server::new(config, market.feed()).unwrap();

    for instrument in market.instruments() {
        server
            .register_instrument(*instrument.instrument())
            .unwrap();
    }

    server.start_empty(MonotonicTimestamp::new(0)).unwrap();

    server
}

// ============================================================
// Server delivery adapter
// ============================================================

/// In-process adapter between the simulator and server.
///
/// The adapter distinguishes an unconsumed input event
/// from an event whose publication remains pending.
struct ServerSink<'a> {
    server: &'a mut Server,

    accepted: u64,
    duplicates: u64,
    buffered: u64,
    resynchronized: u64,
    last_timestamp_ns: u64,
}

impl<'a> ServerSink<'a> {
    fn new(server: &'a mut Server) -> Self {
        Self {
            server,

            accepted: 0,
            duplicates: 0,
            buffered: 0,
            resynchronized: 0,
            last_timestamp_ns: 0,
        }
    }
}

impl EventSink for ServerSink<'_> {
    fn try_send(
        &mut self,
        event: &EventEnvelope,
        received_at: MonotonicTimestamp,
    ) -> SimulatorResult<DeliveryOutcome> {
        let outcome = self
            .server
            .ingest_event(*event, received_at)
            .map_err(|error| SimulatorError::Io(std::io::Error::other(error)))?;

        self.last_timestamp_ns = received_at.raw();

        match outcome {
            RecoveryOutcome::InputBlocked => {
                return Ok(DeliveryOutcome::Blocked);
            }

            RecoveryOutcome::Duplicate => {
                self.duplicates += 1;
            }

            RecoveryOutcome::Buffered => {
                self.buffered += 1;
            }

            RecoveryOutcome::Resynchronized => {
                self.resynchronized += 1;
            }

            RecoveryOutcome::Published { .. }
            | RecoveryOutcome::PublicationPending
            | RecoveryOutcome::Replayed
            | RecoveryOutcome::RecoveryRequired => {}
        }

        self.accepted += 1;

        Ok(DeliveryOutcome::Accepted)
    }
}

// ============================================================
// Independent state comparison
// ============================================================

/// Compares every authoritative resting order against
/// its reconstructed server representation.
///
/// Order priority is intentionally excluded until the
/// authoritative simulator retains priority sequences.
fn assert_market_matches(market: &Market, server: &Server) {
    assert!(
        server.is_synchronized(),
        "server must be synchronized before comparison",
    );

    assert_eq!(
        market.next_expected(),
        server.recovery().next_expected(),
        "upstream sequence cursors differ",
    );

    let books = server.pipeline().books();

    assert_eq!(books.instrument_count(), market.instrument_count(),);

    let mut reconstructed_orders = 0;

    for authoritative in market.instruments() {
        let reconstructed = books
            .book(authoritative.id())
            .expect("missing reconstructed instrument");

        assert_eq!(authoritative.instrument(), reconstructed.instrument(),);

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

            assert_eq!(actual.order_id(), expected.order_id,);

            assert_eq!(actual.side(), expected.side, "order side mismatch",);

            assert_eq!(actual.price(), expected.price, "order price mismatch",);

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

    assert_eq!(reconstructed_orders, market.total_orders(),);

    market.validate_consistency().unwrap();
}

// ============================================================
// Clean market reconstruction
// ============================================================

#[test]
fn clean_simulation_reconstructs_authoritative_market() {
    let mut simulator = simulator(config(10_000));

    let mut server = server(simulator.market());

    {
        let mut sink = ServerSink::new(&mut server);

        let report = simulator.run_until_blocked(&mut sink).unwrap();

        assert!(report.is_completed());
        assert_eq!(sink.accepted, 10_000);
        assert_eq!(sink.duplicates, 0);
        assert_eq!(sink.buffered, 0);
    }

    assert_market_matches(simulator.market(), &server);

    assert_eq!(server.status().publication_watermark, 10_000,);

    assert!(simulator.stats().is_fully_accounted());
}

#[test]
fn clean_reconstruction_matches_after_every_iteration() {
    let config = config(1_000);

    let limits = RuntimeLimits {
        generation_budget: 16,
        delivery_budget: 16,
        ..RuntimeLimits::default()
    };

    let mut simulator = Simulator::new(config, feed(), limits).unwrap();

    let mut server = server(simulator.market());

    simulator.start().unwrap();

    loop {
        let report = {
            let mut sink = ServerSink::new(&mut server);

            simulator.service(&mut sink).unwrap()
        };

        assert_market_matches(simulator.market(), &server);

        if report.is_completed() {
            break;
        }

        assert!(report.made_progress());
    }

    assert_eq!(simulator.generated(), 1_000);
}

// ============================================================
// Duplicate suppression
// ============================================================

#[test]
fn duplicate_delivery_does_not_mutate_market_twice() {
    let mut config = config(10_000);

    config.faults = FaultConfig {
        duplicate_bps: BASIS_POINTS,
        ..FaultConfig::disabled()
    };

    let mut simulator = simulator(config);

    let mut server = server(simulator.market());

    {
        let mut sink = ServerSink::new(&mut server);

        let report = simulator.run_until_blocked(&mut sink).unwrap();

        assert!(report.is_completed());
        assert_eq!(sink.accepted, 20_000);
        assert_eq!(sink.duplicates, 10_000);
    }

    assert_market_matches(simulator.market(), &server);

    // Duplicates must not generate new publications.
    assert_eq!(server.status().publication_watermark, 10_000,);
}

// ============================================================
// Out-of-order delivery
// ============================================================

#[test]
fn reordered_delivery_recovers_authoritative_market() {
    let mut config = config(5_003);

    config.faults = FaultConfig {
        reorder_bps: BASIS_POINTS,
        reorder_window: 4,
        ..FaultConfig::disabled()
    };

    let mut simulator = simulator(config);

    let mut server = server(simulator.market());

    {
        let mut sink = ServerSink::new(&mut server);

        let report = simulator.run_until_blocked(&mut sink).unwrap();

        assert!(report.is_completed());
        assert_eq!(sink.accepted, 5_003);

        assert!(sink.buffered > 0);
        assert!(sink.resynchronized > 0);
    }

    assert_market_matches(simulator.market(), &server);

    assert!(simulator.stats().is_fully_accounted());
}

#[test]
fn duplicates_and_reordering_preserve_market_state() {
    let mut config = config(10_000);

    config.faults = FaultConfig {
        drop_bps: 0,
        duplicate_bps: 2_000,
        reorder_bps: 3_000,
        reorder_window: 8,
    };

    let mut simulator = simulator(config);

    let mut server = server(simulator.market());

    {
        let mut sink = ServerSink::new(&mut server);

        let report = simulator.run_until_blocked(&mut sink).unwrap();

        assert!(report.is_completed());

        assert!(sink.accepted > 10_000);
        assert!(sink.buffered > 0);
        assert!(sink.resynchronized > 0);
    }

    assert_market_matches(simulator.market(), &server);
}

// ============================================================
// Packet loss recovery
// ============================================================

#[test]
fn missing_event_is_recovered_from_authoritative_history() {
    let mut simulator = simulator(config(3));

    // Generate the authoritative workload without
    // delivering it to the server.
    simulator.run_until_blocked(&mut NullSink).unwrap();

    let mut server = server(simulator.market());

    let first = *simulator.history().get(SequenceNumber::new(1)).unwrap();

    let third = *simulator.history().get(SequenceNumber::new(3)).unwrap();

    server
        .ingest_event(first, MonotonicTimestamp::new(1))
        .unwrap();

    assert!(server.is_synchronized());

    // Simulate loss of sequence two.
    server
        .ingest_event(third, MonotonicTimestamp::new(2))
        .unwrap();

    assert!(!server.is_synchronized());
    assert!(!server.is_publishable());

    assert_eq!(server.recovery().next_expected(), SequenceNumber::new(2),);

    // Recover the missing authoritative event.
    let missing = *simulator
        .retransmission(SequenceNumber::new(2), SequenceNumber::new(2))
        .unwrap()
        .next()
        .unwrap();

    let outcome = server
        .ingest_event(missing, MonotonicTimestamp::new(3))
        .unwrap();

    assert_eq!(outcome, RecoveryOutcome::Resynchronized,);

    assert_market_matches(simulator.market(), &server);
}

#[test]
fn multiple_missing_events_are_recovered_in_sequence() {
    let mut simulator = simulator(config(10));

    simulator.run_until_blocked(&mut NullSink).unwrap();

    let mut server = server(simulator.market());

    let first = *simulator.history().get(SequenceNumber::ONE).unwrap();

    server
        .ingest_event(first, MonotonicTimestamp::new(1))
        .unwrap();

    // Withhold sequences two through five.
    let mut now = 2;

    for sequence in 6..=10 {
        let event = *simulator
            .history()
            .get(SequenceNumber::new(sequence))
            .unwrap();

        server
            .ingest_event(event, MonotonicTimestamp::new(now))
            .unwrap();

        now += 1;
    }

    assert!(!server.is_synchronized());

    assert_eq!(server.recovery().next_expected().raw(), 2,);

    let replay = simulator
        .retransmission(SequenceNumber::new(2), SequenceNumber::new(5))
        .unwrap();

    for event in replay {
        server
            .ingest_event(*event, MonotonicTimestamp::new(now))
            .unwrap();

        now += 1;
    }

    assert_market_matches(simulator.market(), &server);

    assert_eq!(server.recovery().next_expected().raw(), 11,);
}

// ============================================================
// Mixed delivery fault recovery
// ============================================================

/// Records the faulty delivery stream for controlled
/// integration tests.
#[derive(Default)]
struct RecordingSink {
    events: Vec<EventEnvelope>,
}

impl EventSink for RecordingSink {
    fn try_send(
        &mut self,
        event: &EventEnvelope,
        _received_at: MonotonicTimestamp,
    ) -> SimulatorResult<DeliveryOutcome> {
        self.events.push(*event);

        Ok(DeliveryOutcome::Accepted)
    }
}

#[test]
fn mixed_faults_recover_using_retained_history() {
    let mut config = config(10_000);

    config.faults = FaultConfig {
        drop_bps: 500,
        duplicate_bps: 1_000,
        reorder_bps: 2_000,
        reorder_window: 8,
    };

    let mut simulator = simulator(config);

    let mut recording = RecordingSink::default();

    simulator.run_until_blocked(&mut recording).unwrap();

    let mut server = server(simulator.market());

    let mut now = 1u64;
    let mut recovered_events = 0u64;

    for event in recording.events {
        server
            .ingest_event(event, MonotonicTimestamp::new(now))
            .unwrap();

        now += 1;

        // Immediately recover the missing range whenever
        // the server detects a sequence gap.
        //
        // This test uses canonical event admission,
        // not the request-correlated recovery transport.
        while server.recovery().next_expected() < event.sequence() {
            let expected = server.recovery().next_expected();

            let missing = *simulator
                .history()
                .get(expected)
                .expect("required history must be retained");

            server
                .ingest_event(missing, MonotonicTimestamp::new(now))
                .unwrap();

            now += 1;
            recovered_events += 1;

            assert!(
                server.recovery().next_expected() > expected,
                "recovery must advance the sequence cursor",
            );
        }
    }

    // Recover any missing tail events. Without a later
    // sequence or explicit upstream watermark, the
    // server cannot independently detect trailing loss.
    while server.recovery().next_expected() <= simulator.watermark() {
        let expected = server.recovery().next_expected();

        let missing = *simulator
            .history()
            .get(expected)
            .expect("missing tail history must be retained");

        server
            .ingest_event(missing, MonotonicTimestamp::new(now))
            .unwrap();

        now += 1;
        recovered_events += 1;

        assert!(server.recovery().next_expected() > expected,);
    }

    assert!(recovered_events > 0);

    assert_market_matches(simulator.market(), &server);

    assert!(simulator.stats().is_fully_accounted());
}

// ============================================================
// Protocol integration
// ============================================================

#[test]
fn encoded_events_reconstruct_authoritative_market() {
    use maxwell_protocol::Encoder;

    let mut simulator = simulator(config(1_000));

    simulator.run_until_blocked(&mut NullSink).unwrap();

    let mut server = server(simulator.market());

    // Reuse the encoder and fixed-size encoding buffer
    // across all authoritative events.
    let encoder = Encoder;
    let mut buffer = [0u8; 1_024];

    for event in simulator.history().iter() {
        let length = encoder.encode_event(*event, &mut buffer).unwrap();

        server
            .ingest_payload(
                &buffer[..length],
                MonotonicTimestamp::new(event.sequence().raw()),
            )
            .unwrap();
    }

    assert_market_matches(simulator.market(), &server);
}

// ============================================================
// Extended workload
// ============================================================

#[test]
fn extended_order_churn_reconstructs_authoritative_market() {
    let mut config = Scenario::OrderChurn.config().unwrap();

    config.run.event_count = 100_000;
    config.market.instrument_count = 4;
    config.market.max_orders_per_instrument = 256;

    let mut simulator = simulator(config);

    let mut server = server(simulator.market());

    {
        let mut sink = ServerSink::new(&mut server);

        let report = simulator.run_until_blocked(&mut sink).unwrap();

        assert!(report.is_completed());
        assert_eq!(sink.accepted, 100_000);
    }

    assert_market_matches(simulator.market(), &server);

    simulator.validate_consistency().unwrap();
}

#[test]
fn capacity_stress_reconstructs_authoritative_market() {
    let mut config = Scenario::CapacityStress.config().unwrap();

    config.run.event_count = 25_000;

    let mut simulator = simulator(config);

    let mut server = server(simulator.market());

    {
        let mut sink = ServerSink::new(&mut server);

        let report = simulator.run_until_blocked(&mut sink).unwrap();

        assert!(report.is_completed());
    }

    assert_market_matches(simulator.market(), &server);

    assert!(simulator.market().total_orders() <= 16,);
}
