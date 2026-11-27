//! Transport-independent recovery requests.

use core::fmt;

use maxwell_core::{FeedId, SequenceNumber, SequenceRange};

use crate::state::RecoveryReason;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(u64);

impl RequestId {
    pub const FIRST: Self = Self(1);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetransmissionRequest {
    pub id: RequestId,
    pub feed: FeedId,
    pub range: SequenceRange,
    pub attempt: u32,
}

impl RetransmissionRequest {
    #[must_use]
    pub const fn contains(self, sequence: SequenceNumber) -> bool {
        self.range.contains(sequence)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotRequest {
    pub id: RequestId,
    pub feed: FeedId,

    /// Snapshot must include every event through at
    /// least this sequence, inclusive.
    pub minimum_watermark: SequenceNumber,

    pub reason: RecoveryReason,
    pub attempt: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryRequest {
    Retransmission(RetransmissionRequest),
    Snapshot(SnapshotRequest),
}

impl RecoveryRequest {
    #[must_use]
    pub const fn id(self) -> RequestId {
        match self {
            Self::Retransmission(request) => request.id,
            Self::Snapshot(request) => request.id,
        }
    }

    #[must_use]
    pub const fn feed(self) -> FeedId {
        match self {
            Self::Retransmission(request) => request.feed,
            Self::Snapshot(request) => request.feed,
        }
    }

    #[must_use]
    pub const fn attempt(self) -> u32 {
        match self {
            Self::Retransmission(request) => request.attempt,
            Self::Snapshot(request) => request.attempt,
        }
    }

    #[must_use]
    pub const fn is_snapshot(self) -> bool {
        matches!(self, Self::Snapshot(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_progression() {
        assert_eq!(RequestId::FIRST.next(), Some(RequestId::new(2)),);
    }

    #[test]
    fn request_ids_do_not_wrap() {
        assert_eq!(RequestId::new(u64::MAX).next(), None);
    }
}
