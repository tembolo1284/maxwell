//! Consistent downstream market snapshot coordination.
//!
//! A downstream snapshot represents reconstructed market
//! state at one global publication watermark.
//!
//! Individual instrument watermarks retain their original
//! upstream sequence semantics.
//!
//! Snapshot capture, transport acceptance, and subscriber
//! activation must not interleave with market mutations.

use maxwell_book::{BookState, RestingOrder};
use maxwell_core::{FeedId, Instrument, InstrumentId, SequenceNumber};
use maxwell_gateway::{SessionId, SessionState};
use maxwell_publisher::{EventKind, SubscriptionFilter};

use crate::config::SnapshotConfig;
use crate::downstream::SnapshotSink;
use crate::error::{ServerError, ServerResult};
use crate::pipeline::Pipeline;

// ============================================================
// Snapshot records
// ============================================================

/// One record in a complete downstream market snapshot.
///
/// Instrument records precede their associated resting
/// order records.
///
/// Instrument metadata is retained even when the
/// corresponding book contains no resting orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotEntry {
    Instrument {
        instrument: Instrument,
        feed: FeedId,
        last_sequence: Option<SequenceNumber>,
    },

    Order {
        instrument_id: InstrumentId,
        order: RestingOrder,
    },
}

impl SnapshotEntry {
    #[must_use]
    pub const fn instrument_id(&self) -> InstrumentId {
        match self {
            Self::Instrument { instrument, .. } => instrument.id(),

            Self::Order { instrument_id, .. } => *instrument_id,
        }
    }
}

// ============================================================
// Market snapshot
// ============================================================

/// Complete downstream snapshot at one publication watermark.
///
/// The snapshot owns its records and is independent of
/// subsequent changes to the reconstructed books.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketSnapshot {
    publisher_watermark: u64,
    entries: Vec<SnapshotEntry>,
}

impl MarketSnapshot {
    /// Constructs an empty snapshot.
    ///
    /// This does not independently establish that the
    /// upstream market is synchronized.
    #[must_use]
    pub const fn empty(publisher_watermark: u64) -> Self {
        Self {
            publisher_watermark,
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub const fn publisher_watermark(&self) -> u64 {
        self.publisher_watermark
    }

    #[must_use]
    pub fn entries(&self) -> &[SnapshotEntry] {
        &self.entries
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ============================================================
// Snapshot capture
// ============================================================

/// Determines whether a subscription requires book state.
///
/// Trade-only subscriptions do not require resting order
/// snapshots.
fn includes_book(filter: &SubscriptionFilter<InstrumentId>, instrument_id: InstrumentId) -> bool {
    filter.matches(Some(&instrument_id), EventKind::BookUpdate)
}

/// Captures a consistent snapshot of subscribed books.
///
/// Snapshot capacity includes both instrument metadata
/// and individual resting order records.
///
/// The caller must additionally verify that upstream
/// recovery is synchronized.
pub fn capture_snapshot(
    pipeline: &Pipeline,
    filter: &SubscriptionFilter<InstrumentId>,
    config: SnapshotConfig,
) -> ServerResult<MarketSnapshot> {
    config.validate()?;

    if !pipeline.is_valid() || pipeline.has_pending_publication() {
        return Err(ServerError::SnapshotUnavailable);
    }

    let watermark = pipeline.publication_watermark();

    // --------------------------------------------------------
    // Determine required capacity before allocating.
    // --------------------------------------------------------

    let mut required = 0_usize;

    for book in pipeline.books().iter() {
        if !includes_book(filter, book.instrument_id()) {
            continue;
        }

        if book.state() != BookState::Live {
            return Err(ServerError::SnapshotUnavailable);
        }

        required = required
            .checked_add(1)
            .and_then(|count| count.checked_add(book.order_count()))
            .ok_or(ServerError::SnapshotCapacityExceeded {
                limit: config.max_entries,
                required: usize::MAX,
            })?;
    }

    if required > config.max_entries {
        return Err(ServerError::SnapshotCapacityExceeded {
            limit: config.max_entries,
            required,
        });
    }

    // --------------------------------------------------------
    // Collect instrument identifiers in deterministic order.
    // --------------------------------------------------------

    let mut instruments = Vec::new();

    instruments
        .try_reserve_exact(pipeline.books().instrument_count())
        .map_err(|_| ServerError::AllocationFailed)?;

    for book in pipeline.books().iter() {
        let id = book.instrument_id();

        if includes_book(filter, id) {
            instruments.push(id);
        }
    }

    instruments.sort_unstable();

    // --------------------------------------------------------
    // Allocate bounded snapshot storage.
    // --------------------------------------------------------

    let mut entries = Vec::new();

    entries
        .try_reserve_exact(required)
        .map_err(|_| ServerError::AllocationFailed)?;

    // --------------------------------------------------------
    // Capture each instrument in canonical order.
    // --------------------------------------------------------

    for instrument_id in instruments {
        let snapshot = pipeline.books().capture_snapshot(instrument_id)?;

        entries.push(SnapshotEntry::Instrument {
            instrument: snapshot.instrument,
            feed: snapshot.feed,
            last_sequence: snapshot.last_sequence,
        });

        // BookSnapshot already provides deterministic
        // ordering of resting orders by order identifier.
        for order in snapshot.orders {
            entries.push(SnapshotEntry::Order {
                instrument_id,
                order,
            });
        }
    }

    if entries.len() != required {
        return Err(ServerError::InvariantViolation(
            "snapshot entry count changed during capture",
        ));
    }

    if pipeline.publication_watermark() != watermark {
        return Err(ServerError::InvariantViolation(
            "publication watermark changed during capture",
        ));
    }

    Ok(MarketSnapshot {
        publisher_watermark: watermark,
        entries,
    })
}

// ============================================================
// Snapshot delivery outcomes
// ============================================================

/// Result of a downstream synchronization attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotOutcome<E> {
    /// Complete snapshot accepted and subscriber activated.
    Activated {
        publisher_watermark: u64,
        entry_count: usize,
    },

    /// Destination rejected the entire snapshot.
    ///
    /// The subscriber remains inactive.
    Rejected(E),
}

// ============================================================
// Snapshot coordination
// ============================================================

/// Captures, delivers, and activates a downstream snapshot.
///
/// This operation must execute without interleaving
/// upstream mutations or publication attempts.
///
/// The supplied filter must be the same filter registered
/// with the gateway for this session.
///
/// The caller must terminate the downstream connection
/// if activation fails after transport acceptance.
pub(crate) fn synchronize_session<S>(
    pipeline: &mut Pipeline,
    session_id: SessionId,
    filter: &SubscriptionFilter<InstrumentId>,
    config: SnapshotConfig,
    sink: &mut S,
    now_ns: u64,
) -> ServerResult<SnapshotOutcome<S::Error>>
where
    S: SnapshotSink,
{
    // --------------------------------------------------------
    // Validate session eligibility.
    // --------------------------------------------------------

    let session = pipeline
        .gateway()
        .session(session_id)
        .ok_or(ServerError::InvalidState("snapshot session does not exist"))?;

    if !matches!(
        session.state(),
        SessionState::AwaitingSnapshot | SessionState::ResyncRequired
    ) {
        return Err(ServerError::InvalidState(
            "session is not awaiting synchronization",
        ));
    }

    if now_ns < pipeline.gateway().now_ns() {
        return Err(ServerError::InvalidState("snapshot timestamp regressed"));
    }

    if session.timeout_due(now_ns, pipeline.gateway().config().session_timeout_ns) {
        return Err(ServerError::InvalidState("snapshot session has expired"));
    }

    // --------------------------------------------------------
    // Capture at the current publication watermark.
    // --------------------------------------------------------

    let snapshot = capture_snapshot(pipeline, filter, config)?;

    let watermark = snapshot.publisher_watermark();
    let entry_count = snapshot.len();

    // --------------------------------------------------------
    // Transfer ownership of the complete transaction.
    // --------------------------------------------------------

    if let Err(error) = sink.try_send_snapshot(session_id, &snapshot) {
        return Ok(SnapshotOutcome::Rejected(error));
    }

    // --------------------------------------------------------
    // Activate only after successful acceptance.
    // --------------------------------------------------------

    if let Err(error) = pipeline
        .gateway_mut()
        .activate_snapshot(session_id, watermark, now_ns)
    {
        // The snapshot has already been accepted by the
        // transport. The connection must not continue as
        // though activation succeeded.
        //
        // Remove the gateway session. The application
        // must also close its associated transport.
        pipeline.gateway_mut().disconnect(session_id)?;

        return Err(ServerError::Gateway(error));
    }

    Ok(SnapshotOutcome::Activated {
        publisher_watermark: watermark,
        entry_count,
    })
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_gateway::ClientRequest;

    use crate::config::ServerConfig;
    use crate::downstream::{ConnectionQueue, DeliveryError, DownstreamMessage};

    fn pipeline() -> Pipeline {
        Pipeline::new(&ServerConfig::default()).unwrap()
    }

    fn filter() -> SubscriptionFilter<InstrumentId> {
        SubscriptionFilter::all()
    }

    fn session_id() -> SessionId {
        SessionId::new(1)
    }

    fn subscribed_pipeline() -> Pipeline {
        let mut pipeline = pipeline();

        pipeline.mark_valid().unwrap();

        pipeline.gateway_mut().connect(session_id(), 0).unwrap();

        pipeline
            .gateway_mut()
            .handle_request(
                session_id(),
                ClientRequest::Subscribe { filter: filter() },
                0,
            )
            .unwrap();

        pipeline
    }

    // --------------------------------------------------------
    // Snapshot representation
    // --------------------------------------------------------

    #[test]
    fn empty_snapshot_preserves_watermark() {
        let snapshot = MarketSnapshot::empty(42);

        assert_eq!(snapshot.publisher_watermark(), 42);
        assert_eq!(snapshot.len(), 0);
        assert!(snapshot.is_empty());
    }

    #[test]
    fn invalid_baseline_prevents_snapshot() {
        let pipeline = pipeline();

        assert!(matches!(
            capture_snapshot(&pipeline, &filter(), SnapshotConfig::default(),),
            Err(ServerError::SnapshotUnavailable),
        ));
    }

    #[test]
    fn valid_empty_market_can_be_captured() {
        let mut pipeline = pipeline();

        pipeline.mark_valid().unwrap();

        let snapshot = capture_snapshot(&pipeline, &filter(), SnapshotConfig::default()).unwrap();

        assert!(snapshot.is_empty());

        assert_eq!(
            snapshot.publisher_watermark(),
            pipeline.publication_watermark(),
        );
    }

    #[test]
    fn zero_snapshot_capacity_is_rejected() {
        let mut pipeline = pipeline();

        pipeline.mark_valid().unwrap();

        assert!(
            capture_snapshot(&pipeline, &filter(), SnapshotConfig { max_entries: 0 },).is_err()
        );
    }

    // --------------------------------------------------------
    // Session synchronization
    // --------------------------------------------------------

    #[test]
    fn accepted_snapshot_activates_session() {
        let mut pipeline = subscribed_pipeline();

        let mut queue = ConnectionQueue::new(session_id(), 4).unwrap();

        let watermark = pipeline.publication_watermark();

        let outcome = synchronize_session(
            &mut pipeline,
            session_id(),
            &filter(),
            SnapshotConfig::default(),
            &mut queue,
            1,
        )
        .unwrap();

        assert_eq!(
            outcome,
            SnapshotOutcome::Activated {
                publisher_watermark: watermark,
                entry_count: 0,
            },
        );

        assert!(
            pipeline
                .gateway()
                .session(session_id())
                .unwrap()
                .is_active()
        );

        assert_eq!(queue.len(), 2);

        assert!(matches!(
            queue.pop_front().unwrap().message,
            DownstreamMessage::SnapshotBegin { .. },
        ));

        assert!(matches!(
            queue.pop_front().unwrap().message,
            DownstreamMessage::SnapshotEnd { .. },
        ));
    }

    #[test]
    fn rejected_snapshot_does_not_activate_session() {
        let mut pipeline = subscribed_pipeline();

        let mut queue = ConnectionQueue::new(session_id(), 1).unwrap();

        let outcome = synchronize_session(
            &mut pipeline,
            session_id(),
            &filter(),
            SnapshotConfig::default(),
            &mut queue,
            1,
        )
        .unwrap();

        assert_eq!(
            outcome,
            SnapshotOutcome::Rejected(DeliveryError::Backpressure,),
        );

        assert_eq!(
            pipeline.gateway().session(session_id()).unwrap().state(),
            SessionState::AwaitingSnapshot,
        );

        assert!(queue.is_empty());
        assert_eq!(queue.next_sequence(), Some(1));
    }

    #[test]
    fn unknown_session_cannot_be_synchronized() {
        let mut pipeline = pipeline();

        pipeline.mark_valid().unwrap();

        let mut queue = ConnectionQueue::new(session_id(), 4).unwrap();

        assert!(
            synchronize_session(
                &mut pipeline,
                session_id(),
                &filter(),
                SnapshotConfig::default(),
                &mut queue,
                1,
            )
            .is_err()
        );

        assert!(queue.is_empty());
    }

    #[test]
    fn invalidated_market_cannot_activate_session() {
        let mut pipeline = subscribed_pipeline();

        pipeline.invalidate();

        let mut queue = ConnectionQueue::new(session_id(), 4).unwrap();

        assert!(
            synchronize_session(
                &mut pipeline,
                session_id(),
                &filter(),
                SnapshotConfig::default(),
                &mut queue,
                1,
            )
            .is_err()
        );

        assert!(queue.is_empty());
    }
}
