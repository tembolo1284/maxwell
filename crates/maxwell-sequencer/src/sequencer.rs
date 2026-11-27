//! Multi-stream sequencing coordinator.
//!
//! Stream registration and removal occur on the control
//! path. Event ingestion performs no allocation.
//!
//! Each stream has an independent sequence namespace.

use std::collections::HashMap;

use maxwell_core::{EventEnvelope, FeedId, SequenceNumber, StreamId};

use crate::config::SequencerConfig;
use crate::error::{SequencerError, SequencerResult};
use crate::stream::{IngestOutcome, StreamSequencer};

#[derive(Debug)]
pub struct Sequencer {
    config: SequencerConfig,
    streams: HashMap<StreamId, StreamSequencer>,
}

impl Sequencer {
    /// Creates a sequencer with preallocated stream slots.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid, or
    /// `AllocationFailed` if storage cannot be reserved.
    pub fn new(config: SequencerConfig) -> SequencerResult<Self> {
        config.validate()?;

        let mut streams = HashMap::new();

        streams
            .try_reserve(config.max_streams)
            .map_err(|_| SequencerError::AllocationFailed)?;

        Ok(Self { config, streams })
    }

    #[must_use]
    #[inline]
    pub const fn config(&self) -> SequencerConfig {
        self.config
    }

    #[must_use]
    #[inline]
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    /// Registers an authoritative feed identity and its
    /// initial expected sequence.
    ///
    /// This is a control-path operation.
    ///
    /// # Errors
    ///
    /// Returns `StreamAlreadyRegistered` for a duplicate,
    /// `StreamCapacityExceeded` if no slot is free, or the error
    /// from constructing the stream.
    pub fn register(
        &mut self,
        stream_id: StreamId,
        feed: FeedId,
        expected: SequenceNumber,
    ) -> SequencerResult<()> {
        if self.streams.contains_key(&stream_id) {
            return Err(SequencerError::StreamAlreadyRegistered(stream_id));
        }

        if self.streams.len() >= self.config.max_streams {
            return Err(SequencerError::StreamCapacityExceeded);
        }

        let stream = StreamSequencer::new(feed, expected, self.config)?;

        self.streams.insert(stream_id, stream);

        Ok(())
    }

    #[must_use]
    #[inline]
    pub fn stream(&self, stream_id: StreamId) -> Option<&StreamSequencer> {
        self.streams.get(&stream_id)
    }

    /// Removes a stream and releases its reorder buffer.
    pub fn remove(&mut self, stream_id: StreamId) -> Option<StreamSequencer> {
        self.streams.remove(&stream_id)
    }

    /// Routes an event to its registered stream sequencer.
    ///
    /// # Errors
    ///
    /// Returns `UnknownStream` if the stream is not registered.
    /// Otherwise propagates the error from the stream.
    #[inline]
    pub fn ingest<F>(&mut self, event: EventEnvelope, deliver: F) -> SequencerResult<IngestOutcome>
    where
        F: FnMut(EventEnvelope),
    {
        let stream_id = event.header.stream_id();

        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(SequencerError::UnknownStream(stream_id))?;

        stream.ingest(event, deliver)
    }

    /// Marks one stream as requiring recovery.
    ///
    /// # Errors
    ///
    /// Returns `UnknownStream` if the stream is not registered.
    pub fn require_recovery(&mut self, stream_id: StreamId) -> SequencerResult<()> {
        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(SequencerError::UnknownStream(stream_id))?;

        stream.require_recovery();

        Ok(())
    }

    /// Installs a recovery checkpoint on one stream.
    ///
    /// # Errors
    ///
    /// Returns `UnknownStream` if the stream is not registered.
    /// Otherwise propagates the error from the stream.
    pub fn install_checkpoint(
        &mut self,
        stream_id: StreamId,
        expected: SequenceNumber,
    ) -> SequencerResult<()> {
        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(SequencerError::UnknownStream(stream_id))?;

        stream.install_checkpoint(expected)
    }

    /// Starts a new upstream session on one stream.
    ///
    /// # Errors
    ///
    /// Returns `UnknownStream` if the stream is not registered.
    /// Otherwise propagates the error from the stream.
    pub fn start_session(
        &mut self,
        stream_id: StreamId,
        feed: FeedId,
        expected: SequenceNumber,
    ) -> SequencerResult<()> {
        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(SequencerError::UnknownStream(stream_id))?;

        stream.start_session(feed, expected)
    }
}
