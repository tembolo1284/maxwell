//! Explicit recovery state machine.

use core::fmt;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryState {
    Uninitialized = 0,
    Live = 1,
    GapDetected = 2,
    AwaitingRetransmission = 3,
    AwaitingSnapshot = 4,
    Replaying = 5,
    Failed = 6,
}

impl RecoveryState {
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Live)
    }

    #[must_use]
    pub const fn is_recovering(self) -> bool {
        matches!(
            self,
            Self::GapDetected
                | Self::AwaitingRetransmission
                | Self::AwaitingSnapshot
                | Self::Replaying
        )
    }

    #[must_use]
    pub const fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    /// Only synchronized live books may be advertised
    /// as current market state.
    #[must_use]
    pub const fn is_publishable(self) -> bool {
        self.is_live()
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uninitialized => "UNINITIALIZED",
            Self::Live => "LIVE",
            Self::GapDetected => "GAP_DETECTED",
            Self::AwaitingRetransmission => "AWAITING_RETRANSMISSION",
            Self::AwaitingSnapshot => "AWAITING_SNAPSHOT",
            Self::Replaying => "REPLAYING",
            Self::Failed => "FAILED",
        }
    }
}

impl fmt::Display for RecoveryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryReason {
    Startup = 0,
    SequenceGap = 1,
    GapTooLarge = 2,
    BufferOverflow = 3,
    RetransmissionUnavailable = 4,
    RetransmissionTimeout = 5,
    SnapshotTimeout = 6,
    SessionChanged = 7,
    ConflictingEvent = 8,
    BookInconsistency = 9,
    ManualResynchronization = 10,
    SequenceExhausted = 11,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_live_is_publishable() {
        let states = [
            RecoveryState::Uninitialized,
            RecoveryState::Live,
            RecoveryState::GapDetected,
            RecoveryState::AwaitingRetransmission,
            RecoveryState::AwaitingSnapshot,
            RecoveryState::Replaying,
            RecoveryState::Failed,
        ];

        for state in states {
            assert_eq!(state.is_publishable(), state == RecoveryState::Live,);
        }
    }

    #[test]
    fn recovery_classification() {
        assert!(RecoveryState::Replaying.is_recovering());
        assert!(RecoveryState::AwaitingSnapshot.is_recovering());
        assert!(!RecoveryState::Failed.is_recovering());
    }
}
