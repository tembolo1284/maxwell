//! Nonblocking publication sink abstraction.

use crate::event::Publication;

/// A downstream destination for publications.
///
/// Implementations must be nonblocking.
///
/// A successful result means the complete publication has
/// been accepted by the sink.
///
/// An error means the publication was not accepted and may
/// safely be retried. Implementations must not return an error
/// after partially accepting a publication unless they retain
/// sufficient internal state to prevent duplicate delivery.
pub trait PublicationSink<I, P> {
    type Error;

    fn try_send(&mut self, publication: &Publication<I, P>) -> Result<(), Self::Error>;
}

/// Result of draining a subscriber queue into a sink.
///
/// A sink failure stops draining immediately. The failed
/// publication remains at the front of the subscriber queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainReport<E> {
    pub sent: usize,
    pub remaining: usize,
    pub error: Option<E>,
}

impl<E> DrainReport<E> {
    #[inline]
    pub fn is_blocked(&self) -> bool {
        self.error.is_some()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.remaining == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_drain_report() {
        let report = DrainReport::<()> {
            sent: 4,
            remaining: 0,
            error: None,
        };

        assert!(!report.is_blocked());
        assert!(report.is_empty());
    }

    #[test]
    fn blocked_drain_report() {
        let report = DrainReport {
            sent: 2,
            remaining: 3,
            error: Some("would block"),
        };

        assert!(report.is_blocked());
        assert!(!report.is_empty());
    }
}
