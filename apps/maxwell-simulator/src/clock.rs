//! Deterministic simulation timestamps and real-time pacing.

use std::thread;
use std::time::{Duration, Instant};

use maxwell_core::{ExchangeTimestamp, MonotonicTimestamp};

use crate::config::{
    NANOS_PER_SECOND, PacingConfig, PacingMode,
};
use crate::error::{SimulatorError, SimulatorResult};

// ============================================================
// Virtual clock
// ============================================================

/// Deterministic clock independent of operating system time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualClock {
    current_ns: u64,
    interval_ns: u64,
    ticks: u64,
}

impl VirtualClock {
    pub fn new(
        start_ns: u64,
        interval_ns: u64,
    ) -> SimulatorResult<Self> {
        if interval_ns == 0 {
            return Err(SimulatorError::InvalidConfig(
                "virtual clock interval must be positive",
            ));
        }

        Ok(Self {
            current_ns: start_ns,
            interval_ns,
            ticks: 0,
        })
    }

    #[must_use]
    pub const fn current_ns(&self) -> u64 {
        self.current_ns
    }

    #[must_use]
    pub const fn interval_ns(&self) -> u64 {
        self.interval_ns
    }

    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    #[must_use]
    pub const fn monotonic_timestamp(
        &self,
    ) -> MonotonicTimestamp {
        MonotonicTimestamp::new(self.current_ns)
    }

    #[must_use]
    pub const fn exchange_timestamp(
        &self,
    ) -> ExchangeTimestamp {
        ExchangeTimestamp::new(self.current_ns)
    }

    /// Advances the clock by one configured interval.
    pub fn advance(&mut self) -> SimulatorResult<u64> {
        self.advance_by(1)
    }

    /// Advances by multiple intervals atomically.
    ///
    /// On arithmetic failure, the clock remains unchanged.
    pub fn advance_by(
        &mut self,
        count: u64,
    ) -> SimulatorResult<u64> {
        let elapsed = self
            .interval_ns
            .checked_mul(count)
            .ok_or(SimulatorError::TimestampOverflow)?;

        let next = self
            .current_ns
            .checked_add(elapsed)
            .ok_or(SimulatorError::TimestampOverflow)?;

        let ticks = self
            .ticks
            .checked_add(count)
            .ok_or(SimulatorError::ArithmeticOverflow(
                "virtual clock tick count",
            ))?;

        self.current_ns = next;
        self.ticks = ticks;

        Ok(next)
    }
}

// ============================================================
// Real-time pacing
// ============================================================

/// Paces event generation using absolute monotonic deadlines.
///
/// Deadline calculations use integer arithmetic to avoid
/// accumulated rounding error at nonintegral event intervals.
#[derive(Debug)]
pub struct Pacer {
    config: PacingConfig,
    started_at: Instant,
}

impl Pacer {
    pub fn new(
        config: PacingConfig,
    ) -> SimulatorResult<Self> {
        config.validate()?;

        Ok(Self {
            config,
            started_at: Instant::now(),
        })
    }

    #[must_use]
    pub const fn config(&self) -> PacingConfig {
        self.config
    }

    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Restarts the wall-clock pacing origin.
    pub fn reset(&mut self) {
        self.started_at = Instant::now();
    }

    /// Returns the target elapsed duration for an event index.
    ///
    /// Event index zero is scheduled immediately.
    pub fn target_elapsed(
        &self,
        event_index: u64,
    ) -> SimulatorResult<Duration> {
        let nanos = u128::from(event_index)
            * u128::from(NANOS_PER_SECOND)
            / u128::from(self.config.events_per_second);

        let seconds =
            nanos / u128::from(NANOS_PER_SECOND);

        let subsecond =
            nanos % u128::from(NANOS_PER_SECOND);

        let seconds = u64::try_from(seconds)
            .map_err(|_| SimulatorError::TimestampOverflow)?;

        Ok(Duration::new(seconds, subsecond as u32))
    }

    /// Waits until the specified event's absolute deadline.
    ///
    /// Virtual and unbounded modes never sleep.
    ///
    /// Real-time mode does not guarantee hard real-time
    /// scheduling or precise operating system wakeups.
    pub fn wait_for(
        &self,
        event_index: u64,
    ) -> SimulatorResult<()> {
        if self.config.mode != PacingMode::RealTime {
            return Ok(());
        }

        let target = self.target_elapsed(event_index)?;

        loop {
            let elapsed = self.started_at.elapsed();

            let Some(remaining) =
                target.checked_sub(elapsed)
            else {
                break;
            };

            if remaining.is_zero() {
                break;
            }

            thread::sleep(remaining);
        }

        Ok(())
    }

    /// Returns how far execution is behind the target.
    pub fn lateness(
        &self,
        event_index: u64,
    ) -> SimulatorResult<Duration> {
        let target = self.target_elapsed(event_index)?;

        Ok(self.elapsed().saturating_sub(target))
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_clock_preserves_initial_timestamp() {
        let clock = VirtualClock::new(100, 20).unwrap();

        assert_eq!(clock.current_ns(), 100);
        assert_eq!(clock.interval_ns(), 20);
        assert_eq!(clock.ticks(), 0);
    }

    #[test]
    fn virtual_clock_advances_deterministically() {
        let mut clock =
            VirtualClock::new(100, 20).unwrap();

        assert_eq!(clock.advance().unwrap(), 120);
        assert_eq!(clock.advance().unwrap(), 140);
        assert_eq!(clock.advance_by(3).unwrap(), 200);
        assert_eq!(clock.ticks(), 5);
    }

    #[test]
    fn zero_advance_preserves_clock() {
        let mut clock =
            VirtualClock::new(100, 20).unwrap();

        let before = clock;

        clock.advance_by(0).unwrap();

        assert_eq!(clock, before);
    }

    #[test]
    fn timestamp_overflow_is_atomic() {
        let mut clock =
            VirtualClock::new(u64::MAX - 1, 2).unwrap();

        let before = clock;

        assert!(matches!(
            clock.advance(),
            Err(SimulatorError::TimestampOverflow),
        ));

        assert_eq!(clock, before);
    }

    #[test]
    fn interval_multiplication_overflow_is_atomic() {
        let mut clock =
            VirtualClock::new(0, u64::MAX).unwrap();

        let before = clock;

        assert!(clock.advance_by(2).is_err());
        assert_eq!(clock, before);
    }

    #[test]
    fn zero_interval_is_rejected() {
        assert!(VirtualClock::new(0, 0).is_err());
    }

    #[test]
    fn timestamp_wrappers_preserve_raw_values() {
        let clock = VirtualClock::new(123, 1).unwrap();

        assert_eq!(
            clock.monotonic_timestamp().raw(),
            123,
        );

        assert_eq!(
            clock.exchange_timestamp().raw(),
            123,
        );
    }

    #[test]
    fn pacing_deadlines_do_not_accumulate_rounding_error() {
        let config = PacingConfig {
            events_per_second: 3,
            ..PacingConfig::default()
        };

        let pacer = Pacer::new(config).unwrap();

        assert_eq!(
            pacer.target_elapsed(1).unwrap(),
            Duration::from_nanos(333_333_333),
        );

        assert_eq!(
            pacer.target_elapsed(3).unwrap(),
            Duration::from_secs(1),
        );
    }

    #[test]
    fn default_rate_has_twenty_microsecond_interval() {
        let pacer =
            Pacer::new(PacingConfig::default()).unwrap();

        assert_eq!(
            pacer.target_elapsed(1).unwrap(),
            Duration::from_micros(20),
        );

        assert_eq!(
            pacer.target_elapsed(50_000).unwrap(),
            Duration::from_secs(1),
        );
    }

    #[test]
    fn virtual_pacing_does_not_wait() {
        let pacer =
            Pacer::new(PacingConfig::default()).unwrap();

        pacer.wait_for(u64::MAX).unwrap();
    }

    #[test]
    fn unbounded_pacing_does_not_wait() {
        let config = PacingConfig {
            mode: PacingMode::Unbounded,
            ..PacingConfig::default()
        };

        let pacer = Pacer::new(config).unwrap();

        pacer.wait_for(u64::MAX).unwrap();
    }
}
