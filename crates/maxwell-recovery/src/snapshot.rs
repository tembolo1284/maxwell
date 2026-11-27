//! Snapshot identity and sequence watermark validation.
//!
//! A snapshot must represent a consistent feed-wide
//! state through its inclusive watermark.

use maxwell_core::{FeedId, SequenceNumber};

use crate::error::{RecoveryError, RecoveryResult};
use crate::request::{RequestId, SnapshotRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotWatermark {
    pub request_id: RequestId,
    pub feed: FeedId,
    pub last_sequence: SequenceNumber,
}

impl SnapshotWatermark {
    #[must_use]
    pub const fn new(request_id: RequestId, feed: FeedId, last_sequence: SequenceNumber) -> Self {
        Self {
            request_id,
            feed,
            last_sequence,
        }
    }

    pub fn next_expected(self) -> RecoveryResult<SequenceNumber> {
        self.last_sequence
            .next()
            .ok_or(RecoveryError::SequenceExhausted)
    }

    pub fn validate(self, request: SnapshotRequest) -> RecoveryResult<()> {
        if self.request_id != request.id {
            return Err(RecoveryError::StaleResponse);
        }

        if self.feed != request.feed {
            return Err(RecoveryError::FeedMismatch {
                expected: request.feed,
                received: self.feed,
            });
        }

        if self.last_sequence < request.minimum_watermark {
            return Err(RecoveryError::SnapshotTooOld);
        }

        self.next_expected()?;

        Ok(())
    }

    #[must_use]
    pub fn covers(self, sequence: SequenceNumber) -> bool {
        sequence <= self.last_sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::state::RecoveryReason;
    use maxwell_core::{SessionId, StreamId};

    fn request() -> SnapshotRequest {
        SnapshotRequest {
            id: RequestId::FIRST,
            feed: FeedId::new(StreamId::new(1), SessionId::new(1)),
            minimum_watermark: SequenceNumber::new(100),
            reason: RecoveryReason::GapTooLarge,
            attempt: 1,
        }
    }

    #[test]
    fn valid_snapshot() {
        let request = request();

        let snapshot = SnapshotWatermark::new(request.id, request.feed, SequenceNumber::new(150));

        assert!(snapshot.validate(request).is_ok());

        assert_eq!(snapshot.next_expected().unwrap(), SequenceNumber::new(151),);
    }

    #[test]
    fn old_snapshot_rejected() {
        let request = request();

        let snapshot = SnapshotWatermark::new(request.id, request.feed, SequenceNumber::new(99));

        assert_eq!(
            snapshot.validate(request),
            Err(RecoveryError::SnapshotTooOld),
        );
    }

    #[test]
    fn stale_request_rejected() {
        let request = request();

        let snapshot =
            SnapshotWatermark::new(RequestId::new(99), request.feed, SequenceNumber::new(150));

        assert_eq!(
            snapshot.validate(request),
            Err(RecoveryError::StaleResponse),
        );
    }
}
