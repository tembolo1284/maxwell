//! Single-owner signed gauges.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Gauge {
    value: i64,
}

impl Gauge {
    #[inline]
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    #[inline]
    pub const fn with_value(value: i64) -> Self {
        Self { value }
    }

    #[inline]
    pub const fn value(&self) -> i64 {
        self.value
    }

    #[inline]
    pub fn set(&mut self, value: i64) {
        self.value = value;
    }

    #[inline]
    pub fn increment(&mut self) {
        self.add(1);
    }

    #[inline]
    pub fn decrement(&mut self) {
        self.subtract(1);
    }

    #[inline]
    pub fn add(&mut self, amount: i64) {
        self.value = self.value.saturating_add(amount);
    }

    #[inline]
    pub fn subtract(&mut self, amount: i64) {
        self.value = self.value.saturating_sub(amount);
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
        assert_eq!(Gauge::new().value(), 0);
    }

    #[test]
    fn sets_value() {
        let mut gauge = Gauge::new();

        gauge.set(100);

        assert_eq!(gauge.value(), 100);
    }

    #[test]
    fn increments_and_decrements() {
        let mut gauge = Gauge::new();

        gauge.increment();
        gauge.increment();
        gauge.decrement();

        assert_eq!(gauge.value(), 1);
    }

    #[test]
    fn supports_negative_values() {
        let mut gauge = Gauge::new();

        gauge.subtract(100);

        assert_eq!(gauge.value(), -100);
    }

    #[test]
    fn saturates_at_upper_bound() {
        let mut gauge = Gauge::with_value(i64::MAX);

        gauge.increment();

        assert_eq!(gauge.value(), i64::MAX);
    }

    #[test]
    fn saturates_at_lower_bound() {
        let mut gauge = Gauge::with_value(i64::MIN);

        gauge.decrement();

        assert_eq!(gauge.value(), i64::MIN);
    }

    #[test]
    fn handles_minimum_signed_operand() {
        let mut gauge = Gauge::new();

        gauge.subtract(i64::MIN);

        assert_eq!(gauge.value(), i64::MAX);
    }

    #[test]
    fn resets_gauge() {
        let mut gauge = Gauge::with_value(-100);

        gauge.reset();

        assert_eq!(gauge.value(), 0);
    }
}
