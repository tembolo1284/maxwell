//! Deterministic recovery deadlines.
//!
//! Time is supplied by the caller. No sleeping,
//! wall-clock acquisition, or background timers.

use maxwell_core::{MonotonicTimestamp, NanoDuration};

use crate::error::{RecoveryError, RecoveryResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryDeadline {
    started_at: MonotonicTimestamp,
    expires_at: MonotonicTimestamp,
}

impl RecoveryDeadline {
    /// Creates a deadline `timeout` after `now`.
    ///
    /// # Errors
    ///
    /// Returns `DeadlineOverflow` if the expiry is not representable.
    pub fn new(now: MonotonicTimestamp, timeout: NanoDuration) -> RecoveryResult<Self> {
        let expires_at = now
            .checked_add(timeout)
            .ok_or(RecoveryError::DeadlineOverflow)?;

        Ok(Self {
            started_at: now,
            expires_at,
        })
    }

    #[must_use]
    pub const fn started_at(self) -> MonotonicTimestamp {
        self.started_at
    }

    #[must_use]
    pub const fn expires_at(self) -> MonotonicTimestamp {
        self.expires_at
    }

    /// Reports whether the deadline has passed.
    ///
    /// # Errors
    ///
    /// Returns `ClockWentBackward` if `now` precedes the creation time.
    pub fn is_expired(self, now: MonotonicTimestamp) -> RecoveryResult<bool> {
        if now < self.started_at {
            return Err(RecoveryError::ClockWentBackward);
        }

        Ok(now >= self.expires_at)
    }

    /// Returns the time elapsed since creation.
    ///
    /// # Errors
    ///
    /// Returns `ClockWentBackward` if `now` precedes the creation time.
    pub fn elapsed(self, now: MonotonicTimestamp) -> RecoveryResult<NanoDuration> {
        now.duration_since(self.started_at)
            .ok_or(RecoveryError::ClockWentBackward)
    }

    /// Returns the time left before expiry.
    ///
    /// # Errors
    ///
    /// Returns `ClockWentBackward` if `now` precedes the creation time.
    pub fn remaining(self, now: MonotonicTimestamp) -> RecoveryResult<NanoDuration> {
        if now < self.started_at {
            return Err(RecoveryError::ClockWentBackward);
        }

        Ok(self
            .expires_at
            .duration_since(now)
            .unwrap_or(NanoDuration::ZERO))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiration_boundary() {
        let deadline =
            RecoveryDeadline::new(MonotonicTimestamp::new(100), NanoDuration::from_nanos(50))
                .unwrap();

        assert!(!deadline.is_expired(MonotonicTimestamp::new(149)).unwrap());

        assert!(deadline.is_expired(MonotonicTimestamp::new(150)).unwrap());
    }

    #[test]
    fn remaining_after_expiration() {
        let deadline =
            RecoveryDeadline::new(MonotonicTimestamp::new(100), NanoDuration::from_nanos(50))
                .unwrap();

        assert_eq!(
            deadline.remaining(MonotonicTimestamp::new(200)).unwrap(),
            NanoDuration::ZERO,
        );
    }

    #[test]
    fn deadline_overflow() {
        assert!(matches!(
            RecoveryDeadline::new(MonotonicTimestamp::MAX, NanoDuration::from_nanos(1),),
            Err(RecoveryError::DeadlineOverflow),
        ));
    }

    #[test]
    fn backward_clock_rejected() {
        let deadline =
            RecoveryDeadline::new(MonotonicTimestamp::new(100), NanoDuration::from_nanos(50))
                .unwrap();

        assert!(matches!(
            deadline.is_expired(MonotonicTimestamp::new(99)),
            Err(RecoveryError::ClockWentBackward),
        ));
    }
}
