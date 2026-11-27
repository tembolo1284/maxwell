//! Single-owner monotonically increasing counters.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counter {
    value: u64,
}

impl Counter {
    #[inline]
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    #[inline]
    pub const fn with_value(value: u64) -> Self {
        Self { value }
    }

    #[inline]
    pub const fn value(&self) -> u64 {
        self.value
    }

    #[inline]
    pub fn increment(&mut self) {
        self.add(1);
    }

    #[inline]
    pub fn add(&mut self, amount: u64) {
        self.value = self.value.saturating_add(amount);
    }

    #[inline]
    pub fn merge(&mut self, other: &Self) {
        self.add(other.value);
    }

    #[inline]
    pub fn reset(&mut self) {
        self.value = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        assert_eq!(Counter::new().value(), 0);
    }

    #[test]
    fn supports_initial_value() {
        assert_eq!(Counter::with_value(42).value(), 42);
    }

    #[test]
    fn increments() {
        let mut counter = Counter::new();

        counter.increment();
        counter.increment();

        assert_eq!(counter.value(), 2);
    }

    #[test]
    fn adds_multiple_observations() {
        let mut counter = Counter::new();

        counter.add(100);
        counter.add(200);

        assert_eq!(counter.value(), 300);
    }

    #[test]
    fn saturates_without_wrapping() {
        let mut counter = Counter::with_value(u64::MAX - 1);

        counter.add(100);

        assert_eq!(counter.value(), u64::MAX);
    }

    #[test]
    fn merges_counters() {
        let mut first = Counter::with_value(100);
        let second = Counter::with_value(200);

        first.merge(&second);

        assert_eq!(first.value(), 300);
        assert_eq!(second.value(), 200);
    }

    #[test]
    fn resets_counter() {
        let mut counter = Counter::with_value(100);

        counter.reset();

        assert_eq!(counter.value(), 0);
    }
}
