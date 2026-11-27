//! Nonblocking downstream transport abstraction.

use crate::message::ServerMessage;
use crate::session::SessionId;

/// Downstream gateway message destination.
///
/// Implementations must preserve accepted message ordering
/// independently for each session.
///
/// Ok means the complete logical message has been accepted.
///
/// Err means the message has not been accepted and can be
/// retried without introducing duplicates.
///
/// Implementations handling partial TCP writes should retain
/// those bytes internally and report successful acceptance
/// once ownership of the complete message has transferred.
///
/// Implementations must also enforce their own memory bounds.
pub trait GatewaySink<I, P> {
    type Error;

    fn try_send(
        &mut self,
        session_id: SessionId,
        message: &ServerMessage<I, P>,
    ) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSink {
        accepted: usize,
        capacity: usize,
    }

    impl GatewaySink<u32, u64> for TestSink {
        type Error = ();

        fn try_send(
            &mut self,
            _session_id: SessionId,
            _message: &ServerMessage<u32, u64>,
        ) -> Result<(), Self::Error> {
            if self.accepted == self.capacity {
                return Err(());
            }

            self.accepted += 1;

            Ok(())
        }
    }

    #[test]
    fn sink_reports_explicit_backpressure() {
        let mut sink = TestSink {
            accepted: 0,
            capacity: 1,
        };

        let message = ServerMessage::Heartbeat {
            timestamp_ns: 1000,
            publisher_watermark: 10,
        };

        let id = SessionId::new(1);

        assert_eq!(sink.try_send(id, &message), Ok(()));
        assert_eq!(sink.try_send(id, &message), Err(()));
        assert_eq!(sink.accepted, 1);
    }
}
