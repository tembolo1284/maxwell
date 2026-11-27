//! Deterministic and wall-clock replay scheduling.

use std::time::{Duration, Instant};

use crate::config::PlaybackMode;
use crate::error::{ReplayError, ReplayResult};

#[derive(Debug)]
pub struct ReplayClock {
    mode: PlaybackMode,
    origin_timestamp_ns: Option<u64>,
    wall_origin: Option<Instant>,
    virtual_elapsed_ns: u64,
}

impl ReplayClock {
    pub fn new(mode: PlaybackMode) -> ReplayResult<Self> {
        mode.validate()?;

        Ok(Self {
            mode,
            origin_timestamp_ns: None,
            wall_origin: None,
            virtual_elapsed_ns: 0,
        })
    }

    pub fn reset(&mut self) {
        self.origin_timestamp_ns = None;
        self.wall_origin = None;
        self.virtual_elapsed_ns = 0;
    }

    #[must_use]
    pub const fn mode(&self) -> PlaybackMode {
        self.mode
    }

    #[must_use]
    pub const fn virtual_elapsed_ns(&self) -> u64 {
        self.virtual_elapsed_ns
    }

    pub fn scaled_elapsed_ns(
        &self,
        elapsed_ns: u64,
    ) -> ReplayResult<u64> {
        let Some((numerator, denominator)) =
            self.mode.speed_ratio()
        else {
            return Ok(elapsed_ns);
        };

        let scaled = u128::from(elapsed_ns)
            * u128::from(denominator)
            / u128::from(numerator);

        u64::try_from(scaled)
            .map_err(|_| ReplayError::ClockOverflow)
    }

    pub fn remaining_delay(
        &mut self,
        timestamp_ns: u64,
        now: Instant,
    ) -> ReplayResult<Duration> {
        let origin = *self.origin_timestamp_ns
            .get_or_insert(timestamp_ns);

        let wall_origin = *self.wall_origin.get_or_insert(now);

        let elapsed = timestamp_ns.checked_sub(origin).ok_or(
            ReplayError::TimestampRegression {
                previous: origin,
                actual: timestamp_ns,
            },
        )?;

        if !self.mode.uses_wall_clock() {
            return Ok(Duration::ZERO);
        }

        let target = Duration::from_nanos(
            self.scaled_elapsed_ns(elapsed)?,
        );

        let actual = now.saturating_duration_since(wall_origin);

        Ok(target.saturating_sub(actual))
    }

    pub fn commit(
        &mut self,
        timestamp_ns: u64,
    ) -> ReplayResult<()> {
        let origin = self.origin_timestamp_ns.ok_or(
            ReplayError::InvalidState {
                operation: "clock commit",
                state: "not initialized",
            },
        )?;

        self.virtual_elapsed_ns =
            timestamp_ns.checked_sub(origin).ok_or(
                ReplayError::TimestampRegression {
                    previous: origin,
                    actual: timestamp_ns,
                },
            )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_clock_never_waits() {
        let mut clock =
            ReplayClock::new(PlaybackMode::Virtual).unwrap();

        let now = Instant::now();

        assert_eq!(
            clock.remaining_delay(100, now).unwrap(),
            Duration::ZERO
        );

        assert_eq!(
            clock.remaining_delay(200, now).unwrap(),
            Duration::ZERO
        );

        clock.commit(200).unwrap();

        assert_eq!(clock.virtual_elapsed_ns(), 100);
    }

    #[test]
    fn real_time_clock_preserves_intervals() {
        let mut clock =
            ReplayClock::new(PlaybackMode::RealTime).unwrap();

        let now = Instant::now();

        clock.remaining_delay(100, now).unwrap();

        assert_eq!(
            clock.remaining_delay(1_100, now).unwrap(),
            Duration::from_nanos(1_000)
        );
    }

    #[test]
    fn accelerated_playback() {
        let clock = ReplayClock::new(PlaybackMode::Scaled {
            numerator: 10,
            denominator: 1,
        })
        .unwrap();

        assert_eq!(
            clock.scaled_elapsed_ns(1_000).unwrap(),
            100
        );
    }

    #[test]
    fn slowed_playback() {
        let clock = ReplayClock::new(PlaybackMode::Scaled {
            numerator: 1,
            denominator: 2,
        })
        .unwrap();

        assert_eq!(
            clock.scaled_elapsed_ns(1_000).unwrap(),
            2_000
        );
    }

    #[test]
    fn scaling_overflow_is_rejected() {
        let clock = ReplayClock::new(PlaybackMode::Scaled {
            numerator: 1,
            denominator: 2,
        })
        .unwrap();

        assert!(clock.scaled_elapsed_ns(u64::MAX).is_err());
    }
}
