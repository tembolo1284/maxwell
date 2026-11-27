//! Timestamp and duration primitives for Maxwell.
//!
//! Distinguishes between exchange timestamps, local receive
//! timestamps, and monotonic timestamps.
//!
//! All timestamps and durations use nanosecond resolution.
//!
//! Exchange and receive timestamps are expressed relative
//! to the Unix epoch. Monotonic timestamps are relative to
//! an unspecified, process-local clock origin.
//!
//! Timestamps from different clock domains must not be
//! directly compared without explicit synchronization.
//!
//! Nanosecond resolution does not imply nanosecond accuracy.

use core::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ============================================================
// Constants
// ============================================================

pub const NANOS_PER_MICROSECOND: u64 = 1_000;
pub const NANOS_PER_MILLISECOND: u64 = 1_000_000;
pub const NANOS_PER_SECOND: u64 = 1_000_000_000;

// ============================================================
// Timestamp errors
// ============================================================

/// Errors associated with timestamp acquisition and conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampError {
    BeforeUnixEpoch,
    Overflow,
    ClockWentBackward,
}

impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::BeforeUnixEpoch => "timestamp precedes the Unix epoch",
            Self::Overflow => "timestamp exceeds representable range",
            Self::ClockWentBackward => "clock moved backward",
        };

        f.write_str(message)
    }
}

impl std::error::Error for TimestampError {}

// ============================================================
// Nanosecond duration
// ============================================================

/// A nonnegative duration expressed in nanoseconds.
///
/// This type is intended for latency measurements, elapsed
/// time calculations, and timeout configuration.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NanoDuration(u64);

impl NanoDuration {
    // ========================================================
    // Constants
    // ========================================================

    pub const ZERO: Self = Self(0);
    pub const MIN: Self = Self::ZERO;
    pub const MAX: Self = Self(u64::MAX);

    // ========================================================
    // Construction
    // ========================================================

    #[must_use]
    #[inline]
    pub const fn from_nanos(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    #[inline]
    pub const fn from_micros(value: u64) -> Option<Self> {
        match value.checked_mul(NANOS_PER_MICROSECOND) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    #[must_use]
    #[inline]
    pub const fn from_millis(value: u64) -> Option<Self> {
        match value.checked_mul(NANOS_PER_MILLISECOND) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    #[must_use]
    #[inline]
    pub const fn from_secs(value: u64) -> Option<Self> {
        match value.checked_mul(NANOS_PER_SECOND) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    // ========================================================
    // Accessors
    // ========================================================

    #[must_use]
    #[inline]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Returns whole microseconds, truncating any remainder.
    #[must_use]
    #[inline]
    pub const fn as_micros(self) -> u64 {
        self.0 / NANOS_PER_MICROSECOND
    }

    /// Returns whole milliseconds, truncating any remainder.
    #[must_use]
    #[inline]
    pub const fn as_millis(self) -> u64 {
        self.0 / NANOS_PER_MILLISECOND
    }

    /// Returns whole seconds, truncating any remainder.
    #[must_use]
    #[inline]
    pub const fn as_secs(self) -> u64 {
        self.0 / NANOS_PER_SECOND
    }

    #[must_use]
    #[inline]
    pub const fn subsec_nanos(self) -> u32 {
        (self.0 % NANOS_PER_SECOND) as u32
    }

    #[must_use]
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    // ========================================================
    // Checked arithmetic
    // ========================================================

    #[must_use]
    #[inline]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.0.checked_add(rhs.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    #[inline]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    #[inline]
    pub const fn checked_mul(self, rhs: u64) -> Option<Self> {
        match self.0.checked_mul(rhs) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    #[inline]
    pub const fn checked_div(self, rhs: u64) -> Option<Self> {
        match self.0.checked_div(rhs) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    #[inline]
    pub const fn abs_diff(self, rhs: Self) -> Self {
        Self(self.0.abs_diff(rhs.0))
    }

    // ========================================================
    // Standard library interoperability
    // ========================================================

    #[must_use]
    #[inline]
    pub const fn as_std(self) -> Duration {
        Duration::from_nanos(self.0)
    }
}

impl From<NanoDuration> for Duration {
    #[inline]
    fn from(value: NanoDuration) -> Self {
        value.as_std()
    }
}

impl TryFrom<Duration> for NanoDuration {
    type Error = TimestampError;

    #[inline]
    fn try_from(value: Duration) -> Result<Self, Self::Error> {
        let nanos = u64::try_from(value.as_nanos()).map_err(|_| TimestampError::Overflow)?;

        Ok(Self(nanos))
    }
}

impl fmt::Display for NanoDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ns", self.0)
    }
}

// ============================================================
// Timestamp generation
// ============================================================

/// Defines a strongly typed nanosecond timestamp.
///
/// Separate timestamp types prevent accidental arithmetic
/// between different clock domains.
macro_rules! define_timestamp {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
        )]
        pub struct $name(u64);

        impl $name {
            pub const ZERO: Self = Self(0);
            pub const MIN: Self = Self::ZERO;
            pub const MAX: Self = Self(u64::MAX);

            /// Constructs a timestamp from raw nanoseconds.
            #[must_use]
            #[inline]
            pub const fn new(nanos: u64) -> Self {
                Self(nanos)
            }

            /// Returns the raw nanosecond representation.
            #[must_use]
            #[inline]
            pub const fn raw(self) -> u64 {
                self.0
            }

            #[must_use]
            #[inline]
            pub const fn is_zero(self) -> bool {
                self.0 == 0
            }

            /// Returns the elapsed duration since an earlier
            /// timestamp in the same clock domain.
            ///
            /// Returns None if earlier exceeds self.
            #[must_use]
            #[inline]
            pub const fn duration_since(
                self,
                earlier: Self,
            ) -> Option<NanoDuration> {
                match self.0.checked_sub(earlier.0) {
                    Some(value) => {
                        Some(NanoDuration::from_nanos(value))
                    }
                    None => None,
                }
            }

            /// Returns the absolute temporal distance.
            #[must_use]
            #[inline]
            pub const fn abs_diff(
                self,
                other: Self,
            ) -> NanoDuration {
                NanoDuration::from_nanos(
                    self.0.abs_diff(other.0),
                )
            }

            /// Advances the timestamp by a duration.
            ///
            /// Returns None on overflow.
            #[must_use]
            #[inline]
            pub const fn checked_add(
                self,
                duration: NanoDuration,
            ) -> Option<Self> {
                match self.0.checked_add(duration.as_nanos()) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            /// Moves the timestamp backward by a duration.
            ///
            /// Returns None on underflow.
            #[must_use]
            #[inline]
            pub const fn checked_sub(
                self,
                duration: NanoDuration,
            ) -> Option<Self> {
                match self.0.checked_sub(duration.as_nanos()) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }
        }

        impl From<$name> for u64 {
            #[inline]
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(
                &self,
                f: &mut fmt::Formatter<'_>,
            ) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

// ============================================================
// Clock domains
// ============================================================

define_timestamp!(
    /// Timestamp assigned by the authoritative exchange.
    ///
    /// Expressed in nanoseconds since the Unix epoch.
    ///
    /// Exchange timestamps must originate from exchange data
    /// rather than Maxwell's local system clock.
    ExchangeTimestamp
);

define_timestamp!(
    /// Local wall-clock timestamp associated with receipt.
    ///
    /// Expressed in nanoseconds since the Unix epoch.
    ///
    /// This clock may be adjusted by the operating system.
    /// It must not be used for reliable elapsed-time measurement.
    ReceiveTimestamp
);

define_timestamp!(
    /// Process-local monotonic timestamp.
    ///
    /// Intended for elapsed-time and latency measurements.
    ///
    /// Values are comparable only when they originate from
    /// the same process and monotonic clock origin.
    MonotonicTimestamp
);

// ============================================================
// Unix timestamp conversion
// ============================================================

fn system_time_to_nanos(time: SystemTime) -> Result<u64, TimestampError> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| TimestampError::BeforeUnixEpoch)?;

    u64::try_from(duration.as_nanos()).map_err(|_| TimestampError::Overflow)
}

fn nanos_to_system_time(nanos: u64) -> Result<SystemTime, TimestampError> {
    UNIX_EPOCH
        .checked_add(Duration::from_nanos(nanos))
        .ok_or(TimestampError::Overflow)
}

// ============================================================
// Exchange timestamp
// ============================================================

impl ExchangeTimestamp {
    /// Constructs an exchange timestamp from a `SystemTime`.
    ///
    /// # Errors
    ///
    /// Returns an error for timestamps preceding the Unix
    /// epoch or exceeding the supported nanosecond range.
    #[inline]
    pub fn from_system_time(time: SystemTime) -> Result<Self, TimestampError> {
        system_time_to_nanos(time).map(Self)
    }

    /// Converts the timestamp to `SystemTime`.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform cannot represent it.
    #[inline]
    pub fn to_system_time(self) -> Result<SystemTime, TimestampError> {
        nanos_to_system_time(self.0)
    }

    #[must_use]
    #[inline]
    pub const fn unix_seconds(self) -> u64 {
        self.0 / NANOS_PER_SECOND
    }

    #[must_use]
    #[inline]
    pub const fn subsec_nanos(self) -> u32 {
        (self.0 % NANOS_PER_SECOND) as u32
    }
}

// ============================================================
// Receive timestamp
// ============================================================

impl ReceiveTimestamp {
    /// Captures the current local wall-clock time.
    ///
    /// This represents the time of this function call, not
    /// necessarily the actual network packet arrival time.
    ///
    /// # Errors
    ///
    /// Returns an error if the system time is outside the
    /// supported Unix nanosecond range.
    #[inline]
    pub fn now() -> Result<Self, TimestampError> {
        Self::from_system_time(SystemTime::now())
    }

    /// Constructs a receive timestamp from `SystemTime`.
    ///
    /// # Errors
    ///
    /// Returns an error for timestamps preceding the Unix
    /// epoch or exceeding the supported range.
    #[inline]
    pub fn from_system_time(time: SystemTime) -> Result<Self, TimestampError> {
        system_time_to_nanos(time).map(Self)
    }

    /// Converts the timestamp to `SystemTime`.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform cannot represent it.
    #[inline]
    pub fn to_system_time(self) -> Result<SystemTime, TimestampError> {
        nanos_to_system_time(self.0)
    }

    #[must_use]
    #[inline]
    pub const fn unix_seconds(self) -> u64 {
        self.0 / NANOS_PER_SECOND
    }

    #[must_use]
    #[inline]
    pub const fn subsec_nanos(self) -> u32 {
        (self.0 % NANOS_PER_SECOND) as u32
    }

    /// Returns the signed difference between the local
    /// receive timestamp and the exchange timestamp.
    ///
    /// This is an observed wall-clock difference, not a
    /// guaranteed measurement of network latency.
    ///
    /// Clock synchronization errors may produce negative
    /// results.
    #[must_use]
    #[inline]
    pub fn observed_exchange_offset(self, exchange: ExchangeTimestamp) -> i128 {
        i128::from(self.0) - i128::from(exchange.raw())
    }
}

// ============================================================
// Monotonic clock
// ============================================================

static MONOTONIC_ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Process-local monotonic clock.
///
/// Initialize this clock before starting latency-sensitive
/// processing threads to avoid first-use initialization
/// overhead on the hot path.
#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    /// Obtains a clock sharing the process-wide origin.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: *MONOTONIC_ORIGIN.get_or_init(Instant::now),
        }
    }

    /// Captures the current monotonic timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying clock moves
    /// backward or the elapsed duration exceeds u64.
    #[inline]
    pub fn now(&self) -> Result<MonotonicTimestamp, TimestampError> {
        let elapsed = Instant::now()
            .checked_duration_since(self.origin)
            .ok_or(TimestampError::ClockWentBackward)?;

        let duration = NanoDuration::try_from(elapsed)?;

        Ok(MonotonicTimestamp::new(duration.as_nanos()))
    }

    /// Returns the elapsed duration since a previous timestamp.
    ///
    /// The timestamp must originate from this process.
    ///
    /// # Errors
    ///
    /// Returns an error if the current clock value precedes
    /// the supplied timestamp or cannot be represented.
    #[inline]
    pub fn elapsed_since(
        &self,
        earlier: MonotonicTimestamp,
    ) -> Result<NanoDuration, TimestampError> {
        self.now()?
            .duration_since(earlier)
            .ok_or(TimestampError::ClockWentBackward)
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================
// Compile-time layout guarantees
// ============================================================

const _: () = {
    assert!(size_of::<NanoDuration>() == size_of::<u64>());
    assert!(size_of::<ExchangeTimestamp>() == size_of::<u64>());
    assert!(size_of::<ReceiveTimestamp>() == size_of::<u64>());
    assert!(size_of::<MonotonicTimestamp>() == size_of::<u64>());

    assert!(align_of::<NanoDuration>() == align_of::<u64>());
    assert!(align_of::<ExchangeTimestamp>() == align_of::<u64>());
    assert!(align_of::<ReceiveTimestamp>() == align_of::<u64>());
    assert!(align_of::<MonotonicTimestamp>() == align_of::<u64>());
};

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================
    // Duration tests
    // ========================================================

    #[test]
    fn nanosecond_construction() {
        let duration = NanoDuration::from_nanos(500);

        assert_eq!(duration.as_nanos(), 500);
    }

    #[test]
    fn microsecond_construction() {
        assert_eq!(
            NanoDuration::from_micros(5),
            Some(NanoDuration::from_nanos(5_000)),
        );
    }

    #[test]
    fn millisecond_construction() {
        assert_eq!(
            NanoDuration::from_millis(5),
            Some(NanoDuration::from_nanos(5_000_000)),
        );
    }

    #[test]
    fn second_construction() {
        assert_eq!(
            NanoDuration::from_secs(2),
            Some(NanoDuration::from_nanos(2_000_000_000)),
        );
    }

    #[test]
    fn construction_overflow() {
        assert_eq!(NanoDuration::from_micros(u64::MAX), None);
        assert_eq!(NanoDuration::from_millis(u64::MAX), None);
        assert_eq!(NanoDuration::from_secs(u64::MAX), None);
    }

    #[test]
    fn duration_accessors() {
        let duration = NanoDuration::from_nanos(1_234_567_890);

        assert_eq!(duration.as_nanos(), 1_234_567_890);
        assert_eq!(duration.as_micros(), 1_234_567);
        assert_eq!(duration.as_millis(), 1_234);
        assert_eq!(duration.as_secs(), 1);
        assert_eq!(duration.subsec_nanos(), 234_567_890);
    }

    #[test]
    fn zero_duration() {
        assert!(NanoDuration::ZERO.is_zero());
        assert!(!NanoDuration::from_nanos(1).is_zero());
    }

    #[test]
    fn duration_ordering() {
        assert!(NanoDuration::from_nanos(100) < NanoDuration::from_nanos(200));
    }

    #[test]
    fn duration_addition() {
        assert_eq!(
            NanoDuration::from_nanos(100).checked_add(NanoDuration::from_nanos(50)),
            Some(NanoDuration::from_nanos(150)),
        );
    }

    #[test]
    fn duration_addition_overflow() {
        assert_eq!(
            NanoDuration::MAX.checked_add(NanoDuration::from_nanos(1),),
            None,
        );
    }

    #[test]
    fn duration_subtraction() {
        assert_eq!(
            NanoDuration::from_nanos(100).checked_sub(NanoDuration::from_nanos(50)),
            Some(NanoDuration::from_nanos(50)),
        );
    }

    #[test]
    fn duration_subtraction_underflow() {
        assert_eq!(
            NanoDuration::ZERO.checked_sub(NanoDuration::from_nanos(1),),
            None,
        );
    }

    #[test]
    fn duration_multiplication() {
        assert_eq!(
            NanoDuration::from_nanos(100).checked_mul(3),
            Some(NanoDuration::from_nanos(300)),
        );

        assert_eq!(NanoDuration::MAX.checked_mul(2), None);
    }

    #[test]
    fn duration_division() {
        assert_eq!(
            NanoDuration::from_nanos(103).checked_div(5),
            Some(NanoDuration::from_nanos(20)),
        );

        assert_eq!(NanoDuration::MAX.checked_div(0), None);
    }

    #[test]
    fn duration_absolute_difference() {
        assert_eq!(
            NanoDuration::from_nanos(100).abs_diff(NanoDuration::from_nanos(150)),
            NanoDuration::from_nanos(50),
        );
    }

    #[test]
    fn standard_duration_roundtrip() {
        let original = NanoDuration::from_nanos(123_456_789);
        let standard: Duration = original.into();

        assert_eq!(NanoDuration::try_from(standard), Ok(original),);
    }

    #[test]
    fn standard_duration_overflow() {
        let duration = Duration::from_secs(u64::MAX);

        assert_eq!(
            NanoDuration::try_from(duration),
            Err(TimestampError::Overflow),
        );
    }

    #[test]
    fn duration_display() {
        assert_eq!(NanoDuration::from_nanos(500).to_string(), "500 ns",);
    }

    // ========================================================
    // Exchange timestamp tests
    // ========================================================

    #[test]
    fn exchange_timestamp_construction() {
        let timestamp = ExchangeTimestamp::new(123_456);

        assert_eq!(timestamp.raw(), 123_456);
        assert_eq!(u64::from(timestamp), 123_456);
    }

    #[test]
    fn exchange_timestamp_ordering() {
        assert!(ExchangeTimestamp::new(100) < ExchangeTimestamp::new(200));
    }

    #[test]
    fn exchange_timestamp_duration() {
        assert_eq!(
            ExchangeTimestamp::new(150).duration_since(ExchangeTimestamp::new(100)),
            Some(NanoDuration::from_nanos(50)),
        );
    }

    #[test]
    fn exchange_timestamp_reversed_duration() {
        assert_eq!(
            ExchangeTimestamp::new(100).duration_since(ExchangeTimestamp::new(150)),
            None,
        );
    }

    #[test]
    fn exchange_timestamp_addition() {
        assert_eq!(
            ExchangeTimestamp::new(100).checked_add(NanoDuration::from_nanos(50)),
            Some(ExchangeTimestamp::new(150)),
        );
    }

    #[test]
    fn exchange_timestamp_overflow() {
        assert_eq!(
            ExchangeTimestamp::MAX.checked_add(NanoDuration::from_nanos(1),),
            None,
        );
    }

    #[test]
    fn exchange_timestamp_subtraction() {
        assert_eq!(
            ExchangeTimestamp::new(100).checked_sub(NanoDuration::from_nanos(50)),
            Some(ExchangeTimestamp::new(50)),
        );
    }

    #[test]
    fn exchange_timestamp_underflow() {
        assert_eq!(
            ExchangeTimestamp::ZERO.checked_sub(NanoDuration::from_nanos(1),),
            None,
        );
    }

    #[test]
    fn exchange_timestamp_absolute_difference() {
        assert_eq!(
            ExchangeTimestamp::MAX.abs_diff(ExchangeTimestamp::ZERO,),
            NanoDuration::MAX,
        );
    }

    #[test]
    fn exchange_timestamp_epoch() {
        assert_eq!(
            ExchangeTimestamp::from_system_time(UNIX_EPOCH),
            Ok(ExchangeTimestamp::ZERO),
        );
    }

    #[test]
    fn exchange_timestamp_before_epoch() {
        let time = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();

        assert_eq!(
            ExchangeTimestamp::from_system_time(time),
            Err(TimestampError::BeforeUnixEpoch),
        );
    }

    #[test]
    fn exchange_timestamp_roundtrip() {
        let original = ExchangeTimestamp::new(1_700_000_000_123_456_789);

        let system_time = original.to_system_time().unwrap();

        assert_eq!(
            ExchangeTimestamp::from_system_time(system_time),
            Ok(original),
        );
    }

    #[test]
    fn exchange_timestamp_components() {
        let timestamp = ExchangeTimestamp::new(1_234_567_890);

        assert_eq!(timestamp.unix_seconds(), 1);
        assert_eq!(timestamp.subsec_nanos(), 234_567_890);
    }

    // ========================================================
    // Receive timestamp tests
    // ========================================================

    #[test]
    fn receive_timestamp_construction() {
        let timestamp = ReceiveTimestamp::new(500);

        assert_eq!(timestamp.raw(), 500);
    }

    #[test]
    fn receive_timestamp_now() {
        let timestamp = ReceiveTimestamp::now().unwrap();

        assert!(!timestamp.is_zero());
    }

    #[test]
    fn receive_timestamp_roundtrip() {
        let original = ReceiveTimestamp::new(1_700_000_000_123_456_789);

        let system_time = original.to_system_time().unwrap();

        assert_eq!(
            ReceiveTimestamp::from_system_time(system_time),
            Ok(original),
        );
    }

    #[test]
    fn positive_exchange_offset() {
        assert_eq!(
            ReceiveTimestamp::new(150).observed_exchange_offset(ExchangeTimestamp::new(100),),
            50,
        );
    }

    #[test]
    fn negative_exchange_offset() {
        assert_eq!(
            ReceiveTimestamp::new(100).observed_exchange_offset(ExchangeTimestamp::new(150),),
            -50,
        );
    }

    #[test]
    fn zero_exchange_offset() {
        assert_eq!(
            ReceiveTimestamp::new(100).observed_exchange_offset(ExchangeTimestamp::new(100),),
            0,
        );
    }

    #[test]
    fn extreme_exchange_offsets() {
        assert_eq!(
            ReceiveTimestamp::MAX.observed_exchange_offset(ExchangeTimestamp::ZERO,),
            i128::from(u64::MAX),
        );

        assert_eq!(
            ReceiveTimestamp::ZERO.observed_exchange_offset(ExchangeTimestamp::MAX,),
            -i128::from(u64::MAX),
        );
    }

    // ========================================================
    // Monotonic timestamp tests
    // ========================================================

    #[test]
    fn monotonic_timestamp_construction() {
        let timestamp = MonotonicTimestamp::new(100);

        assert_eq!(timestamp.raw(), 100);
    }

    #[test]
    fn monotonic_timestamp_duration() {
        assert_eq!(
            MonotonicTimestamp::new(150).duration_since(MonotonicTimestamp::new(100)),
            Some(NanoDuration::from_nanos(50)),
        );
    }

    #[test]
    fn monotonic_timestamp_reversed_duration() {
        assert_eq!(
            MonotonicTimestamp::new(100).duration_since(MonotonicTimestamp::new(150)),
            None,
        );
    }

    #[test]
    fn monotonic_clock_ordering() {
        let clock = MonotonicClock::new();

        let first = clock.now().unwrap();
        let second = clock.now().unwrap();

        assert!(second >= first);
    }

    #[test]
    fn monotonic_clock_shared_origin() {
        let first_clock = MonotonicClock::new();
        let second_clock = MonotonicClock::new();

        assert_eq!(first_clock.origin, second_clock.origin);
    }

    #[test]
    fn monotonic_clock_elapsed() {
        let clock = MonotonicClock::new();
        let start = clock.now().unwrap();

        assert!(clock.elapsed_since(start).is_ok());
    }

    #[test]
    fn monotonic_clock_across_threads() {
        let clock = MonotonicClock::new();
        let start = clock.now().unwrap();

        let end = std::thread::spawn(move || clock.now().unwrap())
            .join()
            .unwrap();

        assert!(end >= start);
    }

    // ========================================================
    // Miscellaneous tests
    // ========================================================

    #[test]
    fn timestamp_display() {
        assert_eq!(ExchangeTimestamp::new(123).to_string(), "123",);

        assert_eq!(ReceiveTimestamp::new(456).to_string(), "456",);

        assert_eq!(MonotonicTimestamp::new(789).to_string(), "789",);
    }

    #[test]
    fn error_display() {
        assert_eq!(
            TimestampError::Overflow.to_string(),
            "timestamp exceeds representable range",
        );
    }

    #[test]
    fn const_construction() {
        const START: MonotonicTimestamp = MonotonicTimestamp::new(100);

        const END: MonotonicTimestamp = MonotonicTimestamp::new(200);

        const ELAPSED: Option<NanoDuration> = END.duration_since(START);

        assert_eq!(ELAPSED, Some(NanoDuration::from_nanos(100)),);
    }
}
