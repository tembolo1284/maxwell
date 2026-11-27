//! Logical inbound client messages.

use maxwell_gateway::ServerMessage;

/// A single snapshot record.
///
/// The application defines the concrete record type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundMessage<I, P, S> {
    /// Existing gateway message.
    Gateway(ServerMessage<I, P>),

    /// Start a complete snapshot transaction.
    SnapshotBegin {
        publisher_watermark: u64,
        entry_count: usize,
    },

    /// One snapshot record.
    SnapshotEntry(S),

    /// Complete the snapshot transaction.
    SnapshotEnd { publisher_watermark: u64 },
}

impl<I, P, S> InboundMessage<I, P, S> {
    #[inline]
    pub const fn is_snapshot(&self) -> bool {
        matches!(
            self,
            Self::SnapshotBegin { .. } | Self::SnapshotEntry(_) | Self::SnapshotEnd { .. }
        )
    }
}

/// A decoded connection-local transport frame.
///
/// Every server-to-client message consumes exactly one
/// transport sequence, including control and snapshot
/// messages.
///
/// The first sequence of a new connection must be one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboundFrame<I, P, S> {
    pub transport_sequence: u64,
    pub message: InboundMessage<I, P, S>,
}

impl<I, P, S> InboundFrame<I, P, S> {
    #[inline]
    pub const fn new(transport_sequence: u64, message: InboundMessage<I, P, S>) -> Self {
        Self {
            transport_sequence,
            message,
        }
    }
}

impl<I, P, S> From<ServerMessage<I, P>> for InboundMessage<I, P, S> {
    fn from(message: ServerMessage<I, P>) -> Self {
        Self::Gateway(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_snapshot_begin() {
        let frame = InboundFrame::<u32, u64, u64>::new(
            1,
            InboundMessage::SnapshotBegin {
                publisher_watermark: 100,
                entry_count: 10,
            },
        );

        assert_eq!(frame.transport_sequence, 1);
        assert!(frame.message.is_snapshot());
    }

    #[test]
    fn converts_gateway_message() {
        let message = ServerMessage::<u32, u64>::Heartbeat {
            timestamp_ns: 1000,
            publisher_watermark: 10,
        };

        let inbound: InboundMessage<u32, u64, u64> = message.into();

        assert_eq!(inbound, InboundMessage::Gateway(message));
        assert!(!inbound.is_snapshot());
    }
}
