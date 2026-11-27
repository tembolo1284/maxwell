//! Client configuration and resource limits.

use crate::error::ClientError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientConfig {
    pub heartbeat_interval_ns: u64,
    pub server_timeout_ns: u64,
    pub max_snapshot_entries: usize,
}

impl ClientConfig {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.heartbeat_interval_ns == 0 {
            return Err(ClientError::InvalidConfig(
                "heartbeat interval must be greater than zero",
            ));
        }

        if self.server_timeout_ns <= self.heartbeat_interval_ns {
            return Err(ClientError::InvalidConfig(
                "server timeout must exceed heartbeat interval",
            ));
        }

        if self.max_snapshot_entries == 0 {
            return Err(ClientError::InvalidConfig(
                "snapshot capacity must be greater than zero",
            ));
        }

        Ok(())
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval_ns: 5_000_000_000,
            server_timeout_ns: 15_000_000_000,
            max_snapshot_entries: 65_536,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_is_valid() {
        assert!(ClientConfig::default().validate().is_ok());
    }

    #[test]
    fn rejects_zero_heartbeat_interval() {
        let config = ClientConfig {
            heartbeat_interval_ns: 0,
            ..ClientConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_timeout() {
        let config = ClientConfig {
            heartbeat_interval_ns: 100,
            server_timeout_ns: 100,
            ..ClientConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_snapshot_capacity() {
        let config = ClientConfig {
            max_snapshot_entries: 0,
            ..ClientConfig::default()
        };

        assert!(config.validate().is_err());
    }
}
