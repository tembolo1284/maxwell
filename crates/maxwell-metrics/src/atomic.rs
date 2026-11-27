//! Cache-line-aligned atomic instrumentation.
//!
//! These counters provide atomic telemetry only.
//! They must not be used to synchronize application state.

use std::sync::atomic::{AtomicU64, Ordering};

#[repr(align(64))]
#[derive(Debug, Default)]
pub struct AtomicCounter {
    value: AtomicU64,
}

impl AtomicCounter {
    #[inline]
    pub const fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    #[inline]
    pub const fn with_value(value: u64) -> Self {
        Self {
            value: AtomicU64::new(value),
        }
    }

    #[inline]
    pub fn value(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn increment(&self) {
        self.add(1);
    }

    /// Saturating atomic addition.
    ///
    /// Uses a compare-exchange loop to prevent overflow.
    /// Under contention, this operation may retry.
    #[inline]
    pub fn add(&self, amount: u64) {
        if amount == 0 {
            return;
        }

        let mut current = self.value.load(Ordering::Relaxed);

        loop {
            if current == u64::MAX {
                return;
            }

            let next = current.saturating_add(amount);

            match self.value.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    /// Atomically retrieve and reset the counter.
    ///
    /// Intended for interval-based telemetry collection.
    #[inline]
    pub fn take(&self) -> u64 {
        self.value.swap(0, Ordering::Relaxed)
    }

    /// Reset the counter, discarding its previous value.
    #[inline]
    pub fn reset(&self) {
        self.value.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};
    use std::thread;

    #[test]
    fn starts_at_zero() {
        assert_eq!(AtomicCounter::new().value(), 0);
    }

    #[test]
    fn increments() {
        let counter = AtomicCounter::new();

        counter.increment();
        counter.increment();

        assert_eq!(counter.value(), 2);
    }

    #[test]
    fn saturates_without_wrapping() {
        let counter = AtomicCounter::with_value(u64::MAX - 1);

        counter.add(100);
        counter.increment();

        assert_eq!(counter.value(), u64::MAX);
    }

    #[test]
    fn takes_and_resets_value() {
        let counter = AtomicCounter::with_value(100);

        assert_eq!(counter.take(), 100);
        assert_eq!(counter.value(), 0);
    }

    #[test]
    fn reset_discards_value() {
        let counter = AtomicCounter::with_value(100);

        counter.reset();

        assert_eq!(counter.value(), 0);
    }

    #[test]
    fn has_cache_line_alignment() {
        assert_eq!(align_of::<AtomicCounter>(), 64);
        assert_eq!(size_of::<AtomicCounter>() % 64, 0);
    }

    #[test]
    fn concurrent_increments_are_not_lost() {
        let counter = AtomicCounter::new();

        thread::scope(|scope| {
            for _ in 0..8 {
                let counter = &counter;

                scope.spawn(move || {
                    for _ in 0..10_000 {
                        counter.increment();
                    }
                });
            }
        });

        assert_eq!(counter.value(), 80_000);
    }

    #[test]
    fn concurrent_additions_saturate() {
        let counter = AtomicCounter::with_value(u64::MAX - 100);

        thread::scope(|scope| {
            for _ in 0..8 {
                let counter = &counter;

                scope.spawn(move || {
                    for _ in 0..100 {
                        counter.increment();
                    }
                });
            }
        });

        assert_eq!(counter.value(), u64::MAX);
    }
}
