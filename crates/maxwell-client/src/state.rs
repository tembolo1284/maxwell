//! Client lifecycle and recovery state.

/// Lifecycle of a single downstream client connection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    #[default]
    Disconnected,

    /// Transport established, awaiting a snapshot.
    AwaitingSnapshot,

    /// A bounded snapshot transaction is in progress.
    ReceivingSnapshot,

    /// A valid baseline has been installed.
    Active,

    /// Market data state is invalid and requires recovery.
    RecoveryRequired,

    /// Server activity exceeded the configured timeout.
    TimedOut,
}

impl ClientState {
    #[inline]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    #[inline]
    pub const fn is_connected(self) -> bool {
        !matches!(self, Self::Disconnected | Self::TimedOut)
    }

    #[inline]
    pub const fn requires_recovery(self) -> bool {
        matches!(self, Self::RecoveryRequired)
    }
}

/// Reason the client invalidated its market data baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReason {
    TransportGap { expected: u64, received: u64 },

    ServerRequested { publisher_watermark: u64 },

    InvalidSnapshot,

    PublicationRegression { previous: u64, received: u64 },

    HandlerFailure,

    UnexpectedMessage,

    TransportSequenceExhausted,

    ServerTimeout,

    ExplicitRequest,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_disconnected() {
        assert_eq!(ClientState::default(), ClientState::Disconnected);
    }

    #[test]
    fn identifies_active_state() {
        assert!(ClientState::Active.is_active());
        assert!(!ClientState::ReceivingSnapshot.is_active());
    }

    #[test]
    fn identifies_connected_states() {
        assert!(ClientState::AwaitingSnapshot.is_connected());
        assert!(ClientState::RecoveryRequired.is_connected());
        assert!(!ClientState::Disconnected.is_connected());
        assert!(!ClientState::TimedOut.is_connected());
    }

    #[test]
    fn identifies_recovery_state() {
        assert!(ClientState::RecoveryRequired.requires_recovery());
        assert!(!ClientState::Active.requires_recovery());
    }
}
