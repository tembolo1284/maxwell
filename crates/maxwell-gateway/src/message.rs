//! Logical gateway request and response messages.
//!
//! These are application-level representations, not wire
//! formats. Encoding and decoding belong to the protocol
//! and transport integration layers.

use maxwell_publisher::{Publication, SubscriptionFilter};

/// Requests received from an authenticated client session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientRequest<I> {
    Subscribe { filter: SubscriptionFilter<I> },

    Unsubscribe,

    Heartbeat,

    RequestResync,

    Disconnect,
}

/// Messages offered to a downstream transport.
///
/// The publication sequence is a global publisher watermark.
/// It is not a contiguous per-client transport sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMessage<I, P> {
    Incremental(Publication<I, P>),

    Heartbeat {
        timestamp_ns: u64,
        publisher_watermark: u64,
    },

    ResyncRequired {
        publisher_watermark: u64,
    },
}

impl<I, P> ServerMessage<I, P> {
    #[inline]
    pub const fn is_incremental(&self) -> bool {
        matches!(self, Self::Incremental(_))
    }

    #[inline]
    pub const fn is_control(&self) -> bool {
        !self.is_incremental()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxwell_publisher::EventKind;

    #[test]
    fn identifies_incremental_message() {
        let message = ServerMessage::Incremental(Publication::new(
            1,
            Some(100u32),
            EventKind::Trade,
            1000,
            42u64,
        ));

        assert!(message.is_incremental());
        assert!(!message.is_control());
    }

    #[test]
    fn identifies_heartbeat_message() {
        let message = ServerMessage::<u32, u64>::Heartbeat {
            timestamp_ns: 1000,
            publisher_watermark: 10,
        };

        assert!(message.is_control());
        assert!(!message.is_incremental());
    }

    #[test]
    fn identifies_resync_message() {
        let message = ServerMessage::<u32, u64>::ResyncRequired {
            publisher_watermark: 100,
        };

        assert!(message.is_control());
    }
}
