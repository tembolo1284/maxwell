//! Upstream sequencing and recovery orchestration.
//!
//! RecoveryManager owns the authoritative upstream sequence
//! cursor for the initial single-feed server.
//!
//! Live events are staged for publication after successful
//! book application. Recovery events reconstruct the book
//! without publishing historical incrementals.
//!
//! Downstream subscribers must resynchronize after a gap.

use maxwell_core::{EventEnvelope, FeedId, MonotonicTimestamp, SequenceNumber};
use maxwell_recovery::{
    IngestOutcome, RecoveryError, RecoveryManager, RecoveryRequest, RecoveryState, RequestId,
};

use crate::config::ServerConfig;
use crate::error::{ServerError, ServerResult};
use crate::pipeline::{Pipeline, PublicationOutcome};

// ============================================================
// Processing outcomes
// ============================================================

/// Result of admitting an upstream event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// An event was applied and its publication accepted.
    Published { sequence: u64 },

    /// An event was applied, but publication was rejected.
    ///
    /// The event has been consumed. Only its publication
    /// should be retried.
    PublicationPending,

    /// The supplied event was not consumed because a
    /// previous publication remains pending.
    ///
    /// The caller retains ownership of the supplied event.
    InputBlocked,

    /// An out-of-order event was buffered.
    Buffered,

    /// An already-consumed event was received.
    Duplicate,

    /// Recovery applied one or more events without
    /// publishing historical incrementals.
    Replayed,

    /// The reconstructed state is synchronized again.
    ///
    /// Invalidated downstream subscribers still require
    /// a new snapshot.
    Resynchronized,

    /// Recovery requires additional authoritative data.
    RecoveryRequired,
}

// ============================================================
// Recovery coordinator
// ============================================================

/// Coordinates the upstream recovery manager and market
/// data processing pipeline.
///
/// This component does not own network sockets or perform
/// recovery request serialization.
pub struct RecoveryCoordinator {
    manager: RecoveryManager,
}

impl RecoveryCoordinator {
    /// Constructs an uninitialized recovery coordinator.
    pub fn new(feed: FeedId, config: &ServerConfig) -> ServerResult<Self> {
        Ok(Self {
            manager: RecoveryManager::new(feed, config.recovery)?,
        })
    }

    #[must_use]
    pub const fn manager(&self) -> &RecoveryManager {
        &self.manager
    }

    #[must_use]
    pub const fn feed(&self) -> FeedId {
        self.manager.feed()
    }

    #[must_use]
    pub const fn state(&self) -> RecoveryState {
        self.manager.state()
    }

    #[must_use]
    pub const fn next_expected(&self) -> SequenceNumber {
        self.manager.next_expected()
    }

    #[must_use]
    pub const fn is_synchronized(&self) -> bool {
        self.manager.is_publishable()
    }

    #[must_use]
    pub const fn pending_request(&self) -> Option<RecoveryRequest> {
        self.manager.pending_request()
    }

    // --------------------------------------------------------
    // Initialization
    // --------------------------------------------------------

    /// Establishes an authoritative initial book baseline.
    ///
    /// The caller must have already registered instruments
    /// and installed the appropriate initial market state.
    ///
    /// An arbitrary empty book must not be treated as a
    /// valid baseline for an already-running upstream feed.
    pub(crate) fn initialize_live(
        &mut self,
        next_expected: SequenceNumber,
        pipeline: &mut Pipeline,
    ) -> ServerResult<()> {
        if pipeline.has_pending_publication() {
            return Err(ServerError::InvariantViolation(
                "initialization with pending publication",
            ));
        }

        self.manager.initialize_live(next_expected)?;

        pipeline.mark_valid()?;

        Ok(())
    }

    /// Requests authoritative snapshot recovery.
    ///
    /// The upstream adapter must determine whether the
    /// source supports the resulting recovery request.
    pub(crate) fn start_snapshot(
        &mut self,
        now: MonotonicTimestamp,
        pipeline: &mut Pipeline,
    ) -> ServerResult<()> {
        pipeline.invalidate();

        self.manager.start_snapshot(now)?;

        Ok(())
    }

    // --------------------------------------------------------
    // Live ingestion
    // --------------------------------------------------------

    /// Admits a canonical upstream event.
    ///
    /// The recovery manager validates feed identity and
    /// sequence continuity.
    ///
    /// A pending publication blocks new input before the
    /// recovery manager can consume the supplied event.
    pub fn ingest(
        &mut self,
        event: EventEnvelope,
        now: MonotonicTimestamp,
        pipeline: &mut Pipeline,
    ) -> ServerResult<RecoveryOutcome> {
        if pipeline.has_pending_publication() {
            return Ok(RecoveryOutcome::InputBlocked);
        }

        if event.header.feed() != self.manager.feed() {
            return Err(ServerError::Recovery(RecoveryError::FeedMismatch {
                expected: self.manager.feed(),
                received: event.header.feed(),
            }));
        }

        let was_live = self.manager.is_publishable();

        if was_live && !pipeline.is_valid() {
            return Err(ServerError::InvariantViolation(
                "live recovery manager has invalid book baseline",
            ));
        }

        // A future sequence establishes a gap. Invalidate
        // downstream state before admitting the event.
        if was_live && event.sequence() > self.manager.next_expected() {
            pipeline.invalidate();
        }

        let publish_live =
            was_live && pipeline.is_valid() && event.sequence() == self.manager.next_expected();

        let mut application_error = None;

        let result = self.manager.ingest(event, now, &mut |received| {
            let applied = if publish_live {
                pipeline.stage_live(*received, now.raw())
            } else {
                pipeline.apply_recovery(*received)
            };

            match applied {
                Ok(()) => Ok(()),

                Err(error) => {
                    application_error = Some(error);

                    Err(RecoveryError::EventApplicationFailed)
                }
            }
        });

        self.complete_ingestion(result, application_error, publish_live, pipeline)
    }

    // --------------------------------------------------------
    // Retransmission ingestion
    // --------------------------------------------------------

    /// Admits an event correlated with an active
    /// retransmission request.
    ///
    /// Recovered events are applied without downstream
    /// incremental publication.
    pub fn ingest_retransmission(
        &mut self,
        request_id: RequestId,
        event: EventEnvelope,
        now: MonotonicTimestamp,
        pipeline: &mut Pipeline,
    ) -> ServerResult<RecoveryOutcome> {
        if pipeline.has_pending_publication() {
            return Ok(RecoveryOutcome::InputBlocked);
        }

        if self.manager.is_publishable() || pipeline.is_valid() {
            return Err(ServerError::InvalidState(
                "retransmission requires an active recovery",
            ));
        }

        let mut application_error = None;

        let result = self
            .manager
            .ingest_retransmission(request_id, event, now, &mut |received| match pipeline
                .apply_recovery(*received)
            {
                Ok(()) => Ok(()),

                Err(error) => {
                    application_error = Some(error);

                    Err(RecoveryError::EventApplicationFailed)
                }
            });

        self.complete_ingestion(result, application_error, false, pipeline)
    }

    // --------------------------------------------------------
    // Ingestion completion
    // --------------------------------------------------------

    /// Reconciles successful upstream application with
    /// downstream publication and synchronization.
    fn complete_ingestion(
        &mut self,
        result: Result<IngestOutcome, RecoveryError>,
        application_error: Option<ServerError>,
        publish_live: bool,
        pipeline: &mut Pipeline,
    ) -> ServerResult<RecoveryOutcome> {
        if let Some(error) = application_error {
            pipeline.invalidate();

            return Err(error);
        }

        let outcome = match result {
            Ok(outcome) => outcome,

            Err(error) => {
                if !self.manager.is_publishable()
                    || pipeline.has_pending_publication()
                    || !pipeline.is_valid()
                {
                    pipeline.invalidate();
                }

                return Err(ServerError::Recovery(error));
            }
        };

        // Recovery is not complete. Existing subscribers
        // must remain invalidated.
        if !self.manager.is_publishable() {
            pipeline.invalidate();

            return Ok(match outcome {
                IngestOutcome::Buffered => RecoveryOutcome::Buffered,

                IngestOutcome::Duplicate => RecoveryOutcome::Duplicate,

                IngestOutcome::Applied => RecoveryOutcome::Replayed,

                IngestOutcome::RecoveryRequired => RecoveryOutcome::RecoveryRequired,
            });
        }

        // The upstream cursor is synchronized again.
        // Historical recovery events were not published.
        if !pipeline.is_valid() {
            pipeline.mark_valid()?;

            return Ok(RecoveryOutcome::Resynchronized);
        }

        match outcome {
            IngestOutcome::Applied if publish_live => match pipeline.retry_publication()? {
                PublicationOutcome::Published { sequence } => {
                    Ok(RecoveryOutcome::Published { sequence })
                }

                PublicationOutcome::Blocked => Ok(RecoveryOutcome::PublicationPending),

                PublicationOutcome::Idle => Err(ServerError::InvariantViolation(
                    "applied live event has no publication",
                )),
            },

            IngestOutcome::Applied => Ok(RecoveryOutcome::Replayed),

            IngestOutcome::Duplicate => Ok(RecoveryOutcome::Duplicate),

            IngestOutcome::Buffered => Ok(RecoveryOutcome::Buffered),

            IngestOutcome::RecoveryRequired => Ok(RecoveryOutcome::RecoveryRequired),
        }
    }

    // --------------------------------------------------------
    // Publication retry
    // --------------------------------------------------------

    /// Retries an already-applied live publication.
    ///
    /// This operation does not advance upstream sequencing
    /// or mutate the reconstructed book.
    pub fn retry_publication(
        &mut self,
        pipeline: &mut Pipeline,
    ) -> ServerResult<PublicationOutcome> {
        if !pipeline.has_pending_publication() {
            return Ok(PublicationOutcome::Idle);
        }

        if !self.manager.is_publishable() {
            pipeline.invalidate();

            return Err(ServerError::InvalidState(
                "cannot retry publication during recovery",
            ));
        }

        pipeline.retry_publication()
    }

    // --------------------------------------------------------
    // Recovery requests
    // --------------------------------------------------------

    /// Acknowledges successful submission of a complete
    /// recovery request to the upstream transport.
    ///
    /// A partial write must not be acknowledged.
    pub fn acknowledge_request(&mut self, request_id: RequestId) -> ServerResult<()> {
        self.manager.acknowledge_request(request_id)?;

        Ok(())
    }

    /// Reports that an active retransmission request
    /// cannot be fulfilled by the upstream source.
    ///
    /// The recovery manager determines the appropriate
    /// escalation according to its configured policy.
    pub fn retransmission_unavailable(
        &mut self,
        request_id: RequestId,
        now: MonotonicTimestamp,
        pipeline: &mut Pipeline,
    ) -> ServerResult<()> {
        let result = self.manager.retransmission_unavailable(request_id, now);

        if !self.manager.is_publishable() {
            pipeline.invalidate();
        }

        result?;

        Ok(())
    }

    // --------------------------------------------------------
    // Watermark observation
    // --------------------------------------------------------

    /// Observes an authoritative upstream watermark.
    ///
    /// This can detect missing trailing events even when
    /// no subsequent incremental event has arrived.
    pub fn observe_watermark(
        &mut self,
        feed: FeedId,
        last_sequence: SequenceNumber,
        now: MonotonicTimestamp,
        pipeline: &mut Pipeline,
    ) -> ServerResult<()> {
        let result = self.manager.observe_watermark(feed, last_sequence, now);

        if !self.manager.is_publishable() {
            pipeline.invalidate();
        }

        result?;

        Ok(())
    }

    // --------------------------------------------------------
    // Timers
    // --------------------------------------------------------

    /// Services recovery deadlines and retry policies.
    ///
    /// The supplied timestamp must use the same monotonic
    /// clock as upstream ingestion.
    pub fn poll(&mut self, now: MonotonicTimestamp, pipeline: &mut Pipeline) -> ServerResult<()> {
        let result = self.manager.poll(now);

        if !self.manager.is_publishable() {
            pipeline.invalidate();
        }

        result?;

        Ok(())
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{SessionId, StreamId};

    fn feed() -> FeedId {
        FeedId::new(StreamId::new(1), SessionId::new(1))
    }

    fn components() -> (RecoveryCoordinator, Pipeline) {
        let config = ServerConfig::default();

        let coordinator = RecoveryCoordinator::new(feed(), &config).unwrap();

        let pipeline = Pipeline::new(&config).unwrap();

        (coordinator, pipeline)
    }

    #[test]
    fn new_coordinator_is_uninitialized() {
        let (coordinator, pipeline) = components();

        assert_eq!(coordinator.state(), RecoveryState::Uninitialized,);

        assert!(!coordinator.is_synchronized());
        assert!(!pipeline.is_valid());
    }

    #[test]
    fn initializes_authoritative_baseline() {
        let (mut coordinator, mut pipeline) = components();

        coordinator
            .initialize_live(SequenceNumber::ONE, &mut pipeline)
            .unwrap();

        assert_eq!(coordinator.state(), RecoveryState::Live,);

        assert_eq!(coordinator.next_expected(), SequenceNumber::ONE,);

        assert!(coordinator.is_synchronized());
        assert!(pipeline.is_valid());
    }

    #[test]
    fn rejects_repeated_initialization() {
        let (mut coordinator, mut pipeline) = components();

        coordinator
            .initialize_live(SequenceNumber::ONE, &mut pipeline)
            .unwrap();

        assert!(
            coordinator
                .initialize_live(SequenceNumber::ONE, &mut pipeline,)
                .is_err()
        );
    }

    #[test]
    fn no_pending_publication_is_idle() {
        let (mut coordinator, mut pipeline) = components();

        assert_eq!(
            coordinator.retry_publication(&mut pipeline).unwrap(),
            PublicationOutcome::Idle,
        );
    }

    #[test]
    fn snapshot_recovery_invalidates_baseline() {
        let (mut coordinator, mut pipeline) = components();

        coordinator
            .initialize_live(SequenceNumber::ONE, &mut pipeline)
            .unwrap();

        coordinator
            .start_snapshot(MonotonicTimestamp::new(100), &mut pipeline)
            .unwrap();

        assert!(!coordinator.is_synchronized());
        assert!(!pipeline.is_valid());

        assert!(matches!(
            coordinator.pending_request(),
            Some(RecoveryRequest::Snapshot(_)),
        ));
    }

    #[test]
    fn watermark_detects_missing_event() {
        let (mut coordinator, mut pipeline) = components();

        coordinator
            .initialize_live(SequenceNumber::ONE, &mut pipeline)
            .unwrap();

        coordinator
            .observe_watermark(
                feed(),
                SequenceNumber::ONE,
                MonotonicTimestamp::new(100),
                &mut pipeline,
            )
            .unwrap();

        assert!(!coordinator.is_synchronized());
        assert!(!pipeline.is_valid());
        assert!(coordinator.pending_request().is_some());
    }

    #[test]
    fn blocked_input_and_pending_publication_are_distinct() {
        assert_ne!(
            RecoveryOutcome::InputBlocked,
            RecoveryOutcome::PublicationPending,
        );
    }
}
