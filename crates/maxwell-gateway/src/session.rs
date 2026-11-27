//! Client session identity and lifecycle.

use std::fmt;

use maxwell_publisher::SubscriberId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SessionId(u64);

impl SessionId {
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn subscriber_id(self) -> SubscriberId {
        SubscriberId::new(self.0)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Session lifecycle.
///
/// Connected:
///     Transport established, no subscription registered.
///
/// AwaitingSnapshot:
///     Subscription registered, awaiting a valid baseline.
///
/// Active:
///     Eligible for incremental market data delivery.
///
/// ResyncRequired:
///     Incremental state invalidated.
///
/// Expired:
///     Session timed out and must be disconnected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Connected,
    AwaitingSnapshot,
    Active,
    ResyncRequired,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    id: SessionId,
    state: SessionState,
    connected_at_ns: u64,
    last_rx_ns: u64,
    last_tx_ns: u64,
    resync_notified: bool,
}

impl Session {
    pub(crate) fn new(id: SessionId, now_ns: u64) -> Self {
        Self {
            id,
            state: SessionState::Connected,
            connected_at_ns: now_ns,
            last_rx_ns: now_ns,
            last_tx_ns: now_ns,
            resync_notified: false,
        }
    }

    #[inline]
    pub fn id(&self) -> SessionId {
        self.id
    }

    #[inline]
    pub fn state(&self) -> SessionState {
        self.state
    }

    #[inline]
    pub fn connected_at_ns(&self) -> u64 {
        self.connected_at_ns
    }

    #[inline]
    pub fn last_rx_ns(&self) -> u64 {
        self.last_rx_ns
    }

    #[inline]
    pub fn last_tx_ns(&self) -> u64 {
        self.last_tx_ns
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.state == SessionState::Active
    }

    #[inline]
    pub fn is_expired(&self) -> bool {
        self.state == SessionState::Expired
    }

    #[inline]
    pub fn resync_notified(&self) -> bool {
        self.resync_notified
    }

    #[inline]
    pub fn has_subscription(&self) -> bool {
        matches!(
            self.state,
            SessionState::AwaitingSnapshot | SessionState::Active | SessionState::ResyncRequired
        )
    }

    #[inline]
    pub fn heartbeat_due(&self, now_ns: u64, interval_ns: u64) -> bool {
        now_ns.saturating_sub(self.last_tx_ns) >= interval_ns
    }

    #[inline]
    pub fn timeout_due(&self, now_ns: u64, timeout_ns: u64) -> bool {
        now_ns.saturating_sub(self.last_rx_ns) >= timeout_ns
    }

    #[inline]
    pub(crate) fn record_rx(&mut self, now_ns: u64) {
        self.last_rx_ns = now_ns;
    }

    #[inline]
    pub(crate) fn record_tx(&mut self, now_ns: u64) {
        self.last_tx_ns = now_ns;
    }

    pub(crate) fn set_state(&mut self, state: SessionState) {
        self.state = state;
        self.resync_notified = false;
    }

    pub(crate) fn mark_resync_notified(&mut self) {
        self.resync_notified = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_is_connected() {
        let session = Session::new(SessionId::new(1), 100);

        assert_eq!(session.state(), SessionState::Connected);
        assert_eq!(session.connected_at_ns(), 100);
        assert_eq!(session.last_rx_ns(), 100);
        assert_eq!(session.last_tx_ns(), 100);
    }

    #[test]
    fn maps_session_to_subscriber() {
        let id = SessionId::new(42);

        assert_eq!(id.subscriber_id(), SubscriberId::new(42));
    }

    #[test]
    fn detects_heartbeat_boundary() {
        let session = Session::new(SessionId::new(1), 100);

        assert!(!session.heartbeat_due(149, 50));
        assert!(session.heartbeat_due(150, 50));
    }

    #[test]
    fn detects_timeout_boundary() {
        let session = Session::new(SessionId::new(1), 100);

        assert!(!session.timeout_due(199, 100));
        assert!(session.timeout_due(200, 100));
    }

    #[test]
    fn receive_activity_extends_timeout() {
        let mut session = Session::new(SessionId::new(1), 100);

        session.record_rx(150);

        assert!(!session.timeout_due(200, 100));
        assert!(session.timeout_due(250, 100));
    }

    #[test]
    fn transmission_resets_heartbeat_timer() {
        let mut session = Session::new(SessionId::new(1), 100);

        session.record_tx(140);

        assert!(!session.heartbeat_due(150, 50));
        assert!(session.heartbeat_due(190, 50));
    }

    #[test]
    fn tracks_resync_notification() {
        let mut session = Session::new(SessionId::new(1), 100);

        session.set_state(SessionState::ResyncRequired);

        assert!(!session.resync_notified());

        session.mark_resync_notified();

        assert!(session.resync_notified());

        session.set_state(SessionState::Active);

        assert!(!session.resync_notified());
    }
}
