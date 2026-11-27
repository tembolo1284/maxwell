//! Server configuration and application-level resource limits.
//!
//! The server aggregates configuration from the foundational
//! Maxwell crates rather than duplicating component-specific
//! configuration or validation logic.
//!
//! Application-level configuration controls processing budgets,
//! snapshot limits, and graceful shutdown behavior.

use std::time::Duration;

use maxwell_book::{BookConfig, BookError};
use maxwell_gateway::{GatewayConfig, GatewayError};
use maxwell_recovery::{RecoveryConfig, RecoveryError};
use maxwell_sequencer::{SequencerConfig, SequencerError};
use maxwell_transport::{
    FramingConfig, TcpConfig, TransportError, UdpConfig,
};

use crate::error::{ServerError, ServerResult};

// ============================================================
// Default application limits
// ============================================================

/// Maximum upstream events processed during one service iteration.
pub const DEFAULT_UPSTREAM_BUDGET: usize = 256;

/// Maximum downstream service operations per iteration.
pub const DEFAULT_DOWNSTREAM_BUDGET: usize = 64;

/// Maximum recovery operations per service iteration.
pub const DEFAULT_RECOVERY_BUDGET: usize = 256;

/// Maximum entries permitted in a downstream snapshot.
pub const DEFAULT_MAX_SNAPSHOT_ENTRIES: usize = 65_536;

/// Maximum time allowed for graceful shutdown.
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration =
    Duration::from_secs(5);

// ============================================================
// Processing configuration
// ============================================================

/// Application-level processing budgets.
///
/// Budgets prevent any individual processing stage from
/// monopolizing the single-owner server event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessingConfig {
    /// Maximum upstream events processed per iteration.
    pub upstream_budget: usize,

    /// Maximum downstream service operations per iteration.
    ///
    /// This is separate from the gateway's per-service
    /// message budget.
    pub downstream_budget: usize,

    /// Maximum recovery operations per iteration.
    pub recovery_budget: usize,
}

impl ProcessingConfig {
    /// Validates the application processing budgets.
    ///
    /// # Errors
    ///
    /// Returns an error if any processing budget is zero.
    pub fn validate(&self) -> ServerResult<()> {
        if self.upstream_budget == 0 {
            return Err(ServerError::InvalidConfig(
                "upstream budget must be greater than zero",
            ));
        }

        if self.downstream_budget == 0 {
            return Err(ServerError::InvalidConfig(
                "downstream budget must be greater than zero",
            ));
        }

        if self.recovery_budget == 0 {
            return Err(ServerError::InvalidConfig(
                "recovery budget must be greater than zero",
            ));
        }

        Ok(())
    }
}

impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            upstream_budget: DEFAULT_UPSTREAM_BUDGET,
            downstream_budget: DEFAULT_DOWNSTREAM_BUDGET,
            recovery_budget: DEFAULT_RECOVERY_BUDGET,
        }
    }
}

// ============================================================
// Snapshot configuration
// ============================================================

/// Resource limits for downstream snapshot generation.
///
/// This configuration does not imply that the upstream
/// market data source supports authoritative snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotConfig {
    /// Maximum entries permitted in a generated snapshot.
    pub max_entries: usize,
}

impl SnapshotConfig {
    /// Validates downstream snapshot limits.
    ///
    /// # Errors
    ///
    /// Returns an error if snapshot capacity is zero.
    pub fn validate(&self) -> ServerResult<()> {
        if self.max_entries == 0 {
            return Err(ServerError::InvalidConfig(
                "snapshot capacity must be greater than zero",
            ));
        }

        Ok(())
    }
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_SNAPSHOT_ENTRIES,
        }
    }
}

// ============================================================
// Shutdown configuration
// ============================================================

/// Configuration for controlled server shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownConfig {
    /// Maximum duration permitted for graceful shutdown.
    pub timeout: Duration,
}

impl ShutdownConfig {
    /// Validates shutdown configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the shutdown timeout is zero.
    pub fn validate(&self) -> ServerResult<()> {
        if self.timeout.is_zero() {
            return Err(ServerError::InvalidConfig(
                "shutdown timeout must be greater than zero",
            ));
        }

        Ok(())
    }
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }
}

// ============================================================
// Network configuration
// ============================================================

/// Transport configuration used by the server.
///
/// Endpoint selection and connection establishment are
/// handled separately from transport resource limits.
///
/// The initial in-memory server does not open network sockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetworkConfig {
    /// Upstream UDP transport configuration.
    pub upstream_udp: UdpConfig,

    /// Upstream TCP transport configuration.
    pub upstream_tcp: TcpConfig,

    /// Downstream TCP transport configuration.
    pub downstream_tcp: TcpConfig,

    /// Upstream TCP framing configuration.
    pub upstream_framing: FramingConfig,

    /// Downstream TCP framing configuration.
    pub downstream_framing: FramingConfig,
}

impl NetworkConfig {
    /// Validates transport and framing configuration.
    ///
    /// The initial server uses a single-owner processing
    /// model and therefore requires nonblocking sockets.
    ///
    /// # Errors
    ///
    /// Returns an error if a transport configuration is
    /// invalid or blocking socket operation is requested.
    pub fn validate(&self) -> ServerResult<()> {
        self.upstream_udp
            .validate()
            .map_err(ServerError::Transport)?;

        self.upstream_tcp
            .validate()
            .map_err(ServerError::Transport)?;

        self.downstream_tcp
            .validate()
            .map_err(ServerError::Transport)?;

        self.upstream_framing
            .validate()
            .map_err(ServerError::Transport)?;

        self.downstream_framing
            .validate()
            .map_err(ServerError::Transport)?;

        if !self.upstream_udp.socket.nonblocking {
            return Err(ServerError::InvalidConfig(
                "upstream UDP socket must be nonblocking",
            ));
        }

        if !self.upstream_tcp.socket.nonblocking {
            return Err(ServerError::InvalidConfig(
                "upstream TCP socket must be nonblocking",
            ));
        }

        if !self.downstream_tcp.socket.nonblocking {
            return Err(ServerError::InvalidConfig(
                "downstream TCP socket must be nonblocking",
            ));
        }

        Ok(())
    }
}

// ============================================================
// Server configuration
// ============================================================

/// Complete configuration for a Maxwell server instance.
///
/// Component-specific configuration remains owned by its
/// corresponding foundational crate.
///
/// Publisher configuration is accessed through `gateway.publisher`
/// to avoid maintaining multiple sources of configuration truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ServerConfig {
    /// Upstream sequencing and reorder limits.
    pub sequencer: SequencerConfig,

    /// Order book resource limits.
    pub book: BookConfig,

    /// Upstream recovery limits and retry policies.
    pub recovery: RecoveryConfig,

    /// Downstream gateway and publisher configuration.
    pub gateway: GatewayConfig,

    /// Transport and framing configuration.
    pub network: NetworkConfig,

    /// Application-level processing budgets.
    pub processing: ProcessingConfig,

    /// Downstream snapshot resource limits.
    pub snapshot: SnapshotConfig,

    /// Controlled shutdown configuration.
    pub shutdown: ShutdownConfig,
}

impl ServerConfig {
    /// Validates all component and application configuration.
    ///
    /// Validation must complete successfully before the server
    /// allocates processing resources or opens network sockets.
    ///
    /// # Errors
    ///
    /// Returns the first component or application configuration
    /// error encountered.
    pub fn validate(&self) -> ServerResult<()> {
        self.sequencer
            .validate()
            .map_err(ServerError::Sequencer)?;

        self.book
            .validate()
            .map_err(ServerError::Book)?;

        self.recovery
            .validate()
            .map_err(ServerError::Recovery)?;

        self.gateway
            .validate()
            .map_err(ServerError::Gateway)?;

        self.network.validate()?;
        self.processing.validate()?;
        self.snapshot.validate()?;
        self.shutdown.validate()?;

        Ok(())
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_publisher::BackpressurePolicy;

    // --------------------------------------------------------
    // Default configuration
    // --------------------------------------------------------

    #[test]
    fn default_server_configuration_is_valid() {
        let config = ServerConfig::default();

        assert!(config.validate().is_ok());
    }

    #[test]
    fn default_backpressure_policy_requires_resync() {
        let config = ServerConfig::default();

        assert_eq!(
            config.gateway.publisher.backpressure_policy,
            BackpressurePolicy::RequireResync,
        );
    }

    #[test]
    fn reject_publication_policy_is_supported() {
        let mut config = ServerConfig::default();

        config.gateway.publisher.backpressure_policy =
            BackpressurePolicy::RejectPublication;

        assert!(config.validate().is_ok());
    }

    // --------------------------------------------------------
    // Processing configuration
    // --------------------------------------------------------

    #[test]
    fn rejects_zero_upstream_budget() {
        let config = ProcessingConfig {
            upstream_budget: 0,
            ..ProcessingConfig::default()
        };

        assert!(matches!(
            config.validate(),
            Err(ServerError::InvalidConfig(_)),
        ));
    }

    #[test]
    fn rejects_zero_downstream_budget() {
        let config = ProcessingConfig {
            downstream_budget: 0,
            ..ProcessingConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_recovery_budget() {
        let config = ProcessingConfig {
            recovery_budget: 0,
            ..ProcessingConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn minimum_processing_budgets_are_valid() {
        let config = ProcessingConfig {
            upstream_budget: 1,
            downstream_budget: 1,
            recovery_budget: 1,
        };

        assert!(config.validate().is_ok());
    }

    // --------------------------------------------------------
    // Snapshot configuration
    // --------------------------------------------------------

    #[test]
    fn rejects_zero_snapshot_capacity() {
        let config = SnapshotConfig {
            max_entries: 0,
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn minimum_snapshot_capacity_is_valid() {
        let config = SnapshotConfig {
            max_entries: 1,
        };

        assert!(config.validate().is_ok());
    }

    // --------------------------------------------------------
    // Shutdown configuration
    // --------------------------------------------------------

    #[test]
    fn rejects_zero_shutdown_timeout() {
        let config = ShutdownConfig {
            timeout: Duration::ZERO,
        };

        assert!(config.validate().is_err());
    }

    // --------------------------------------------------------
    // Network configuration
    // --------------------------------------------------------

    #[test]
    fn default_network_configuration_is_valid() {
        assert!(NetworkConfig::default().validate().is_ok());
    }

    #[test]
    fn rejects_blocking_upstream_udp() {
        let mut config = NetworkConfig::default();

        config.upstream_udp.socket.nonblocking = false;

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_blocking_upstream_tcp() {
        let mut config = NetworkConfig::default();

        config.upstream_tcp.socket.nonblocking = false;

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_blocking_downstream_tcp() {
        let mut config = NetworkConfig::default();

        config.downstream_tcp.socket.nonblocking = false;

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_upstream_framing() {
        let mut config = NetworkConfig::default();

        config.upstream_framing.max_frame_size = 0;

        assert!(matches!(
            config.validate(),
            Err(ServerError::Transport(_)),
        ));
    }

    #[test]
    fn rejects_invalid_downstream_framing() {
        let mut config = NetworkConfig::default();

        config.downstream_framing.max_frame_size = 0;

        assert!(config.validate().is_err());
    }

    // --------------------------------------------------------
    // Component validation
    // --------------------------------------------------------

    #[test]
    fn propagates_sequencer_configuration_error() {
        let mut config = ServerConfig::default();

        config.sequencer.reorder_capacity = 3;

        assert!(matches!(
            config.validate(),
            Err(ServerError::Sequencer(_)),
        ));
    }

    #[test]
    fn propagates_book_configuration_error() {
        let mut config = ServerConfig::default();

        config.book.max_orders = 0;

        assert!(matches!(
            config.validate(),
            Err(ServerError::Book(_)),
        ));
    }

    #[test]
    fn propagates_recovery_configuration_error() {
        let mut config = ServerConfig::default();

        config.recovery.buffer_capacity = 0;

        assert!(matches!(
            config.validate(),
            Err(ServerError::Recovery(_)),
        ));
    }

    #[test]
    fn propagates_gateway_configuration_error() {
        let mut config = ServerConfig::default();

        config.gateway.heartbeat_interval_ns = 0;

        assert!(matches!(
            config.validate(),
            Err(ServerError::Gateway(_)),
        ));
    }

    #[test]
    fn propagates_publisher_configuration_error() {
        let mut config = ServerConfig::default();

        config.gateway.publisher.queue_capacity = 0;

        assert!(matches!(
            config.validate(),
            Err(ServerError::Gateway(_)),
        ));
    }

    #[test]
    fn propagates_processing_configuration_error() {
        let mut config = ServerConfig::default();

        config.processing.upstream_budget = 0;

        assert!(matches!(
            config.validate(),
            Err(ServerError::InvalidConfig(_)),
        ));
    }

    #[test]
    fn propagates_snapshot_configuration_error() {
        let mut config = ServerConfig::default();

        config.snapshot.max_entries = 0;

        assert!(matches!(
            config.validate(),
            Err(ServerError::InvalidConfig(_)),
        ));
    }

    #[test]
    fn propagates_shutdown_configuration_error() {
        let mut config = ServerConfig::default();

        config.shutdown.timeout = Duration::ZERO;

        assert!(matches!(
            config.validate(),
            Err(ServerError::InvalidConfig(_)),
        ));
    }
}
