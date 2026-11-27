//! Gateway resource limits and timing configuration.

use maxwell_publisher::PublisherConfig;

use crate::error::GatewayError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayConfig {
    pub publisher: PublisherConfig,
    pub heartbeat_interval_ns: u64,
    pub session_timeout_ns: u64,
    pub max_messages_per_service: usize,
}

impl GatewayConfig {
    pub fn validate(&self) -> Result<(), GatewayError> {
        self.publisher.validate()?;

        if self.heartbeat_interval_ns == 0 {
            return Err(GatewayError::InvalidConfig(
                "heartbeat interval must be greater than zero",
            ));
        }

        if self.session_timeout_ns <= self.heartbeat_interval_ns {
            return Err(GatewayError::InvalidConfig(
                "session timeout must exceed heartbeat interval",
            ));
        }

        if self.max_messages_per_service == 0 {
            return Err(GatewayError::InvalidConfig(
                "service budget must be greater than zero",
            ));
        }

        Ok(())
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            publisher: PublisherConfig::default(),
            heartbeat_interval_ns: 5_000_000_000,
            session_timeout_ns: 15_000_000_000,
            max_messages_per_service: 256,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_is_valid() {
        assert!(GatewayConfig::default().validate().is_ok());
    }

    #[test]
    fn rejects_zero_heartbeat_interval() {
        let config = GatewayConfig {
            heartbeat_interval_ns: 0,
            ..GatewayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_session_timeout() {
        let config = GatewayConfig {
            heartbeat_interval_ns: 100,
            session_timeout_ns: 100,
            ..GatewayConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_service_budget() {
        let config = GatewayConfig {
            max_messages_per_service: 0,
            ..GatewayConfig::default()
        };

        assert!(config.validate().is_err());
    }
}
