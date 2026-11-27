//! Deterministic client replay and resource-bound tests.

use maxwell_client::{
    Client, ClientConfig, ClientState, ClientStats, InboundFrame, InboundMessage,
    MarketDataHandler, ReceiveOutcome, RecoveryReason,
};

use maxwell_gateway::ServerMessage;
use maxwell_publisher::{EventKind, Publication};

#[derive(Debug, Default, PartialEq, Eq)]
struct ReplayHandler {
    valid: bool,
    snapshot: Vec<u64>,
    publications: Vec<Publication<u32, u64>>,
    invalidations: Vec<RecoveryReason>,
}

impl MarketDataHandler<u32, u64, u64> for ReplayHandler {
    type Error = ();

    fn install_snapshot(&mut self, _watermark: u64, entries: &[u64]) -> Result<(), Self::Error> {
        self.valid = true;
        self.snapshot = entries.to_vec();
        self.publications.clear();

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

type TestClient = Client<u32, u64, u64, ReplayHandler>;
type TestMessage = InboundMessage<u32, u64, u64>;

#[derive(Debug, PartialEq, Eq)]
struct ReplayResult {
    state: ClientState,
    publisher_watermark: Option<u64>,
    transport_sequence: u64,
    stats: ClientStats,
    publications: Vec<Publication<u32, u64>>,
    snapshot: Vec<u64>,
}

fn client() -> TestClient {
    Client::new(
        ClientConfig {
            heartbeat_interval_ns: 100,
            server_timeout_ns: 1000,
            max_snapshot_entries: 64,
        },
        ReplayHandler::default(),
    )
    .unwrap()
}

fn send(client: &mut TestClient, message: TestMessage, now_ns: u64) -> ReceiveOutcome {
    let sequence = client.next_transport_sequence().unwrap();

    client
        .receive(InboundFrame::new(sequence, message), now_ns)
        .unwrap()
}

fn snapshot(client: &mut TestClient, watermark: u64, entries: &[u64], now_ns: u64) {
    send(
        client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: watermark,
            entry_count: entries.len(),
        },
        now_ns,
    );

    for entry in entries {
        send(client, InboundMessage::SnapshotEntry(*entry), now_ns);
    }

    assert_eq!(
        send(
            client,
            InboundMessage::SnapshotEnd {
                publisher_watermark: watermark,
            },
            now_ns,
        ),
        ReceiveOutcome::SnapshotInstalled
    );
}

fn next_random(state: &mut u64) -> u64 {
    let mut value = *state;

    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;

    *state = value;

    value
}

fn replay(seed: u64) -> ReplayResult {
    let mut client = client();

    client.connect(0).unwrap();

    snapshot(&mut client, 100, &[10, 20, 30], 0);

    let mut random_state = seed;
    let mut publisher_sequence = 100u64;

    for index in 1u64..=10_000 {
        let random = next_random(&mut random_state);

        // Simulate filtered global publication sequences.
        publisher_sequence += 1 + random % 4;

        let instrument = (random % 32) as u32;

        let kind = match random % 3 {
            0 => EventKind::Trade,
            1 => EventKind::Quote,
            _ => EventKind::BookUpdate,
        };

        let publication = Publication::new(
            publisher_sequence,
            Some(instrument),
            kind,
            index * 1000,
            random,
        );

        assert_eq!(
            send(
                &mut client,
                InboundMessage::Gateway(ServerMessage::Incremental(publication),),
                index,
            ),
            ReceiveOutcome::IncrementalApplied
        );

        if index % 100 == 0 {
            send(
                &mut client,
                InboundMessage::Gateway(ServerMessage::Heartbeat {
                    timestamp_ns: index,
                    publisher_watermark: publisher_sequence,
                }),
                index,
            );
        }
    }

    ReplayResult {
        state: client.state(),
        publisher_watermark: client.publisher_watermark(),
        transport_sequence: client.last_transport_sequence(),
        stats: *client.stats(),
        publications: client.handler().publications.clone(),
        snapshot: client.handler().snapshot.clone(),
    }
}

#[test]
fn identical_inputs_produce_identical_client_state() {
    let first = replay(0x1234_5678_9ABC_DEF0);
    let second = replay(0x1234_5678_9ABC_DEF0);

    assert_eq!(first, second);
}

#[test]
fn different_inputs_produce_different_client_state() {
    let first = replay(12345);
    let second = replay(67890);

    assert_ne!(first.publications, second.publications);
}

#[test]
fn replay_preserves_publication_order() {
    let result = replay(12345);

    assert_eq!(result.publications.len(), 10_000);

    for pair in result.publications.windows(2) {
        assert!(pair[0].sequence < pair[1].sequence);
    }

    assert_eq!(
        result.publisher_watermark,
        result.publications.last().map(|event| event.sequence)
    );
}

#[test]
fn replay_statistics_reconcile() {
    let result = replay(12345);

    assert_eq!(result.state, ClientState::Active);

    assert_eq!(result.stats.incrementals_applied, 10_000);
    assert_eq!(result.stats.heartbeats_received, 100);

    assert_eq!(result.stats.snapshots_started, 1);
    assert_eq!(result.stats.snapshots_completed, 1);
    assert_eq!(result.stats.snapshot_entries_received, 3);

    assert_eq!(result.stats.frames_received, 10_105);
    assert_eq!(result.stats.frames_accepted, 10_105);
    assert_eq!(result.transport_sequence, 10_105);

    assert_eq!(result.stats.frames_rejected, 0);
    assert_eq!(result.stats.transport_gaps, 0);
    assert_eq!(result.stats.duplicate_frames, 0);
    assert_eq!(result.stats.recovery_transitions, 0);
    assert_eq!(result.stats.handler_failures, 0);
}

#[test]
fn repeated_snapshot_recovery_respects_capacity() {
    let mut client = client();

    client.connect(0).unwrap();

    let entries = [42u64; 64];

    for watermark in 1u64..=1000 {
        snapshot(&mut client, watermark, &entries, watermark);

        assert!(client.is_active());
        assert_eq!(client.snapshot_entry_count(), 0);
        assert_eq!(client.handler().snapshot.len(), 64);

        client.require_recovery().unwrap();
    }

    assert_eq!(client.stats().snapshots_completed, 1000);
    assert_eq!(client.stats().snapshot_entries_received, 64_000);
    assert_eq!(client.stats().snapshot_high_watermark, 64);
    assert_eq!(client.stats().snapshots_aborted, 0);
    assert_eq!(client.stats().recovery_transitions, 1000);
}

#[test]
fn repeated_connection_churn_resets_state() {
    let mut client = client();

    for index in 1u64..=10_000 {
        client.connect(index).unwrap();

        snapshot(&mut client, index, &[], index);

        assert!(client.is_active());
        assert_eq!(client.last_transport_sequence(), 2);

        client.disconnect();

        assert_eq!(client.state(), ClientState::Disconnected);
        assert_eq!(client.publisher_watermark(), None);
        assert_eq!(client.last_transport_sequence(), 0);
        assert!(!client.handler().valid);
    }

    assert_eq!(client.stats().connections_established, 10_000);
    assert_eq!(client.stats().connections_closed, 10_000);
    assert_eq!(client.stats().snapshots_completed, 10_000);
}
