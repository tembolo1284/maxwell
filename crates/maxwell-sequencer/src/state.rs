
//! Explicit feed sequencing states.

use core::fmt;

/// Represents the synchronization state of a feed.
///
/// RecoveryRequired is sticky until the caller explicitly
/// installs a recovered sequence checkpoint or a new session.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SequencerState {
    Live = 0,
    GapDetected = 1,
    RecoveryRequired = 2,
    Exhausted = 3,
}

impl SequencerState {
    #[must_use]
    #[inline]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Live)
    }

    #[must_use]
    #[inline]
    pub const fn has_gap(self) -> bool {
        matches!(self, Self::GapDetected)
    }

    #[must_use]
    #[inline]
    pub const fn requires_recovery(self) -> bool {
        matches!(self, Self::RecoveryRequired)
    }

    #[must_use]
    #[inline]
    pub const fn is_exhausted(self) -> bool {
        matches!(self, Self::Exhausted)
    }

    #[must_use]
    #[inline]
    pub const fn accepts_events(self) -> bool {
        matches!(self, Self::Live | Self::GapDetected)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::GapDetected => "gap_detected",
            Self::RecoveryRequired => "recovery_required",
            Self::Exhausted => "exhausted",
        }
    }
}

impl fmt::Display for SequencerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

const _: () = {
    assert!(size_of::<SequencerState>() == 1);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_accepts_events() {
        assert!(SequencerState::Live.accepts_events());
    }

    #[test]
    fn gap_accepts_events() {
        assert!(SequencerState::GapDetected.accepts_events());
    }

    #[test]
    fn recovery_rejects_events() {
        assert!(
            !SequencerState::RecoveryRequired.accepts_events()
        );
    }

    #[test]
    fn exhaustion_rejects_events() {
        assert!(!SequencerState::Exhausted.accepts_events());
    }

    #[test]
    fn state_display() {
        assert_eq!(
            SequencerState::GapDetected.to_string(),
            "gap_detected",
        );
    }
}

