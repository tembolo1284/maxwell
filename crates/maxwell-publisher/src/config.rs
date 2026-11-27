//! Publisher configuration and backpressure policies.

use crate::error::PublisherError;

/// Defines how the publisher handles a full subscriber queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackpressurePolicy {
    /// Reject the publication atomically across all subscribers.
    RejectPublication,

    /// Invalidate the slow subscriber and continue publication
    /// to healthy subscribers.
    #[default]
    RequireResync,
}

/// Resource limits for a publisher instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublisherConfig {
    pub max_subscribers: usize,
    pub queue_capacity: usize,
    pub max_instruments_per_subscription: usize,
    pub backpressure_policy: BackpressurePolicy,
}

impl PublisherConfig {
    /// Validates configuration before allocating publisher resources.
    pub fn validate(&self) -> Result<(), PublisherError> {
        if self.max_subscribers == 0 {
            return Err(PublisherError::InvalidConfig(
                "max_subscribers must be greater than zero",
            ));
        }

        if self.queue_capacity == 0 {
            return Err(PublisherError::InvalidConfig(
                "queue_capacity must be greater than zero",
            ));
        }

        if self.max_instruments_per_subscription == 0 {
            return Err(PublisherError::InvalidConfig(
                "max_instruments_per_subscription must be greater than zero",
            ));
        }

        if self
            .max_subscribers
            .checked_mul(self.queue_capacity)
            .is_none()
        {
            return Err(PublisherError::InvalidConfig(
                "total queue capacity overflows usize",
            ));
        }

        Ok(())
    }
}

impl Default for PublisherConfig {
    fn default() -> Self {
        Self {
            max_subscribers: 64,
            queue_capacity: 4096,
            max_instruments_per_subscription: 256,
            backpressure_policy: BackpressurePolicy::RequireResync,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_is_valid() {
        assert!(PublisherConfig::default().validate().is_ok());
    }

    #[test]
    fn rejects_zero_subscribers() {
        let config = PublisherConfig {
            max_subscribers: 0,
            ..PublisherConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_queue_capacity() {
        let config = PublisherConfig {
            queue_capacity: 0,
            ..PublisherConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_instrument_capacity() {
        let config = PublisherConfig {
            max_instruments_per_subscription: 0,
            ..PublisherConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_capacity_overflow() {
        let config = PublisherConfig {
            max_subscribers: usize::MAX,
            queue_capacity: 2,
            ..PublisherConfig::default()
        };

        assert!(config.validate().is_err());
    }
}
