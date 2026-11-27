//! Integration tests for snapshot synchronization.

use maxwell_client::{
    Client, ClientConfig, ClientError, ClientState, InboundFrame, InboundMessage,
    MarketDataHandler, ReceiveOutcome, RecoveryReason,
};

use maxwell_gateway::ServerMessage;
use maxwell_publisher::{EventKind, Publication};

type TestClient = Client<u32, u64, u64, RecordingHandler>;
type TestMessage = InboundMessage<u32, u64, u64>;

#[derive(Debug, Default)]
struct RecordingHandler {
    valid: bool,
    snapshot: Vec<u64>,
    watermark: Option<u64>,
    incrementals: Vec<Publication<u32, u64>>,
    invalidations: Vec<RecoveryReason>,
    fail_snapshot: bool,
    fail_incremental: bool,
}

impl MarketDataHandler<u32, u64, u64> for RecordingHandler {
    type Error = ();

    fn install_snapshot(&mut self, watermark: u64, entries: &[u64]) -> Result<(), Self::Error> {
        if self.fail_snapshot {
            return Err(());
        }

        self.snapshot = entries.to_vec();
        self.incrementals.clear();
        self.watermark = Some(watermark);
        self.valid = true;

        Ok(())
    }

    fn apply_incremental(
        &mut self,
        publication: &Publication<u32, u64>,
    ) -> Result<(), Self::Error> {
        if self.fail_incremental || !self.valid {
            return Err(());
        }

        self.incrementals.push(*publication);
        self.watermark = Some(publication.sequence);

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

fn client(handler: RecordingHandler) -> TestClient {
    let mut client = Client::new(
        ClientConfig {
            heartbeat_interval_ns: 50,
            server_timeout_ns: 150,
            max_snapshot_entries: 8,
        },
        handler,
    )
    .unwrap();

    client.connect(0).unwrap();

    client
}

fn receive(client: &mut TestClient, message: TestMessage) -> Result<ReceiveOutcome, ClientError> {
    let sequence = client.next_transport_sequence().unwrap();

    client.receive(InboundFrame::new(sequence, message), 0)
}

fn install_snapshot(client: &mut TestClient, watermark: u64, entries: &[u64]) {
    receive(
        client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: watermark,
            entry_count: entries.len(),
        },
    )
    .unwrap();

    for entry in entries {
        receive(client, InboundMessage::SnapshotEntry(*entry)).unwrap();
    }

    receive(
        client,
        InboundMessage::SnapshotEnd {
            publisher_watermark: watermark,
        },
    )
    .unwrap();
}

fn incremental(sequence: u64) -> TestMessage {
    InboundMessage::Gateway(ServerMessage::Incremental(Publication::new(
        sequence,
        Some(100),
        EventKind::BookUpdate,
        sequence * 1000,
        sequence,
    )))
}

#[test]
fn installs_complete_snapshot() {
    let mut client = client(RecordingHandler::default());

    install_snapshot(&mut client, 100, &[10, 20, 30]);

    assert_eq!(client.state(), ClientState::Active);
    assert_eq!(client.publisher_watermark(), Some(100));
    assert_eq!(client.handler().snapshot, vec![10, 20, 30]);
    assert!(client.handler().valid);
    assert_eq!(client.snapshot_entry_count(), 0);
    assert_eq!(client.stats().snapshots_completed, 1);
}

#[test]
fn supports_empty_snapshot() {
    let mut client = client(RecordingHandler::default());

    install_snapshot(&mut client, 0, &[]);

    assert!(client.is_active());
    assert!(client.handler().snapshot.is_empty());
    assert_eq!(client.publisher_watermark(), Some(0));
}

#[test]
fn snapshot_is_not_visible_before_completion() {
    let mut client = client(RecordingHandler::default());

    receive(
        &mut client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: 100,
            entry_count: 2,
        },
    )
    .unwrap();

    receive(&mut client, InboundMessage::SnapshotEntry(10)).unwrap();

    assert_eq!(client.state(), ClientState::ReceivingSnapshot);

    assert!(!client.handler().valid);
    assert!(client.handler().snapshot.is_empty());
    assert_eq!(client.publisher_watermark(), None);
}

#[test]
fn rejects_oversized_snapshot() {
    let mut client = client(RecordingHandler::default());

    let result = receive(
        &mut client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: 100,
            entry_count: 9,
        },
    );

    assert_eq!(
        result,
        Err(ClientError::SnapshotCapacityExceeded {
            limit: 8,
            received: 9,
        })
    );

    assert_eq!(client.state(), ClientState::RecoveryRequired);

    assert_eq!(client.snapshot_entry_count(), 0);
}

#[test]
fn rejects_incomplete_snapshot() {
    let mut client = client(RecordingHandler::default());

    receive(
        &mut client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: 100,
            entry_count: 2,
        },
    )
    .unwrap();

    receive(&mut client, InboundMessage::SnapshotEntry(10)).unwrap();

    let result = receive(
        &mut client,
        InboundMessage::SnapshotEnd {
            publisher_watermark: 100,
        },
    );

    assert_eq!(
        result,
        Err(ClientError::SnapshotCountMismatch {
            expected: 2,
            received: 1,
        })
    );

    assert!(!client.handler().valid);
    assert_eq!(client.stats().snapshots_aborted, 1);
}

#[test]
fn rejects_excess_snapshot_entries() {
    let mut client = client(RecordingHandler::default());

    receive(
        &mut client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: 100,
            entry_count: 1,
        },
    )
    .unwrap();

    receive(&mut client, InboundMessage::SnapshotEntry(10)).unwrap();

    assert_eq!(
        receive(&mut client, InboundMessage::SnapshotEntry(20),),
        Err(ClientError::SnapshotCountMismatch {
            expected: 1,
            received: 2,
        })
    );

    assert_eq!(client.snapshot_entry_count(), 0);
}

#[test]
fn rejects_mismatched_snapshot_watermark() {
    let mut client = client(RecordingHandler::default());

    receive(
        &mut client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: 100,
            entry_count: 0,
        },
    )
    .unwrap();

    assert_eq!(
        receive(
            &mut client,
            InboundMessage::SnapshotEnd {
                publisher_watermark: 101,
            },
        ),
        Err(ClientError::SnapshotWatermarkMismatch {
            expected: 100,
            received: 101,
        })
    );
}

#[test]
fn applies_incrementals_after_snapshot() {
    let mut client = client(RecordingHandler::default());

    install_snapshot(&mut client, 100, &[10, 20]);

    assert_eq!(
        receive(&mut client, incremental(101)),
        Ok(ReceiveOutcome::IncrementalApplied)
    );

    assert_eq!(client.publisher_watermark(), Some(101));
    assert_eq!(client.handler().incrementals.len(), 1);
}

#[test]
fn accepts_filtered_publication_sequence_gaps() {
    let mut client = client(RecordingHandler::default());

    install_snapshot(&mut client, 100, &[]);

    for sequence in [105, 110, 150, 200] {
        assert_eq!(
            receive(&mut client, incremental(sequence)),
            Ok(ReceiveOutcome::IncrementalApplied)
        );
    }

    assert!(client.is_active());
    assert_eq!(client.publisher_watermark(), Some(200));
    assert_eq!(client.stats().transport_gaps, 0);
    assert_eq!(client.stats().incrementals_applied, 4);
}

#[test]
fn ignores_incrementals_before_initial_snapshot() {
    let mut client = client(RecordingHandler::default());

    assert_eq!(
        receive(&mut client, incremental(100)),
        Ok(ReceiveOutcome::IncrementalIgnored)
    );

    assert_eq!(client.stats().incrementals_ignored, 1);
    assert!(client.handler().incrementals.is_empty());
}

#[test]
fn incremental_during_snapshot_invalidates_transaction() {
    let mut client = client(RecordingHandler::default());

    receive(
        &mut client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: 100,
            entry_count: 2,
        },
    )
    .unwrap();

    assert_eq!(
        receive(&mut client, incremental(101)),
        Ok(ReceiveOutcome::RecoveryRequired(
            RecoveryReason::UnexpectedMessage,
        ))
    );

    assert_eq!(client.stats().snapshots_aborted, 1);
    assert!(!client.handler().valid);
}

#[test]
fn snapshot_handler_failure_requires_recovery() {
    let mut client = client(RecordingHandler {
        fail_snapshot: true,
        ..RecordingHandler::default()
    });

    receive(
        &mut client,
        InboundMessage::SnapshotBegin {
            publisher_watermark: 100,
            entry_count: 0,
        },
    )
    .unwrap();

    assert_eq!(
        receive(
            &mut client,
            InboundMessage::SnapshotEnd {
                publisher_watermark: 100,
            },
        ),
        Err(ClientError::HandlerFailure)
    );

    assert_eq!(
        client.recovery_reason(),
        Some(RecoveryReason::HandlerFailure)
    );

    assert_eq!(client.stats().handler_failures, 1);
    assert_eq!(client.stats().snapshots_completed, 0);
}

#[test]
fn incremental_handler_failure_invalidates_baseline() {
    let mut client = client(RecordingHandler {
        fail_incremental: true,
        ..RecordingHandler::default()
    });

    install_snapshot(&mut client, 100, &[]);

    assert_eq!(
        receive(&mut client, incremental(101)),
        Err(ClientError::HandlerFailure)
    );

    assert!(!client.handler().valid);
    assert_eq!(client.publisher_watermark(), None);
    assert_eq!(client.stats().incrementals_applied, 0);

    assert_eq!(
        receive(&mut client, incremental(102)),
        Ok(ReceiveOutcome::IncrementalIgnored)
    );
}

#[test]
fn rejects_snapshot_older_than_previous_baseline() {
    let mut client = client(RecordingHandler::default());

    install_snapshot(&mut client, 100, &[]);
    receive(&mut client, incremental(110)).unwrap();

    client.require_recovery().unwrap();

    assert_eq!(
        receive(
            &mut client,
            InboundMessage::SnapshotBegin {
                publisher_watermark: 109,
                entry_count: 0,
            },
        ),
        Err(ClientError::SnapshotWatermarkRegression {
            minimum: 110,
            received: 109,
        })
    );
}

#[test]
fn recovery_snapshot_replaces_previous_state() {
    let mut client = client(RecordingHandler::default());

    install_snapshot(&mut client, 100, &[10, 20]);

    receive(&mut client, incremental(101)).unwrap();

    client.require_recovery().unwrap();

    install_snapshot(&mut client, 200, &[30, 40]);

    assert!(client.is_active());
    assert_eq!(client.handler().snapshot, vec![30, 40]);
    assert!(client.handler().incrementals.is_empty());
    assert_eq!(client.publisher_watermark(), Some(200));
    assert_eq!(client.stats().snapshots_completed, 2);
}
