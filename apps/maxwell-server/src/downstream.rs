//! Downstream logical messages and bounded connection queues.
//!
//! Connection-local transport sequences are independent of
//! global publication sequences.
//!
//! Every accepted logical message consumes exactly one
//! connection-local sequence number.
//!
//! Rejected messages do not advance the sequence cursor.

use std::collections::VecDeque;

use maxwell_core::{EventEnvelope, InstrumentId};
use maxwell_gateway::{GatewaySink, ServerMessage, SessionId};

use crate::error::{ServerError, ServerResult};
use crate::snapshot::{MarketSnapshot, SnapshotEntry};

// ============================================================
// Logical downstream messages
// ============================================================

pub type MarketMessage = ServerMessage<InstrumentId, EventEnvelope>;

/// Application-level downstream message.
///
/// This is a logical representation, not a binary wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownstreamMessage {
    Gateway(MarketMessage),

    SnapshotBegin {
        publisher_watermark: u64,
        entry_count: usize,
    },

    SnapshotEntry(SnapshotEntry),

    SnapshotEnd {
        publisher_watermark: u64,
    },
}

/// A connection-local sequenced message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownstreamFrame {
    pub transport_sequence: u64,
    pub message: DownstreamMessage,
}

// ============================================================
// Delivery errors
// ============================================================

/// Expected failures when transferring ownership of messages
/// to a downstream connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryError {
    Backpressure,
    SequenceExhausted,
    SessionMismatch,
    ConnectionClosed,
}

// ============================================================
// Snapshot delivery contract
// ============================================================

/// Ordered destination supporting complete snapshot acceptance.
///
/// Successful acceptance transfers ownership of the entire
/// snapshot transaction to the destination.
///
/// Rejection must leave the destination unchanged.
///
/// Previously accepted messages must remain ordered before
/// the snapshot, and subsequent incrementals must follow it.
pub trait SnapshotSink: GatewaySink<InstrumentId, EventEnvelope> {
    fn try_send_snapshot(
        &mut self,
        session_id: SessionId,
        snapshot: &MarketSnapshot,
    ) -> Result<(), Self::Error>;
}

// ============================================================
// Connection queue
// ============================================================

/// Bounded, single-owner outbound queue for one connection.
///
/// Intended initially for deterministic in-memory integration
/// testing. No socket operations occur here.
pub struct ConnectionQueue {
    session_id: SessionId,
    frames: VecDeque<DownstreamFrame>,
    capacity: usize,
    next_sequence: Option<u64>,
    closed: bool,
}

impl ConnectionQueue {
    /// Constructs a preallocated connection queue.
    pub fn new(session_id: SessionId, capacity: usize) -> ServerResult<Self> {
        if capacity == 0 {
            return Err(ServerError::InvalidConfig(
                "connection queue capacity must be positive",
            ));
        }

        let mut frames = VecDeque::new();

        frames
            .try_reserve_exact(capacity)
            .map_err(|_| ServerError::AllocationFailed)?;

        Ok(Self {
            session_id,
            frames,
            capacity,
            next_sequence: Some(1),
            closed: false,
        })
    }

    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    #[must_use]
    pub fn remaining_capacity(&self) -> usize {
        self.capacity - self.frames.len()
    }

    #[must_use]
    pub const fn next_sequence(&self) -> Option<u64> {
        self.next_sequence
    }

    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.closed
    }

    /// Permanently closes the connection.
    ///
    /// Undelivered frames are discarded. Reconnection
    /// requires a new queue and sequence domain.
    pub fn close(&mut self) {
        self.closed = true;
        self.frames.clear();
    }

    /// Inspects the next accepted frame.
    #[must_use]
    pub fn front(&self) -> Option<&DownstreamFrame> {
        self.frames.front()
    }

    /// Removes the next accepted frame.
    ///
    /// A real transport must retain ownership of this frame
    /// until delivery completes or the connection terminates.
    pub fn pop_front(&mut self) -> Option<DownstreamFrame> {
        self.frames.pop_front()
    }

    // --------------------------------------------------------
    // Acceptance validation
    // --------------------------------------------------------

    fn preflight(&self, session_id: SessionId, count: usize) -> Result<(), DeliveryError> {
        if session_id != self.session_id {
            return Err(DeliveryError::SessionMismatch);
        }

        if self.closed {
            return Err(DeliveryError::ConnectionClosed);
        }

        if count > self.remaining_capacity() {
            return Err(DeliveryError::Backpressure);
        }

        if count == 0 {
            return Ok(());
        }

        let first = self.next_sequence.ok_or(DeliveryError::SequenceExhausted)?;

        let offset = u64::try_from(count - 1).map_err(|_| DeliveryError::SequenceExhausted)?;

        first
            .checked_add(offset)
            .ok_or(DeliveryError::SequenceExhausted)?;

        Ok(())
    }

    /// Appends a message after successful preflight.
    ///
    /// This method is private because callers must reserve
    /// sufficient logical capacity and sequence space first.
    fn append_preflighted(&mut self, message: DownstreamMessage) {
        let sequence = self
            .next_sequence
            .expect("sequence must be available after preflight");

        self.frames.push_back(DownstreamFrame {
            transport_sequence: sequence,
            message,
        });

        self.next_sequence = sequence.checked_add(1);
    }

    /// Attempts to accept one logical message.
    pub fn try_accept(
        &mut self,
        session_id: SessionId,
        message: DownstreamMessage,
    ) -> Result<(), DeliveryError> {
        self.preflight(session_id, 1)?;

        self.append_preflighted(message);

        Ok(())
    }
}

// ============================================================
// Gateway integration
// ============================================================

impl GatewaySink<InstrumentId, EventEnvelope> for ConnectionQueue {
    type Error = DeliveryError;

    fn try_send(
        &mut self,
        session_id: SessionId,
        message: &MarketMessage,
    ) -> Result<(), Self::Error> {
        self.try_accept(session_id, DownstreamMessage::Gateway(*message))
    }
}

// ============================================================
// Snapshot integration
// ============================================================

impl SnapshotSink for ConnectionQueue {
    fn try_send_snapshot(
        &mut self,
        session_id: SessionId,
        snapshot: &MarketSnapshot,
    ) -> Result<(), Self::Error> {
        let count = snapshot
            .entries()
            .len()
            .checked_add(2)
            .ok_or(DeliveryError::Backpressure)?;

        // Validate the entire transaction before modifying
        // the queue or advancing its sequence cursor.
        self.preflight(session_id, count)?;

        let watermark = snapshot.publisher_watermark();

        self.append_preflighted(DownstreamMessage::SnapshotBegin {
            publisher_watermark: watermark,
            entry_count: snapshot.entries().len(),
        });

        for entry in snapshot.entries() {
            self.append_preflighted(DownstreamMessage::SnapshotEntry(*entry));
        }

        self.append_preflighted(DownstreamMessage::SnapshotEnd {
            publisher_watermark: watermark,
        });

        Ok(())
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionId {
        SessionId::new(1)
    }

    fn heartbeat() -> MarketMessage {
        ServerMessage::Heartbeat {
            timestamp_ns: 100,
            publisher_watermark: 50,
        }
    }

    #[test]
    fn rejects_zero_capacity() {
        assert!(ConnectionQueue::new(session(), 0).is_err());
    }

    #[test]
    fn new_connection_starts_at_sequence_one() {
        let queue = ConnectionQueue::new(session(), 4).unwrap();

        assert_eq!(queue.next_sequence(), Some(1));
        assert!(queue.is_empty());
    }

    #[test]
    fn accepted_messages_receive_contiguous_sequences() {
        let mut queue = ConnectionQueue::new(session(), 4).unwrap();

        for _ in 0..4 {
            queue.try_send(session(), &heartbeat()).unwrap();
        }

        for expected in 1..=4 {
            let frame = queue.pop_front().unwrap();

            assert_eq!(frame.transport_sequence, expected);
        }

        assert_eq!(queue.next_sequence(), Some(5));
    }

    #[test]
    fn rejection_does_not_consume_sequence() {
        let mut queue = ConnectionQueue::new(session(), 1).unwrap();

        queue.try_send(session(), &heartbeat()).unwrap();

        assert_eq!(
            queue.try_send(session(), &heartbeat()),
            Err(DeliveryError::Backpressure),
        );

        assert_eq!(queue.next_sequence(), Some(2));
        assert_eq!(queue.len(), 1);

        queue.pop_front().unwrap();

        queue.try_send(session(), &heartbeat()).unwrap();

        assert_eq!(queue.pop_front().unwrap().transport_sequence, 2,);
    }

    #[test]
    fn rejects_wrong_session() {
        let mut queue = ConnectionQueue::new(session(), 4).unwrap();

        assert_eq!(
            queue.try_send(SessionId::new(2), &heartbeat(),),
            Err(DeliveryError::SessionMismatch),
        );

        assert!(queue.is_empty());
        assert_eq!(queue.next_sequence(), Some(1));
    }

    #[test]
    fn final_sequence_can_be_used_once() {
        let mut queue = ConnectionQueue::new(session(), 4).unwrap();

        queue.next_sequence = Some(u64::MAX);

        queue.try_send(session(), &heartbeat()).unwrap();

        assert_eq!(queue.next_sequence(), None);

        assert_eq!(
            queue.try_send(session(), &heartbeat()),
            Err(DeliveryError::SequenceExhausted),
        );
    }

    #[test]
    fn snapshot_sequence_overflow_is_rejected_atomically() {
        let mut queue = ConnectionQueue::new(session(), 4).unwrap();

        queue.next_sequence = Some(u64::MAX);

        assert_eq!(
            queue.preflight(session(), 2),
            Err(DeliveryError::SequenceExhausted),
        );

        assert!(queue.is_empty());
        assert_eq!(queue.next_sequence(), Some(u64::MAX));
    }

    #[test]
    fn closed_connection_rejects_delivery() {
        let mut queue = ConnectionQueue::new(session(), 4).unwrap();

        queue.try_send(session(), &heartbeat()).unwrap();

        queue.close();

        assert!(queue.is_closed());
        assert!(queue.is_empty());

        assert_eq!(
            queue.try_send(session(), &heartbeat()),
            Err(DeliveryError::ConnectionClosed),
        );
    }

    #[test]
    fn empty_snapshot_consumes_two_sequences() {
        let mut queue = ConnectionQueue::new(session(), 2).unwrap();

        let snapshot = MarketSnapshot::empty(42);

        queue.try_send_snapshot(session(), &snapshot).unwrap();

        assert_eq!(
            queue.pop_front().unwrap(),
            DownstreamFrame {
                transport_sequence: 1,
                message: DownstreamMessage::SnapshotBegin {
                    publisher_watermark: 42,
                    entry_count: 0,
                },
            },
        );

        assert_eq!(
            queue.pop_front().unwrap(),
            DownstreamFrame {
                transport_sequence: 2,
                message: DownstreamMessage::SnapshotEnd {
                    publisher_watermark: 42,
                },
            },
        );
    }

    #[test]
    fn rejected_snapshot_leaves_queue_unchanged() {
        let mut queue = ConnectionQueue::new(session(), 1).unwrap();

        let snapshot = MarketSnapshot::empty(42);

        assert_eq!(
            queue.try_send_snapshot(session(), &snapshot),
            Err(DeliveryError::Backpressure),
        );

        assert!(queue.is_empty());
        assert_eq!(queue.next_sequence(), Some(1));
    }

    #[test]
    fn snapshot_preserves_existing_delivery_order() {
        let mut queue = ConnectionQueue::new(session(), 4).unwrap();

        queue.try_send(session(), &heartbeat()).unwrap();

        queue
            .try_send_snapshot(session(), &MarketSnapshot::empty(42))
            .unwrap();

        queue.try_send(session(), &heartbeat()).unwrap();

        assert!(matches!(
            queue.pop_front().unwrap().message,
            DownstreamMessage::Gateway(_),
        ));

        assert!(matches!(
            queue.pop_front().unwrap().message,
            DownstreamMessage::SnapshotBegin { .. },
        ));

        assert!(matches!(
            queue.pop_front().unwrap().message,
            DownstreamMessage::SnapshotEnd { .. },
        ));

        assert!(matches!(
            queue.pop_front().unwrap().message,
            DownstreamMessage::Gateway(_),
        ));
    }
}
