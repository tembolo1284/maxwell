//! Application-facing market data handler.

use maxwell_publisher::Publication;

use crate::state::RecoveryReason;

/// Receives validated market data.
///
/// Implementations must enforce their own memory bounds.
///
/// Snapshot installation must be transactional:
///
/// - Ok means the complete baseline was installed.
/// - Err means no partially installed baseline is usable.
///
/// Incremental processing must likewise return Ok only
/// after the event has been applied successfully.
///
/// The client invalidates the baseline after a handler
/// failure and will not apply further increments until
/// another snapshot has been installed.
pub trait MarketDataHandler<I, P, S> {
    type Error;

    /// Replace the current market state with a complete
    /// validated snapshot.
    fn install_snapshot(
        &mut self,
        publisher_watermark: u64,
        entries: &[S],
    ) -> Result<(), Self::Error>;

    /// Apply one validated incremental publication.
    fn apply_incremental(&mut self, publication: &Publication<I, P>) -> Result<(), Self::Error>;

    /// Invalidate application-visible market state.
    ///
    /// This callback must not fail.
    fn on_invalidated(&mut self, reason: RecoveryReason);

    /// Invalidate state associated with a closed connection.
    fn on_disconnected(&mut self);
}
