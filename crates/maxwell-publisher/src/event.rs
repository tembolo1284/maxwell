//! Market data publication envelopes.
//!
//! Instrument and payload types are supplied by the caller.
//! This avoids duplicating protocol and fixed-point types
//! defined by other Maxwell crates.

/// Logical category of a published event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EventKind {
    BookUpdate = 0,
    Trade = 1,
    Quote = 2,
    Snapshot = 3,
    InstrumentStatus = 4,
    FeedStatus = 5,
    Heartbeat = 6,
}

impl EventKind {
    pub const COUNT: usize = 7;

    pub const ALL: [Self; Self::COUNT] = [
        Self::BookUpdate,
        Self::Trade,
        Self::Quote,
        Self::Snapshot,
        Self::InstrumentStatus,
        Self::FeedStatus,
        Self::Heartbeat,
    ];

    #[inline]
    pub const fn bit(self) -> u16 {
        1u16 << (self as u8)
    }

    #[inline]
    pub const fn is_control(self) -> bool {
        matches!(self, Self::FeedStatus | Self::Heartbeat)
    }
}

/// An immutable publication envelope.
///
/// `sequence` is assigned by the publisher and represents
/// publication order, not the original exchange sequence.
///
/// The original feed identifier, exchange sequence and other
/// protocol metadata may be retained in `payload`.
///
/// `instrument` is None for feed-wide or global events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Publication<I, P> {
    pub sequence: u64,
    pub instrument: Option<I>,
    pub kind: EventKind,
    pub timestamp_ns: u64,
    pub payload: P,
}

impl<I, P> Publication<I, P> {
    #[inline]
    pub const fn new(
        sequence: u64,
        instrument: Option<I>,
        kind: EventKind,
        timestamp_ns: u64,
        payload: P,
    ) -> Self {
        Self {
            sequence,
            instrument,
            kind,
            timestamp_ns,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_bits_are_unique() {
        let mut mask = 0u16;

        for kind in EventKind::ALL {
            assert_eq!(mask & kind.bit(), 0);
            mask |= kind.bit();
        }

        assert_eq!(mask.count_ones(), EventKind::COUNT as u32);
    }

    #[test]
    fn identifies_control_events() {
        assert!(EventKind::Heartbeat.is_control());
        assert!(EventKind::FeedStatus.is_control());
        assert!(!EventKind::Trade.is_control());
        assert!(!EventKind::BookUpdate.is_control());
    }

    #[test]
    fn constructs_publication() {
        let publication = Publication::new(10, Some(42u32), EventKind::Trade, 1_000_000, 100i64);

        assert_eq!(publication.sequence, 10);
        assert_eq!(publication.instrument, Some(42));
        assert_eq!(publication.kind, EventKind::Trade);
        assert_eq!(publication.timestamp_ns, 1_000_000);
        assert_eq!(publication.payload, 100);
    }
}
