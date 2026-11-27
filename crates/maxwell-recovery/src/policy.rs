//! Recovery strategy selection.

use maxwell_core::SequenceRange;

use crate::config::RecoveryConfig;
use crate::error::{RecoveryError, RecoveryResult};
use crate::state::RecoveryReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDecision {
    Retransmit(SequenceRange),
    Snapshot(RecoveryReason),
    Fail(RecoveryReason),
}

#[derive(Debug, Clone, Copy)]
pub struct RecoveryPolicy {
    config: RecoveryConfig,
}

impl RecoveryPolicy {
    pub fn new(config: RecoveryConfig) -> RecoveryResult<Self> {
        config.validate()?;

        Ok(Self { config })
    }

    #[must_use]
    pub const fn config(self) -> RecoveryConfig {
        self.config
    }

    /// Selects a bounded retransmission batch or
    /// escalates an excessively large gap.
    pub fn on_gap(self, missing: SequenceRange) -> RecoveryResult<RecoveryDecision> {
        if missing.count() > u128::from(self.config.max_recoverable_gap) {
            return Ok(RecoveryDecision::Snapshot(RecoveryReason::GapTooLarge));
        }

        let count = missing
            .checked_count()
            .ok_or(RecoveryError::InvalidRequest)?;

        let batch_size = count.min(self.config.max_request_batch);

        let batch = SequenceRange::from_count(missing.start(), batch_size)
            .ok_or(RecoveryError::InvalidRequest)?;

        Ok(RecoveryDecision::Retransmit(batch))
    }

    #[must_use]
    pub const fn can_retry_retransmission(self, attempts: u32) -> bool {
        attempts < self.config.max_retransmission_attempts
    }

    #[must_use]
    pub const fn can_retry_snapshot(self, attempts: u32) -> bool {
        attempts < self.config.max_snapshot_attempts
    }

    #[must_use]
    pub const fn on_buffer_overflow(self) -> RecoveryDecision {
        RecoveryDecision::Snapshot(RecoveryReason::BufferOverflow)
    }

    #[must_use]
    pub const fn on_retransmission_unavailable(self) -> RecoveryDecision {
        RecoveryDecision::Snapshot(RecoveryReason::RetransmissionUnavailable)
    }

    #[must_use]
    pub const fn on_snapshot_exhausted(self) -> RecoveryDecision {
        RecoveryDecision::Fail(RecoveryReason::SnapshotTimeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxwell_core::SequenceNumber;

    fn policy() -> RecoveryPolicy {
        RecoveryPolicy::new(RecoveryConfig {
            max_recoverable_gap: 100,
            max_request_batch: 10,
            ..RecoveryConfig::default()
        })
        .unwrap()
    }

    #[test]
    fn small_gap_retransmitted() {
        let range = SequenceRange::new(SequenceNumber::new(1), SequenceNumber::new(5)).unwrap();

        assert_eq!(
            policy().on_gap(range).unwrap(),
            RecoveryDecision::Retransmit(range),
        );
    }

    #[test]
    fn retransmission_batch_is_bounded() {
        let range = SequenceRange::new(SequenceNumber::new(1), SequenceNumber::new(50)).unwrap();

        let expected = SequenceRange::new(SequenceNumber::new(1), SequenceNumber::new(10)).unwrap();

        assert_eq!(
            policy().on_gap(range).unwrap(),
            RecoveryDecision::Retransmit(expected),
        );
    }

    #[test]
    fn large_gap_requires_snapshot() {
        let range = SequenceRange::new(SequenceNumber::new(1), SequenceNumber::new(101)).unwrap();

        assert_eq!(
            policy().on_gap(range).unwrap(),
            RecoveryDecision::Snapshot(RecoveryReason::GapTooLarge),
        );
    }
}
