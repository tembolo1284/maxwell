//! Outstanding retransmission attempt tracking.

use maxwell_core::{
    EventEnvelope, MonotonicTimestamp, NanoDuration, SequenceNumber, SequenceRange,
};

use crate::error::{RecoveryError, RecoveryResult};
use crate::request::{RequestId, RetransmissionRequest};
use crate::timeout::RecoveryDeadline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetransmissionAttempt {
    request: RetransmissionRequest,
    deadline: RecoveryDeadline,
}

impl RetransmissionAttempt {
    pub fn new(
        request: RetransmissionRequest,
        now: MonotonicTimestamp,
        timeout: NanoDuration,
    ) -> RecoveryResult<Self> {
        if request.attempt == 0 {
            return Err(RecoveryError::InvalidRequest);
        }

        Ok(Self {
            request,
            deadline: RecoveryDeadline::new(now, timeout)?,
        })
    }

    #[must_use]
    pub const fn request(self) -> RetransmissionRequest {
        self.request
    }

    #[must_use]
    pub const fn deadline(self) -> RecoveryDeadline {
        self.deadline
    }

    pub fn is_expired(self, now: MonotonicTimestamp) -> RecoveryResult<bool> {
        self.deadline.is_expired(now)
    }

    /// Completion is determined by committed application
    /// progress, not by a remote "retransmission complete"
    /// notification.
    #[must_use]
    pub fn is_complete(self, next_expected: SequenceNumber) -> bool {
        next_expected > self.request.range.end()
    }

    pub fn validate_event(
        self,
        request_id: RequestId,
        event: &EventEnvelope,
    ) -> RecoveryResult<()> {
        if request_id != self.request.id {
            return Err(RecoveryError::StaleResponse);
        }

        if event.feed() != self.request.feed {
            return Err(RecoveryError::FeedMismatch {
                expected: self.request.feed,
                received: event.feed(),
            });
        }

        if !self.request.contains(event.sequence()) {
            return Err(RecoveryError::InvalidRequest);
        }

        Ok(())
    }

    /// Returns the uncommitted portion of this request.
    #[must_use]
    pub fn remaining_range(self, next_expected: SequenceNumber) -> Option<SequenceRange> {
        let start = next_expected.max(self.request.range.start());

        SequenceRange::new(start, self.request.range.end())
    }
}
