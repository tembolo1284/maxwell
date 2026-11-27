//! Application-level error representation.
//!
//! Server errors preserve the underlying component error
//! rather than converting it into an allocated string.
//!
//! Expected operational conditions, such as publication
//! backpressure and recovery progress, should be handled
//! explicitly by the server processing pipeline.
//!
//! An error does not necessarily indicate that the entire
//! server must terminate.

use std::error::Error;
use std::fmt;
use std::io;

use maxwell_book::BookError;
use maxwell_core::CoreError;
use maxwell_gateway::GatewayError;
use maxwell_metrics::MetricsError;
use maxwell_protocol::ProtocolError;
use maxwell_publisher::PublisherError;
use maxwell_recovery::RecoveryError;
use maxwell_sequencer::SequencerError;
use maxwell_transport::TransportError;

// ============================================================
// Result alias
// ============================================================

/// Result type used throughout the Maxwell server application.
pub type ServerResult<T> = Result<T, ServerError>;

// ============================================================
// Server error
// ============================================================

/// Errors originating from server orchestration or one of
/// its foundational components.
///
/// Component errors retain their original representation,
/// allowing callers to inspect the underlying failure.
///
/// This type intentionally does not implement `Clone`, `Copy`,
/// `PartialEq`, or `Eq`, because transport and I/O errors do
/// not provide those guarantees.
#[derive(Debug)]
pub enum ServerError {
    // --------------------------------------------------------
    // Application errors
    // --------------------------------------------------------

    /// Invalid application-level configuration.
    InvalidConfig(&'static str),

    /// An operation is incompatible with the current
    /// server lifecycle or synchronization state.
    InvalidState(&'static str),

    /// A server-owned resource could not be allocated.
    AllocationFailed,

    /// An internal application invariant was violated.
    InvariantViolation(&'static str),

    /// A downstream snapshot exceeds its configured limit.
    SnapshotCapacityExceeded {
        limit: usize,
        required: usize,
    },

    /// The server cannot provide a consistent snapshot
    /// of its current market state.
    SnapshotUnavailable,

    /// The upstream source cannot provide the recovery
    /// mechanism required to restore synchronization.
    RecoveryUnavailable,

    /// The graceful shutdown deadline was exceeded.
    ShutdownTimeout,

    // --------------------------------------------------------
    // Component errors
    // --------------------------------------------------------

    /// Core domain or financial primitive error.
    Core(CoreError),

    /// Binary protocol encoding or decoding error.
    Protocol(ProtocolError),

    /// Upstream sequencing error.
    Sequencer(SequencerError),

    /// Order book reconstruction error.
    Book(BookError),

    /// Transport or framing error.
    Transport(TransportError),

    /// Upstream recovery error.
    Recovery(RecoveryError),

    /// Publication or subscriber management error.
    Publisher(PublisherError),

    /// Downstream gateway or session error.
    Gateway(GatewayError),

    /// Metrics registration or instrumentation error.
    Metrics(MetricsError),

    /// Application-level operating system I/O error.
    Io(io::Error),
}

// ============================================================
// Display
// ============================================================

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(reason) => {
                write!(f, "invalid server configuration: {reason}")
            }

            Self::InvalidState(reason) => {
                write!(f, "invalid server state: {reason}")
            }

            Self::AllocationFailed => {
                f.write_str("server resource allocation failed")
            }

            Self::InvariantViolation(reason) => {
                write!(f, "server invariant violated: {reason}")
            }

            Self::SnapshotCapacityExceeded { limit, required } => {
                write!(
                    f,
                    "snapshot capacity exceeded: \
                     required {required} entries, limit {limit}"
                )
            }

            Self::SnapshotUnavailable => {
                f.write_str("consistent snapshot is unavailable")
            }

            Self::RecoveryUnavailable => {
                f.write_str("required upstream recovery is unavailable")
            }

            Self::ShutdownTimeout => {
                f.write_str("server graceful shutdown timed out")
            }

            Self::Core(error) => {
                write!(f, "core error: {error}")
            }

            Self::Protocol(error) => {
                write!(f, "protocol error: {error}")
            }

            Self::Sequencer(error) => {
                write!(f, "sequencer error: {error}")
            }

            Self::Book(error) => {
                write!(f, "book error: {error}")
            }

            Self::Transport(error) => {
                write!(f, "transport error: {error}")
            }

            Self::Recovery(error) => {
                write!(f, "recovery error: {error}")
            }

            Self::Publisher(error) => {
                write!(f, "publisher error: {error}")
            }

            Self::Gateway(error) => {
                write!(f, "gateway error: {error}")
            }

            Self::Metrics(error) => {
                write!(f, "metrics error: {error}")
            }

            Self::Io(error) => {
                write!(f, "server I/O error: {error}")
            }
        }
    }
}

// ============================================================
// Standard error implementation
// ============================================================

impl Error for ServerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Core(error) => Some(error),
            Self::Protocol(error) => Some(error),
            Self::Sequencer(error) => Some(error),
            Self::Book(error) => Some(error),
            Self::Transport(error) => Some(error),
            Self::Recovery(error) => Some(error),
            Self::Publisher(error) => Some(error),
            Self::Gateway(error) => Some(error),
            Self::Metrics(error) => Some(error),
            Self::Io(error) => Some(error),

            Self::InvalidConfig(_)
            | Self::InvalidState(_)
            | Self::AllocationFailed
            | Self::InvariantViolation(_)
            | Self::SnapshotCapacityExceeded { .. }
            | Self::SnapshotUnavailable
            | Self::RecoveryUnavailable
            | Self::ShutdownTimeout => None,
        }
    }
}

// ============================================================
// Component error conversions
// ============================================================

/// Implements lossless conversion from a component error
/// into its corresponding server error variant.
///
/// This allows component operations to use the `?` operator
/// without manually mapping every error.
macro_rules! impl_server_error_from {
    ($error:ty, $variant:ident) => {
        impl From<$error> for ServerError {
            fn from(error: $error) -> Self {
                Self::$variant(error)
            }
        }
    };
}

impl_server_error_from!(CoreError, Core);
impl_server_error_from!(ProtocolError, Protocol);
impl_server_error_from!(SequencerError, Sequencer);
impl_server_error_from!(BookError, Book);
impl_server_error_from!(TransportError, Transport);
impl_server_error_from!(RecoveryError, Recovery);
impl_server_error_from!(PublisherError, Publisher);
impl_server_error_from!(GatewayError, Gateway);
impl_server_error_from!(MetricsError, Metrics);
impl_server_error_from!(io::Error, Io);

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --------------------------------------------------------
    // Application errors
    // --------------------------------------------------------

    #[test]
    fn invalid_configuration_display() {
        let error = ServerError::InvalidConfig(
            "upstream budget must be greater than zero",
        );

        assert_eq!(
            error.to_string(),
            "invalid server configuration: \
             upstream budget must be greater than zero",
        );
    }

    #[test]
    fn invalid_state_display() {
        let error = ServerError::InvalidState(
            "server is not running",
        );

        assert_eq!(
            error.to_string(),
            "invalid server state: server is not running",
        );
    }

    #[test]
    fn invariant_violation_display() {
        let error = ServerError::InvariantViolation(
            "pending publication was overwritten",
        );

        assert_eq!(
            error.to_string(),
            "server invariant violated: \
             pending publication was overwritten",
        );
    }

    #[test]
    fn allocation_failure_display() {
        let error = ServerError::AllocationFailed;

        assert_eq!(
            error.to_string(),
            "server resource allocation failed",
        );
    }

    #[test]
    fn snapshot_capacity_error_display() {
        let error = ServerError::SnapshotCapacityExceeded {
            limit: 100,
            required: 101,
        };

        assert_eq!(
            error.to_string(),
            "snapshot capacity exceeded: \
             required 101 entries, limit 100",
        );
    }

    #[test]
    fn snapshot_unavailable_display() {
        let error = ServerError::SnapshotUnavailable;

        assert_eq!(
            error.to_string(),
            "consistent snapshot is unavailable",
        );
    }

    #[test]
    fn recovery_unavailable_display() {
        let error = ServerError::RecoveryUnavailable;

        assert_eq!(
            error.to_string(),
            "required upstream recovery is unavailable",
        );
    }

    #[test]
    fn shutdown_timeout_display() {
        let error = ServerError::ShutdownTimeout;

        assert_eq!(
            error.to_string(),
            "server graceful shutdown timed out",
        );
    }

    #[test]
    fn application_errors_have_no_underlying_source() {
        let errors = [
            ServerError::InvalidConfig("invalid configuration"),
            ServerError::InvalidState("invalid state"),
            ServerError::AllocationFailed,
            ServerError::InvariantViolation("invalid invariant"),
            ServerError::SnapshotCapacityExceeded {
                limit: 10,
                required: 11,
            },
            ServerError::SnapshotUnavailable,
            ServerError::RecoveryUnavailable,
            ServerError::ShutdownTimeout,
        ];

        for error in errors {
            assert!(error.source().is_none());
        }
    }

    // --------------------------------------------------------
    // Component conversions
    // --------------------------------------------------------

    #[test]
    fn converts_protocol_error() {
        let error = ServerError::from(
            ProtocolError::InvalidMagic,
        );

        assert!(matches!(
            error,
            ServerError::Protocol(ProtocolError::InvalidMagic),
        ));
    }

    #[test]
    fn converts_sequencer_error() {
        let error = ServerError::from(
            SequencerError::SequenceExhausted,
        );

        assert!(matches!(
            error,
            ServerError::Sequencer(
                SequencerError::SequenceExhausted,
            ),
        ));
    }

    #[test]
    fn converts_book_error() {
        let error = ServerError::from(
            BookError::OrderCapacityExceeded,
        );

        assert!(matches!(
            error,
            ServerError::Book(
                BookError::OrderCapacityExceeded,
            ),
        ));
    }

    #[test]
    fn converts_transport_error() {
        let error = ServerError::from(
            TransportError::ConnectionClosed,
        );

        assert!(matches!(
            error,
            ServerError::Transport(
                TransportError::ConnectionClosed,
            ),
        ));
    }

    #[test]
    fn converts_recovery_error() {
        let error = ServerError::from(
            RecoveryError::SnapshotTooOld,
        );

        assert!(matches!(
            error,
            ServerError::Recovery(
                RecoveryError::SnapshotTooOld,
            ),
        ));
    }

    #[test]
    fn converts_publisher_error() {
        let error = ServerError::from(
            PublisherError::SequenceExhausted,
        );

        assert!(matches!(
            error,
            ServerError::Publisher(
                PublisherError::SequenceExhausted,
            ),
        ));
    }

    #[test]
    fn converts_gateway_error() {
        let error = ServerError::from(
            GatewayError::AllocationFailed,
        );

        assert!(matches!(
            error,
            ServerError::Gateway(
                GatewayError::AllocationFailed,
            ),
        ));
    }

    #[test]
    fn converts_metrics_error() {
        let error = ServerError::from(
            MetricsError::RegistryFull,
        );

        assert!(matches!(
            error,
            ServerError::Metrics(
                MetricsError::RegistryFull,
            ),
        ));
    }

    #[test]
    fn converts_io_error() {
        let error = ServerError::from(
            io::Error::from(io::ErrorKind::PermissionDenied),
        );

        match error {
            ServerError::Io(source) => {
                assert_eq!(
                    source.kind(),
                    io::ErrorKind::PermissionDenied,
                );
            }

            other => {
                panic!("unexpected error: {other}");
            }
        }
    }

    // --------------------------------------------------------
    // Error propagation
    // --------------------------------------------------------

    #[test]
    fn question_mark_propagates_component_error() {
        fn component_operation() -> Result<(), BookError> {
            Err(BookError::InconsistentBook)
        }

        fn server_operation() -> ServerResult<()> {
            component_operation()?;

            Ok(())
        }

        assert!(matches!(
            server_operation(),
            Err(ServerError::Book(
                BookError::InconsistentBook,
            )),
        ));
    }

    #[test]
    fn question_mark_propagates_io_error() {
        fn io_operation() -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::NotFound))
        }

        fn server_operation() -> ServerResult<()> {
            io_operation()?;

            Ok(())
        }

        match server_operation() {
            Err(ServerError::Io(error)) => {
                assert_eq!(
                    error.kind(),
                    io::ErrorKind::NotFound,
                );
            }

            other => {
                panic!("unexpected result: {other:?}");
            }
        }
    }

    // --------------------------------------------------------
    // Error source preservation
    // --------------------------------------------------------

    #[test]
    fn preserves_protocol_error_source() {
        let error = ServerError::from(
            ProtocolError::InvalidMagic,
        );

        let source = error
            .source()
            .expect("protocol error source must exist");

        assert_eq!(
            source.downcast_ref::<ProtocolError>(),
            Some(&ProtocolError::InvalidMagic),
        );
    }

    #[test]
    fn preserves_book_error_source() {
        let error = ServerError::from(
            BookError::RecoveryRequired,
        );

        let source = error
            .source()
            .expect("book error source must exist");

        assert_eq!(
            source.downcast_ref::<BookError>(),
            Some(&BookError::RecoveryRequired),
        );
    }

    #[test]
    fn preserves_nested_gateway_publisher_error() {
        let publisher_error =
            PublisherError::SequenceExhausted;

        let gateway_error =
            GatewayError::from(publisher_error);

        let error = ServerError::from(gateway_error);

        let gateway_source = error
            .source()
            .expect("gateway error source must exist");

        assert!(
            gateway_source
                .downcast_ref::<GatewayError>()
                .is_some()
        );

        let publisher_source = gateway_source
            .source()
            .expect("publisher error source must exist");

        assert_eq!(
            publisher_source.downcast_ref::<PublisherError>(),
            Some(&PublisherError::SequenceExhausted),
        );
    }

    #[test]
    fn preserves_nested_transport_io_error() {
        let io_error =
            io::Error::from(io::ErrorKind::ConnectionReset);

        let transport_error =
            TransportError::from(io_error);

        let error = ServerError::from(transport_error);

        let transport_source = error
            .source()
            .expect("transport error source must exist");

        assert!(
            transport_source
                .downcast_ref::<TransportError>()
                .is_some()
        );

        let io_source = transport_source
            .source()
            .expect("I/O error source must exist");

        let original = io_source
            .downcast_ref::<io::Error>()
            .expect("source must be an I/O error");

        assert_eq!(
            original.kind(),
            io::ErrorKind::ConnectionReset,
        );
    }

    #[test]
    fn component_display_preserves_original_message() {
        let component = ProtocolError::InvalidMagic;

        let expected = format!("protocol error: {component}");

        let error = ServerError::from(component);

        assert_eq!(error.to_string(), expected);
    }

    // --------------------------------------------------------
    // Thread-safety guarantees
    // --------------------------------------------------------

    #[test]
    fn server_error_is_send_sync_and_static() {
        fn assert_error_traits<T>()
        where
            T: Error + Send + Sync + 'static,
        {
        }

        assert_error_traits::<ServerError>();
    }
}
