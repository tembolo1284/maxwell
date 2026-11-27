
//! Sequencer configuration and resource limits.

use crate::error::{SequencerError, SequencerResult};

pub const DEFAULT_REORDER_CAPACITY: usize = 4_096;
pub const MAX_REORDER_CAPACITY: usize = 1_048_576;
pub const DEFAULT_MAX_STREAMS: usize = 64;
pub const MAX_STREAMS: usize = 4_096;

/// Configuration shared by stream sequencers.
///
/// Reorder capacity must be a nonzero power of two,
/// allowing efficient sequence-to-slot mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencerConfig {
    pub reorder_capacity: usize,
    pub max_streams: usize,
}

impl SequencerConfig {
    #[must_use]
    pub const fn new(
        reorder_capacity: usize,
        max_streams: usize,
    ) -> Self {
        Self {
            reorder_capacity,
            max_streams,
        }
    }

    /// Validates resource limits before allocation.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported resource limits.
    pub const fn validate(self) -> SequencerResult<()> {
        if self.reorder_capacity == 0
            || self.reorder_capacity > MAX_REORDER_CAPACITY
            || !self.reorder_capacity.is_power_of_two()
        {
            return Err(
                SequencerError::InvalidReorderCapacity(
                    self.reorder_capacity,
                ),
            );
        }

        if self.max_streams == 0
            || self.max_streams > MAX_STREAMS
        {
            return Err(
                SequencerError::InvalidStreamCapacity(
                    self.max_streams,
                ),
            );
        }

        Ok(())
    }
}

impl Default for SequencerConfig {
    fn default() -> Self {
        Self {
            reorder_capacity: DEFAULT_REORDER_CAPACITY,
            max_streams: DEFAULT_MAX_STREAMS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_valid() {
        assert!(SequencerConfig::default().validate().is_ok());
    }

    #[test]
    fn zero_capacity_rejected() {
        assert!(SequencerConfig::new(0, 1).validate().is_err());
    }

    #[test]
    fn non_power_of_two_rejected() {
        assert!(SequencerConfig::new(100, 1).validate().is_err());
    }

    #[test]
    fn excessive_capacity_rejected() {
        assert!(
            SequencerConfig::new(
                MAX_REORDER_CAPACITY * 2,
                1,
            )
            .validate()
            .is_err()
        );
    }

    #[test]
    fn zero_streams_rejected() {
        assert!(SequencerConfig::new(16, 0).validate().is_err());
    }

    #[test]
    fn minimum_configuration_valid() {
        assert!(SequencerConfig::new(1, 1).validate().is_ok());
    }
}

