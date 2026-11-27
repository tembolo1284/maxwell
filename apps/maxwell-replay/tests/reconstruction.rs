//! End-to-end deterministic market reconstruction.
//!
//! The simulator generates an independent authoritative
//! market state. Events are delivered to a live server
//! and persisted in an authoritative journal.
//!
//! The journal is subsequently replayed into a fresh
//! server, and both reconstructed states are compared
//! against the simulator's authoritative market.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use maxwell_core::{EventEnvelope, FeedId, MonotonicTimestamp, SessionId, StreamId};

use maxwell_replay::{
    JournalReader, JournalRecord, PlaybackMode, ReaderConfig, Recorder, ReplayConfig, ReplayEngine,
    ReplayError, ReplayFilter, ReplayResult, ReplaySink, ReplayStatus, SinkStatus, WriterConfig,
};

use maxwell_server::config::ServerConfig;
use maxwell_server::recovery::RecoveryOutcome;
use maxwell_server::server::Server;

use maxwell_simulator::market::Market;
use maxwell_simulator::scenario::Scenario;
use maxwell_simulator::simulator::{DeliveryOutcome, EventSink, RuntimeLimits, Simulator};
use maxwell_simulator::{SimulatorError, SimulatorResult};

// ============================================================
// Test resources
// ============================================================

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct JournalFile {
    path: PathBuf,
}

impl JournalFile {
    fn new() -> Self {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);

        let path = std::env::temp_dir().join(format!(
            "maxwell-reconstruction-{}-{id}.mxj",
            std::process::id(),
        ));

        Self { path }
    }
}

impl Drop for JournalFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

// ============================================================
// Fixtures
// ============================================================

fn feed() -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(1))
}

fn make_simulator(event_count: u64) -> Simulator {
    let mut config = Scenario::Smoke.config().unwrap();

    config.run.event_count = event_count;

    Simulator::new(config, feed(), RuntimeLimits::default()).unwrap()
}

fn make_server(market: &Market) -> Server {
    let mut config = ServerConfig::default();

    config.book.max_instruments = market.instrument_count();

    config.book.max_orders = market.config().max_orders_per_instrument;

    // Dense price ladders must accommodate the complete
    // instrument price band.
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
// Live recording adapter
// ============================================================

/// Delivers clean simulator events to the original server
/// and records accepted events in the journal.
///
/// This adapter is intended for deterministic integration
/// testing, not transactional production persistence.
///
/// If recording fails after server acceptance, the test
/// terminates rather than retrying an already applied event.
struct RecordingServerSink<'a> {
    server: &'a mut Server,
    recorder: &'a mut Recorder,
    accepted: u64,
}

impl<'a> RecordingServerSink<'a> {
    fn new(server: &'a mut Server, recorder: &'a mut Recorder) -> Self {
        Self {
            server,
            recorder,
            accepted: 0,
        }
    }
}

impl EventSink for RecordingServerSink<'_> {
    fn try_send(
        &mut self,
        event: &EventEnvelope,
        received_at: MonotonicTimestamp,
    ) -> SimulatorResult<DeliveryOutcome> {
        let outcome = self
            .server
            .ingest_event(*event, received_at)
            .map_err(|error| SimulatorError::Io(std::io::Error::other(error)))?;

        match outcome {
            RecoveryOutcome::InputBlocked => {
                return Ok(DeliveryOutcome::Blocked);
            }

            RecoveryOutcome::Published { .. } | RecoveryOutcome::PublicationPending => {}

            unexpected => {
                return Err(SimulatorError::Io(std::io::Error::other(format!(
                    "unexpected clean delivery outcome: \
                         {unexpected:?}"
                ))));
            }
        }

        self.recorder
            .record_at(*event, received_at.raw())
            .map_err(|error| SimulatorError::Io(std::io::Error::other(error)))?;

        self.accepted += 1;

        Ok(DeliveryOutcome::Accepted)
    }
}

// ============================================================
// Historical replay adapter
// ============================================================

/// Delivers historical records to a fresh server.
///
/// InputBlocked means the event has not been consumed.
///
/// PublicationPending means the event has already been
/// applied and must not be submitted a second time.
struct HistoricalServerSink<'a> {
    server: &'a mut Server,
    accepted: u64,
}

impl<'a> HistoricalServerSink<'a> {
    fn new(server: &'a mut Server) -> Self {
        Self {
            server,
            accepted: 0,
        }
    }
}

impl ReplaySink for HistoricalServerSink<'_> {
    fn deliver(&mut self, record: &JournalRecord) -> ReplayResult<SinkStatus> {
        let received_at = MonotonicTimestamp::new(record.recording_timestamp_ns);

        let outcome = self
            .server
            .ingest_event(record.event, received_at)
            .map_err(|error| ReplayError::SinkRejected {
                reason: error.to_string(),
            })?;

        match outcome {
            RecoveryOutcome::InputBlocked => Ok(SinkStatus::Backpressured),

            RecoveryOutcome::Published { .. } | RecoveryOutcome::PublicationPending => {
                self.accepted += 1;

                Ok(SinkStatus::Accepted)
            }

            unexpected => Err(ReplayError::SinkRejected {
                reason: format!(
                    "unexpected authoritative replay \
                         outcome: {unexpected:?}"
                ),
            }),
        }
    }
}

// ============================================================
// Independent market comparison
// ============================================================

fn assert_market_matches(market: &Market, server: &Server) {
    assert!(
        server.is_synchronized(),
        "server must be synchronized before comparison",
    );

    assert_eq!(
        market.next_expected(),
        server.recovery().next_expected(),
        "upstream sequence cursor mismatch",
    );

    let books = server.pipeline().books();

    assert_eq!(
        books.instrument_count(),
        market.instrument_count(),
        "instrument count mismatch",
    );

    let mut reconstructed_orders = 0;

    for authoritative in market.instruments() {
        let reconstructed = books
            .book(authoritative.id())
            .expect("missing reconstructed instrument");

        assert_eq!(
            authoritative.instrument(),
            reconstructed.instrument(),
            "instrument definition mismatch",
        );

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

        assert!(!reconstructed.is_crossed(), "reconstructed book is crossed",);

        reconstructed_orders += reconstructed.order_count();
    }

    assert_eq!(
        reconstructed_orders,
        market.total_orders(),
        "total resting order count mismatch",
    );

    market.validate_consistency().unwrap();
}

// ============================================================
// Server-to-server comparison
// ============================================================

fn assert_servers_match(market: &Market, original: &Server, reconstructed: &Server) {
    assert_market_matches(market, original);
    assert_market_matches(market, reconstructed);

    assert_eq!(
        original.recovery().next_expected(),
        reconstructed.recovery().next_expected(),
    );

    assert_eq!(
        original.status().publication_watermark,
        reconstructed.status().publication_watermark,
        "publication watermark mismatch",
    );

    let original_books = original.pipeline().books();
    let replayed_books = reconstructed.pipeline().books();

    for instrument in market.instruments() {
        let original_book = original_books
            .book(instrument.id())
            .expect("missing original instrument");

        let replayed_book = replayed_books
            .book(instrument.id())
            .expect("missing replayed instrument");

        assert_eq!(original_book.last_sequence(), replayed_book.last_sequence(),);

        assert_eq!(original_book.order_count(), replayed_book.order_count(),);

        for expected in instrument.orders() {
            let original_order = original_book
                .orders()
                .get(expected.order_id)
                .expect("missing original order");

            let replayed_order = replayed_book
                .orders()
                .get(expected.order_id)
                .expect("missing replayed order");

            assert_eq!(original_order.order_id(), replayed_order.order_id(),);

            assert_eq!(original_order.side(), replayed_order.side(),);

            assert_eq!(original_order.price(), replayed_order.price(),);

            assert_eq!(original_order.quantity(), replayed_order.quantity(),);
        }
    }
}

// ============================================================
// Recording helpers
// ============================================================

fn record_simulation(
    simulator: &mut Simulator,
    original: &mut Server,
    journal: &JournalFile,
    expected_events: u64,
) {
    let mut recorder = Recorder::create(
        &journal.path,
        WriterConfig::default(),
        simulator.market().feed(),
        1,
    )
    .unwrap();

    {
        let mut sink = RecordingServerSink::new(original, &mut recorder);

        let report = simulator.run_until_blocked(&mut sink).unwrap();

        assert!(report.is_completed());

        assert_eq!(sink.accepted, expected_events,);
    }

    let stats = recorder.finish(false).unwrap();

    assert_eq!(stats.records_written, expected_events,);

    assert_eq!(stats.last_sequence, Some(expected_events),);

    assert_market_matches(simulator.market(), original);
}

fn replay_into_server(
    journal: &JournalFile,
    server: &mut Server,
    expected_events: u64,
    max_records_per_step: usize,
) {
    let config = ReplayConfig {
        playback_mode: PlaybackMode::Virtual,
        max_records_per_step,
        ..ReplayConfig::default()
    };

    let mut engine = ReplayEngine::open(&journal.path, config, ReplayFilter::default()).unwrap();

    engine.require_authoritative_stream().unwrap();

    let mut sink = HistoricalServerSink::new(server);

    loop {
        match engine.step(&mut sink).unwrap() {
            ReplayStatus::Progress => {}

            ReplayStatus::Finished => break,

            unexpected => {
                panic!(
                    "unexpected reconstruction status: \
                     {unexpected:?}"
                );
            }
        }
    }

    assert_eq!(sink.accepted, expected_events,);

    assert_eq!(engine.stats().records_read, expected_events,);

    assert_eq!(engine.stats().records_delivered, expected_events,);

    assert_eq!(engine.stats().records_filtered, 0,);

    assert_eq!(engine.stats().backpressure_attempts, 0,);

    assert_eq!(
        engine.stats().last_delivered_sequence,
        Some(expected_events),
    );

    assert!(!engine.incomplete_tail());
}

// ============================================================
// Integration tests
// ============================================================

#[test]
fn journal_reconstructs_authoritative_market() {
    const EVENT_COUNT: u64 = 10_000;

    let journal = JournalFile::new();

    let mut simulator = make_simulator(EVENT_COUNT);

    let mut original = make_server(simulator.market());

    let mut reconstructed = make_server(simulator.market());

    record_simulation(&mut simulator, &mut original, &journal, EVENT_COUNT);

    replay_into_server(&journal, &mut reconstructed, EVENT_COUNT, 256);

    assert_servers_match(simulator.market(), &original, &reconstructed);
}

#[test]
fn reconstruction_is_independent_of_replay_batch_size() {
    const EVENT_COUNT: u64 = 10_000;

    let journal = JournalFile::new();

    let mut simulator = make_simulator(EVENT_COUNT);

    let mut original = make_server(simulator.market());

    record_simulation(&mut simulator, &mut original, &journal, EVENT_COUNT);

    for batch_size in [1, 7, 64, 256, 4_096] {
        let mut reconstructed = make_server(simulator.market());

        replay_into_server(&journal, &mut reconstructed, EVENT_COUNT, batch_size);

        assert_servers_match(simulator.market(), &original, &reconstructed);
    }
}

#[test]
fn repeated_reconstruction_produces_identical_state() {
    const EVENT_COUNT: u64 = 5_000;

    let journal = JournalFile::new();

    let mut simulator = make_simulator(EVENT_COUNT);

    let mut original = make_server(simulator.market());

    record_simulation(&mut simulator, &mut original, &journal, EVENT_COUNT);

    for _ in 0..3 {
        let mut reconstructed = make_server(simulator.market());

        replay_into_server(&journal, &mut reconstructed, EVENT_COUNT, 128);

        assert_servers_match(simulator.market(), &original, &reconstructed);
    }
}

#[test]
fn recorded_simulation_preserves_sequence_continuity() {
    const EVENT_COUNT: u64 = 10_000;

    let journal = JournalFile::new();

    let mut simulator = make_simulator(EVENT_COUNT);

    let mut original = make_server(simulator.market());

    record_simulation(&mut simulator, &mut original, &journal, EVENT_COUNT);

    let mut reader = JournalReader::open(&journal.path, ReaderConfig::default()).unwrap();

    let mut expected_sequence = 1_u64;
    let mut previous_timestamp = 0_u64;

    while let Some(record) = reader.next_record().unwrap() {
        assert_eq!(record.sequence(), expected_sequence,);

        assert!(record.recording_timestamp_ns >= previous_timestamp,);

        previous_timestamp = record.recording_timestamp_ns;

        expected_sequence += 1;
    }

    assert_eq!(expected_sequence, EVENT_COUNT + 1,);

    assert_eq!(reader.records_read(), EVENT_COUNT,);

    assert!(!reader.incomplete_tail());
}
