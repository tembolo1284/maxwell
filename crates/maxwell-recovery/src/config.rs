//! Recovery resource limits and retry configuration.

use maxwell_core::NanoDuration;

use crate::error::{RecoveryError, RecoveryResult};

pub const DEFAULT_BUFFER_CAPACITY: usize = 65_536;
pub const DEFAULT_MAX_GAP: u64 = 100_000;
pub const DEFAULT_REQUEST_BATCH: u64 = 4_096;

pub const DEFAULT_RETRANSMISSION_TIMEOUT: NanoDuration = NanoDuration::from_nanos(100_000_000);

pub const DEFAULT_SNAPSHOT_TIMEOUT: NanoDuration = NanoDuration::from_nanos(5_000_000_000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryConfig {
    /// Maximum number of buffered out-of-order events.
    pub buffer_capacity: usize,

    /// Gaps larger than this require a snapshot.
    pub max_recoverable_gap: u64,

    /// Maximum events requested per retransmission.
    pub max_request_batch: u64,

    /// Total attempts per retransmission range.
    pub max_retransmission_attempts: u32,

    /// Total snapshot request attempts.
    pub max_snapshot_attempts: u32,

    pub retransmission_timeout: NanoDuration,
    pub snapshot_timeout: NanoDuration,
}

impl RecoveryConfig {
    /// Validates the recovery limits and timeouts.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfiguration` if a field is out of range.
    pub const fn validate(self) -> RecoveryResult<()> {
        if self.buffer_capacity == 0 {
            return Err(RecoveryError::InvalidConfiguration(
                "buffer capacity must be positive",
            ));
        }

        if self.max_recoverable_gap == 0 {
            return Err(RecoveryError::InvalidConfiguration(
                "maximum recoverable gap must be positive",
            ));
        }

        if self.max_request_batch == 0 || self.max_request_batch > self.max_recoverable_gap {
            return Err(RecoveryError::InvalidConfiguration(
                "invalid retransmission batch size",
            ));
        }

        if self.max_retransmission_attempts == 0 || self.max_snapshot_attempts == 0 {
            return Err(RecoveryError::InvalidConfiguration(
                "attempt limits must be positive",
            ));
        }

        if self.retransmission_timeout.is_zero() || self.snapshot_timeout.is_zero() {
            return Err(RecoveryError::InvalidConfiguration(
                "recovery timeouts must be positive",
            ));
        }

        Ok(())
    }
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            buffer_capacity: DEFAULT_BUFFER_CAPACITY,
            max_recoverable_gap: DEFAULT_MAX_GAP,
            max_request_batch: DEFAULT_REQUEST_BATCH,
            max_retransmission_attempts: 3,
            max_snapshot_attempts: 3,
            retransmission_timeout: DEFAULT_RETRANSMISSION_TIMEOUT,
            snapshot_timeout: DEFAULT_SNAPSHOT_TIMEOUT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(RecoveryConfig::default().validate().is_ok());
    }

    #[test]
    fn zero_capacity_rejected() {
        let config = RecoveryConfig {
            buffer_capacity: 0,
            ..RecoveryConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn oversized_batch_rejected() {
        let config = RecoveryConfig {
            max_recoverable_gap: 10,
            max_request_batch: 11,
            ..RecoveryConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_timeout_rejected() {
        let config = RecoveryConfig {
            snapshot_timeout: NanoDuration::ZERO,
            ..RecoveryConfig::default()
        };

        assert!(config.validate().is_err());
    }
}
