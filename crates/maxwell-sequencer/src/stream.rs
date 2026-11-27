//! Single-feed sequencing engine.
//!
//! Events are delivered through a synchronous callback.
//! The callback must accept every event without failure
//! or panic. A downstream failure requires coordinated
//! recovery before sequencing resumes.

use maxwell_core::{EventEnvelope, FeedId, SequenceNumber};

use crate::config::SequencerConfig;
use crate::dedup::SequenceDisposition;
use crate::error::{SequencerError, SequencerResult};
use crate::gap::SequenceGap;
use crate::metrics::SequencerMetrics;
use crate::reorder::{InsertOutcome, ReorderBuffer};
use crate::state::SequencerState;
use crate::tracker::SequenceTracker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    Delivered { count: usize },
    Buffered,
    Duplicate,
}

#[derive(Debug)]
pub struct StreamSequencer {
    feed: FeedId,
    tracker: SequenceTracker,
    reorder: ReorderBuffer,
    state: SequencerState,
    metrics: SequencerMetrics,
}

impl StreamSequencer {
    pub fn new(
        feed: FeedId,
        expected: SequenceNumber,
        config: SequencerConfig,
    ) -> SequencerResult<Self> {
        config.validate()?;

        Ok(Self {
            feed,
            tracker: SequenceTracker::new(expected),
            reorder: ReorderBuffer::new(config.reorder_capacity)?,
            state: SequencerState::Live,
            metrics: SequencerMetrics::new(),
        })
    }

    #[must_use]
    #[inline]
    pub const fn feed(&self) -> FeedId {
        self.feed
    }

    #[must_use]
    #[inline]
    pub const fn state(&self) -> SequencerState {
        self.state
    }

    #[must_use]
    #[inline]
    pub const fn expected(&self) -> Option<SequenceNumber> {
        self.tracker.expected()
    }

    #[must_use]
    #[inline]
    pub const fn last_delivered(&self) -> Option<SequenceNumber> {
        self.tracker.last_delivered()
    }

    #[must_use]
    #[inline]
    pub const fn buffered(&self) -> usize {
        self.reorder.len()
    }

    #[must_use]
    #[inline]
    pub const fn metrics(&self) -> SequencerMetrics {
        self.metrics
    }

    /// Returns the first contiguous missing range preceding
    /// the nearest buffered event.
    ///
    /// O(capacity); intended for recovery orchestration.
    #[must_use]
    pub fn current_gap(&self) -> Option<SequenceGap> {
        let expected = self.tracker.expected()?;

        let nearest = self.reorder.lowest_sequence(expected)?;

        SequenceGap::between(expected, nearest)
    }

    /// Processes an event and delivers all newly contiguous
    /// events in strictly increasing sequence order.
    ///
    /// No allocation is performed by the sequencer.
    ///
    /// The delivery callback must not fail or panic.
    pub fn ingest<F>(
        &mut self,
        event: EventEnvelope,
        mut deliver: F,
    ) -> SequencerResult<IngestOutcome>
    where
        F: FnMut(EventEnvelope),
    {
        self.metrics.on_received();

        let actual_feed = FeedId::new(event.header.stream_id(), event.header.session_id());

        if actual_feed != self.feed {
            self.metrics.on_rejected();

            return Err(SequencerError::FeedMismatch {
                expected: self.feed,
                actual: actual_feed,
            });
        }

        if self.state.requires_recovery() {
            self.metrics.on_rejected();

            return Err(SequencerError::RecoveryRequired);
        }

        if self.state.is_exhausted() {
            self.metrics.on_rejected();

            return Err(SequencerError::SequenceExhausted);
        }

        if let Err(error) = event.validate() {
            self.metrics.on_rejected();
            self.require_recovery();

            return Err(error.into());
        }

        let received = event.header.sequence();

        match self.tracker.classify(received) {
            SequenceDisposition::PreviouslyDelivered => {
                self.metrics.on_duplicate();

                Ok(IngestOutcome::Duplicate)
            }

            SequenceDisposition::Expected => {
                self.deliver_one(event, &mut deliver)?;

                let mut count = 1;

                // Drain every contiguous buffered event.
                while let Some(expected) = self.tracker.expected() {
                    let Some(next) = self.reorder.take(expected) else {
                        break;
                    };

                    self.deliver_one(next, &mut deliver)?;

                    count += 1;
                }

                self.refresh_state();

                Ok(IngestOutcome::Delivered { count })
            }

            SequenceDisposition::Future { .. } => {
                let expected = self
                    .tracker
                    .expected()
                    .ok_or(SequencerError::SequenceExhausted)?;

                match self.reorder.insert(expected, event) {
                    Ok(InsertOutcome::Inserted) => {
                        self.metrics.on_out_of_order(self.reorder.len());

                        if self.state.is_live() {
                            self.metrics.on_gap_detected();
                        }

                        self.state = SequencerState::GapDetected;

                        Ok(IngestOutcome::Buffered)
                    }

                    Ok(InsertOutcome::Duplicate) => {
                        self.metrics.on_duplicate();

                        Ok(IngestOutcome::Duplicate)
                    }

                    Err(error) => {
                        self.metrics.on_rejected();
                        self.require_recovery();

                        Err(error)
                    }
                }
            }

            SequenceDisposition::Exhausted => {
                self.metrics.on_rejected();

                Err(SequencerError::SequenceExhausted)
            }
        }
    }

    #[inline]
    fn deliver_one<F>(&mut self, event: EventEnvelope, deliver: &mut F) -> SequencerResult<()>
    where
        F: FnMut(EventEnvelope),
    {
        let sequence = event.header.sequence();

        // Temporarily fail closed during downstream delivery.
        // If the callback panics and the caller catches it,
        // the stream cannot silently resume.
        let previous_state = self.state;
        self.state = SequencerState::RecoveryRequired;

        deliver(event);

        self.tracker.commit(sequence)?;
        self.metrics.on_delivered();

        self.state = previous_state;

        Ok(())
    }

    #[inline]
    fn refresh_state(&mut self) {
        let previous = self.state;

        self.state = if self.tracker.is_exhausted() {
            SequencerState::Exhausted
        } else if self.reorder.is_empty() {
            SequencerState::Live
        } else {
            SequencerState::GapDetected
        };

        if previous.has_gap() && !self.state.has_gap() {
            self.metrics.on_gap_resolved();
        }
    }

    /// Explicitly blocks delivery until recovery completes.
    ///
    /// May be called by an external gap timeout, transport
    /// failure, or downstream consistency check.
    pub fn require_recovery(&mut self) {
        if !self.state.requires_recovery() {
            self.metrics.on_recovery_requested();
        }

        self.state = SequencerState::RecoveryRequired;
    }

    /// Installs a same-session recovery checkpoint.
    ///
    /// The caller must first restore the downstream book to
    /// the state immediately preceding `expected`.
    ///
    /// Buffered events are discarded. The recovery layer
    /// must replay any events still needed after this point.
    pub fn install_checkpoint(&mut self, expected: SequenceNumber) -> SequencerResult<()> {
        let current = self
            .tracker
            .expected()
            .ok_or(SequencerError::SequenceExhausted)?;

        if expected < current {
            return Err(SequencerError::InvalidRecoverySequence);
        }

        self.reorder.clear();
        self.tracker.reset(expected);
        self.state = SequencerState::Live;

        self.metrics.on_recovery_completed();

        Ok(())
    }

    /// Explicitly starts a new feed session.
    ///
    /// The caller must validate session authenticity and
    /// initialize downstream state before calling this.
    ///
    /// Reusing the current feed identity is prohibited.
    pub fn start_session(&mut self, feed: FeedId, expected: SequenceNumber) -> SequencerResult<()> {
        if feed == self.feed {
            return Err(SequencerError::InvalidRecoverySequence);
        }

        self.reorder.clear();
        self.tracker.reset(expected);
        self.feed = feed;
        self.state = SequencerState::Live;

        self.metrics.on_session_changed();

        Ok(())
    }
}
