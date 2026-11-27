//! Application lifecycle and synchronization readiness.
//!
//! The server lifecycle is independent of the upstream
//! sequencer and recovery state machines.
//!
//! A running server is not necessarily synchronized.
//! Downstream snapshot availability depends on both
//! component synchronization and publication consistency.

use core::fmt;

use maxwell_recovery::RecoveryState;
use maxwell_sequencer::SequencerState;

use crate::error::{ServerError, ServerResult};

// ============================================================
// Server lifecycle
// ============================================================

/// Application-level server lifecycle.
///
/// This state describes the server process, not the
/// synchronization state of its upstream market data.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerState {
    /// Components have been initialized but processing
    /// has not started.
    #[default]
    Initialized,

    /// The server is accepting and processing work.
    Running,

    /// New upstream work is no longer accepted.
    /// Outstanding work may be drained.
    Draining,

    /// The server has completed shutdown.
    Stopped,

    /// A fatal application-level failure occurred.
    Failed,
}

impl ServerState {
    /// Returns whether new upstream work may be accepted.
    #[must_use]
    pub const fn accepts_upstream(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Returns whether existing downstream work may
    /// continue to be serviced.
    #[must_use]
    pub const fn services_downstream(self) -> bool {
        matches!(self, Self::Running | Self::Draining)
    }

    /// Returns whether the server is draining.
    #[must_use]
    pub const fn is_draining(self) -> bool {
        matches!(self, Self::Draining)
    }

    /// Returns whether the lifecycle is terminal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }

    /// Returns whether a lifecycle transition is permitted.
    ///
    /// Terminal states cannot be restarted. A new server
    /// instance must be constructed instead.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Initialized, Self::Running)
                | (Self::Initialized, Self::Stopped)
                | (Self::Initialized, Self::Failed)
                | (Self::Running, Self::Draining)
                | (Self::Running, Self::Failed)
                | (Self::Draining, Self::Stopped)
                | (Self::Draining, Self::Failed)
        )
    }

    /// Performs a validated lifecycle transition.
    ///
    /// An invalid transition leaves the current state unchanged.
    ///
    /// # Errors
    ///
    /// Returns `ServerError::InvalidState` if the requested
    /// transition is not permitted.
    pub fn transition_to(&mut self, next: Self) -> ServerResult<()> {
        if !self.can_transition_to(next) {
            return Err(ServerError::InvalidState(
                "invalid server lifecycle transition",
            ));
        }

        *self = next;

        Ok(())
    }

    /// Returns the stable textual representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initialized => "INITIALIZED",
            Self::Running => "RUNNING",
            Self::Draining => "DRAINING",
            Self::Stopped => "STOPPED",
            Self::Failed => "FAILED",
        }
    }
}

impl fmt::Display for ServerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================
// Synchronization readiness
// ============================================================

/// Point-in-time readiness information derived from the
/// authoritative processing components.
///
/// This value does not own or mutate component state.
/// The server must construct it from current component
/// observations whenever readiness is evaluated.
///
/// It must not be cached across processing iterations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerReadiness {
    /// Current application lifecycle.
    pub lifecycle: ServerState,

    /// Current upstream sequencing state.
    pub sequencer: SequencerState,

    /// Current upstream recovery state.
    pub recovery: RecoveryState,

    /// Whether an authoritative book baseline is installed
    /// and the reconstructed book remains valid.
    pub book_valid: bool,

    /// Whether a book mutation has an outstanding
    /// publication that has not yet been accepted.
    pub publication_pending: bool,
}

impl ServerReadiness {
    /// Constructs a point-in-time readiness observation.
    #[must_use]
    pub const fn new(
        lifecycle: ServerState,
        sequencer: SequencerState,
        recovery: RecoveryState,
        book_valid: bool,
        publication_pending: bool,
    ) -> Self {
        Self {
            lifecycle,
            sequencer,
            recovery,
            book_valid,
            publication_pending,
        }
    }

    /// Returns whether the reconstructed upstream market
    /// state is synchronized.
    ///
    /// Synchronization does not imply that the current
    /// publication watermark represents the entire book.
    #[must_use]
    pub const fn is_synchronized(self) -> bool {
        self.sequencer.is_live()
            && self.recovery.is_publishable()
            && self.book_valid
    }

    /// Returns whether the server may advertise its
    /// reconstructed market state as current.
    ///
    /// An outstanding publication prevents the book and
    /// publication watermark from being advertised as
    /// a consistent current state.
    #[must_use]
    pub const fn can_advertise_current(self) -> bool {
        self.lifecycle.accepts_upstream()
            && self.is_synchronized()
            && !self.publication_pending
    }

    /// Returns whether a consistent downstream snapshot
    /// may be captured.
    ///
    /// Snapshot capture and subscriber activation must
    /// still be coordinated by the server owner.
    #[must_use]
    pub const fn can_capture_snapshot(self) -> bool {
        self.can_advertise_current()
    }

    /// Returns whether the server may begin processing
    /// another normal live upstream event.
    ///
    /// Recovery ingestion and replay use separate
    /// application-level processing paths.
    #[must_use]
    pub const fn can_process_live_event(self) -> bool {
        self.can_advertise_current()
    }

    /// Returns whether an outstanding publication may
    /// be retried during normal operation or shutdown.
    ///
    /// This is a readiness check, not authorization to
    /// publish an event from an invalidated stream.
    #[must_use]
    pub const fn can_retry_publication(self) -> bool {
        self.lifecycle.services_downstream()
            && self.is_synchronized()
            && self.publication_pending
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [ServerState; 5] = [
        ServerState::Initialized,
        ServerState::Running,
        ServerState::Draining,
        ServerState::Stopped,
        ServerState::Failed,
    ];

    fn live_readiness() -> ServerReadiness {
        ServerReadiness::new(
            ServerState::Running,
            SequencerState::Live,
            RecoveryState::Live,
            true,
            false,
        )
    }

    // --------------------------------------------------------
    // Lifecycle
    // --------------------------------------------------------

    #[test]
    fn default_state_is_initialized() {
        assert_eq!(
            ServerState::default(),
            ServerState::Initialized,
        );
    }

    #[test]
    fn normal_lifecycle() {
        let mut state = ServerState::default();

        state.transition_to(ServerState::Running).unwrap();
        state.transition_to(ServerState::Draining).unwrap();
        state.transition_to(ServerState::Stopped).unwrap();

        assert!(state.is_terminal());
    }

    #[test]
    fn initialized_server_can_stop_without_starting() {
        let mut state = ServerState::Initialized;

        state.transition_to(ServerState::Stopped).unwrap();

        assert!(state.is_terminal());
    }

    #[test]
    fn invalid_transition_preserves_state() {
        let mut state = ServerState::Initialized;

        assert!(matches!(
            state.transition_to(ServerState::Draining),
            Err(ServerError::InvalidState(_)),
        ));

        assert_eq!(state, ServerState::Initialized);
    }

    #[test]
    fn terminal_states_cannot_transition() {
        for terminal in [
            ServerState::Stopped,
            ServerState::Failed,
        ] {
            for next in STATES {
                assert!(!terminal.can_transition_to(next));
            }
        }
    }

    #[test]
    fn active_states_can_fail() {
        for mut state in [
            ServerState::Initialized,
            ServerState::Running,
            ServerState::Draining,
        ] {
            state.transition_to(ServerState::Failed).unwrap();

            assert!(state.is_terminal());
        }
    }

    #[test]
    fn running_server_cannot_skip_draining() {
        let mut state = ServerState::Running;

        assert!(
            state.transition_to(ServerState::Stopped).is_err()
        );

        assert_eq!(state, ServerState::Running);
    }

    #[test]
    fn self_transitions_are_rejected() {
        for mut state in STATES {
            let current = state;

            assert!(state.transition_to(current).is_err());
            assert_eq!(state, current);
        }
    }

    #[test]
    fn only_running_accepts_upstream() {
        for state in STATES {
            assert_eq!(
                state.accepts_upstream(),
                state == ServerState::Running,
            );
        }
    }

    #[test]
    fn downstream_service_continues_during_draining() {
        for state in STATES {
            assert_eq!(
                state.services_downstream(),
                matches!(
                    state,
                    ServerState::Running | ServerState::Draining
                ),
            );
        }
    }

    #[test]
    fn lifecycle_display() {
        assert_eq!(
            ServerState::Initialized.to_string(),
            "INITIALIZED",
        );

        assert_eq!(ServerState::Running.to_string(), "RUNNING");
        assert_eq!(ServerState::Draining.to_string(), "DRAINING");
        assert_eq!(ServerState::Stopped.to_string(), "STOPPED");
        assert_eq!(ServerState::Failed.to_string(), "FAILED");
    }

    // --------------------------------------------------------
    // Synchronization readiness
    // --------------------------------------------------------

    #[test]
    fn synchronized_server_is_ready() {
        let readiness = live_readiness();

        assert!(readiness.is_synchronized());
        assert!(readiness.can_advertise_current());
        assert!(readiness.can_capture_snapshot());
        assert!(readiness.can_process_live_event());
        assert!(!readiness.can_retry_publication());
    }

    #[test]
    fn running_does_not_imply_synchronized() {
        let readiness = ServerReadiness {
            recovery: RecoveryState::Uninitialized,
            book_valid: false,
            ..live_readiness()
        };

        assert!(readiness.lifecycle.accepts_upstream());
        assert!(!readiness.is_synchronized());
        assert!(!readiness.can_advertise_current());
    }

    #[test]
    fn non_live_sequencer_prevents_readiness() {
        for sequencer in [
            SequencerState::GapDetected,
            SequencerState::RecoveryRequired,
            SequencerState::Exhausted,
        ] {
            let readiness = ServerReadiness {
                sequencer,
                ..live_readiness()
            };

            assert!(!readiness.is_synchronized());
            assert!(!readiness.can_capture_snapshot());
            assert!(!readiness.can_process_live_event());
        }
    }

    #[test]
    fn non_live_recovery_prevents_readiness() {
        for recovery in [
            RecoveryState::Uninitialized,
            RecoveryState::GapDetected,
            RecoveryState::AwaitingRetransmission,
            RecoveryState::AwaitingSnapshot,
            RecoveryState::Replaying,
            RecoveryState::Failed,
        ] {
            let readiness = ServerReadiness {
                recovery,
                ..live_readiness()
            };

            assert!(!readiness.is_synchronized());
            assert!(!readiness.can_advertise_current());
            assert!(!readiness.can_capture_snapshot());
        }
    }

    #[test]
    fn invalid_book_prevents_readiness() {
        let readiness = ServerReadiness {
            book_valid: false,
            ..live_readiness()
        };

        assert!(!readiness.is_synchronized());
        assert!(!readiness.can_capture_snapshot());
        assert!(!readiness.can_process_live_event());
    }

    // --------------------------------------------------------
    // Publication backpressure
    // --------------------------------------------------------

    #[test]
    fn pending_publication_blocks_new_live_events() {
        let readiness = ServerReadiness {
            publication_pending: true,
            ..live_readiness()
        };

        assert!(readiness.is_synchronized());
        assert!(!readiness.can_advertise_current());
        assert!(!readiness.can_capture_snapshot());
        assert!(!readiness.can_process_live_event());
        assert!(readiness.can_retry_publication());
    }

    #[test]
    fn accepted_publication_restores_readiness() {
        let mut readiness = ServerReadiness {
            publication_pending: true,
            ..live_readiness()
        };

        assert!(!readiness.can_capture_snapshot());

        readiness.publication_pending = false;

        assert!(readiness.can_capture_snapshot());
        assert!(readiness.can_process_live_event());
    }

    #[test]
    fn invalidated_book_prevents_publication_retry() {
        let readiness = ServerReadiness {
            publication_pending: true,
            book_valid: false,
            ..live_readiness()
        };

        assert!(!readiness.can_retry_publication());
    }

    #[test]
    fn recovery_prevents_live_publication_retry() {
        let readiness = ServerReadiness {
            publication_pending: true,
            recovery: RecoveryState::Replaying,
            ..live_readiness()
        };

        assert!(!readiness.can_retry_publication());
    }

    // --------------------------------------------------------
    // Shutdown readiness
    // --------------------------------------------------------

    #[test]
    fn draining_prevents_new_snapshots_and_live_events() {
        let readiness = ServerReadiness {
            lifecycle: ServerState::Draining,
            ..live_readiness()
        };

        assert!(readiness.is_synchronized());
        assert!(!readiness.can_capture_snapshot());
        assert!(!readiness.can_process_live_event());
        assert!(!readiness.can_advertise_current());
    }

    #[test]
    fn draining_allows_pending_publication_retry() {
        let readiness = ServerReadiness {
            lifecycle: ServerState::Draining,
            publication_pending: true,
            ..live_readiness()
        };

        assert!(readiness.can_retry_publication());
        assert!(!readiness.can_process_live_event());
    }

    #[test]
    fn inactive_server_cannot_retry_publication() {
        for lifecycle in [
            ServerState::Initialized,
            ServerState::Stopped,
            ServerState::Failed,
        ] {
            let readiness = ServerReadiness {
                lifecycle,
                publication_pending: true,
                ..live_readiness()
            };

            assert!(!readiness.can_retry_publication());
            assert!(!readiness.can_capture_snapshot());
        }
    }
}
