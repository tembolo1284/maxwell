//! Bounded single-owner metrics registry.
//!
//! Metrics receive stable numeric identifiers at
//! registration time. Recording uses indexed access
//! rather than name-based lookup.
//!
//! Registration order determines snapshot order.

use crate::counter::Counter;
use crate::error::MetricsError;
use crate::gauge::Gauge;
use crate::histogram::Histogram;
use crate::snapshot::MetricSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetricId(usize);

impl MetricId {
    #[inline]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Metric<const N: usize> {
    Counter(Counter),
    Gauge(Gauge),
    Histogram(Histogram<N>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MetricEntry<const N: usize> {
    name: &'static str,
    metric: Metric<N>,
}

#[derive(Debug)]
pub struct MetricsRegistry<const N: usize> {
    entries: Vec<MetricEntry<N>>,
    max_metrics: usize,
}

impl<const N: usize> MetricsRegistry<N> {
    pub fn new(max_metrics: usize) -> Result<Self, MetricsError> {
        if max_metrics == 0 {
            return Err(MetricsError::InvalidCapacity);
        }

        let mut entries = Vec::new();

        entries
            .try_reserve_exact(max_metrics)
            .map_err(|_| MetricsError::AllocationFailed)?;

        Ok(Self {
            entries,
            max_metrics,
        })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub const fn capacity(&self) -> usize {
        self.max_metrics
    }

    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.max_metrics - self.entries.len()
    }

    /// Validate the conservative Prometheus metric name
    /// syntax used by Maxwell.
    pub fn valid_name(name: &str) -> bool {
        let mut bytes = name.bytes();

        let Some(first) = bytes.next() else {
            return false;
        };

        if !(first.is_ascii_alphabetic() || first == b'_') {
            return false;
        }

        bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    }

    /// Resolve a name during configuration or reporting.
    ///
    /// The processing path should retain MetricId values.
    pub fn find(&self, name: &str) -> Option<MetricId> {
        self.entries
            .iter()
            .position(|entry| entry.name == name)
            .map(MetricId)
    }

    /// Determine whether two exported metric families
    /// would produce overlapping sample names.
    fn names_conflict(
        first: &str,
        first_histogram: bool,
        second: &str,
        second_histogram: bool,
    ) -> bool {
        if first == second {
            return true;
        }

        let suffixes = ["_bucket", "_sum", "_count"];

        if first_histogram
            && second
                .strip_prefix(first)
                .is_some_and(|suffix| suffixes.contains(&suffix))
        {
            return true;
        }

        if second_histogram
            && first
                .strip_prefix(second)
                .is_some_and(|suffix| suffixes.contains(&suffix))
        {
            return true;
        }

        false
    }

    fn register(
        &mut self,
        name: &'static str,
        metric: Metric<N>,
    ) -> Result<MetricId, MetricsError> {
        if !Self::valid_name(name) {
            return Err(MetricsError::InvalidMetricName);
        }

        let is_histogram = matches!(&metric, Metric::Histogram(_));

        for entry in &self.entries {
            let existing_histogram = matches!(&entry.metric, Metric::Histogram(_));

            if Self::names_conflict(name, is_histogram, entry.name, existing_histogram) {
                return Err(MetricsError::DuplicateMetricName);
            }
        }

        if self.entries.len() == self.max_metrics {
            return Err(MetricsError::RegistryFull);
        }

        let id = MetricId(self.entries.len());

        self.entries.push(MetricEntry { name, metric });

        Ok(id)
    }

    pub fn register_counter(&mut self, name: &'static str) -> Result<MetricId, MetricsError> {
        self.register(name, Metric::Counter(Counter::new()))
    }

    pub fn register_gauge(&mut self, name: &'static str) -> Result<MetricId, MetricsError> {
        self.register(name, Metric::Gauge(Gauge::new()))
    }

    pub fn register_histogram(
        &mut self,
        name: &'static str,
        bounds: [u64; N],
    ) -> Result<MetricId, MetricsError> {
        let histogram = Histogram::new(bounds)?;

        self.register(name, Metric::Histogram(histogram))
    }

    #[inline]
    fn entry(&self, id: MetricId) -> Result<&MetricEntry<N>, MetricsError> {
        self.entries
            .get(id.index())
            .ok_or(MetricsError::UnknownMetric)
    }

    #[inline]
    fn entry_mut(&mut self, id: MetricId) -> Result<&mut MetricEntry<N>, MetricsError> {
        self.entries
            .get_mut(id.index())
            .ok_or(MetricsError::UnknownMetric)
    }

    #[inline]
    pub fn counter(&self, id: MetricId) -> Result<&Counter, MetricsError> {
        match &self.entry(id)?.metric {
            Metric::Counter(counter) => Ok(counter),
            _ => Err(MetricsError::MetricTypeMismatch),
        }
    }

    #[inline]
    pub fn counter_mut(&mut self, id: MetricId) -> Result<&mut Counter, MetricsError> {
        match &mut self.entry_mut(id)?.metric {
            Metric::Counter(counter) => Ok(counter),
            _ => Err(MetricsError::MetricTypeMismatch),
        }
    }

    #[inline]
    pub fn gauge(&self, id: MetricId) -> Result<&Gauge, MetricsError> {
        match &self.entry(id)?.metric {
            Metric::Gauge(gauge) => Ok(gauge),
            _ => Err(MetricsError::MetricTypeMismatch),
        }
    }

    #[inline]
    pub fn gauge_mut(&mut self, id: MetricId) -> Result<&mut Gauge, MetricsError> {
        match &mut self.entry_mut(id)?.metric {
            Metric::Gauge(gauge) => Ok(gauge),
            _ => Err(MetricsError::MetricTypeMismatch),
        }
    }

    #[inline]
    pub fn histogram(&self, id: MetricId) -> Result<&Histogram<N>, MetricsError> {
        match &self.entry(id)?.metric {
            Metric::Histogram(histogram) => Ok(histogram),
            _ => Err(MetricsError::MetricTypeMismatch),
        }
    }

    #[inline]
    pub fn histogram_mut(&mut self, id: MetricId) -> Result<&mut Histogram<N>, MetricsError> {
        match &mut self.entry_mut(id)?.metric {
            Metric::Histogram(histogram) => Ok(histogram),
            _ => Err(MetricsError::MetricTypeMismatch),
        }
    }

    #[inline]
    pub fn increment(&mut self, id: MetricId) -> Result<(), MetricsError> {
        self.counter_mut(id)?.increment();

        Ok(())
    }

    #[inline]
    pub fn add(&mut self, id: MetricId, amount: u64) -> Result<(), MetricsError> {
        self.counter_mut(id)?.add(amount);

        Ok(())
    }

    #[inline]
    pub fn set_gauge(&mut self, id: MetricId, value: i64) -> Result<(), MetricsError> {
        self.gauge_mut(id)?.set(value);

        Ok(())
    }

    #[inline]
    pub fn record(&mut self, id: MetricId, value: u64) -> Result<bool, MetricsError> {
        Ok(self.histogram_mut(id)?.record(value))
    }

    fn snapshot_entry(entry: &MetricEntry<N>) -> MetricSnapshot<N> {
        match &entry.metric {
            Metric::Counter(counter) => MetricSnapshot::Counter {
                name: entry.name,
                value: counter.value(),
            },

            Metric::Gauge(gauge) => MetricSnapshot::Gauge {
                name: entry.name,
                value: gauge.value(),
            },

            Metric::Histogram(histogram) => MetricSnapshot::Histogram {
                name: entry.name,
                value: histogram.snapshot(),
            },
        }
    }

    pub fn snapshot(&self, id: MetricId) -> Result<MetricSnapshot<N>, MetricsError> {
        Ok(Self::snapshot_entry(self.entry(id)?))
    }

    /// Iterate over snapshots in registration order.
    ///
    /// No intermediate collection is allocated.
    pub fn snapshots(&self) -> impl ExactSizeIterator<Item = MetricSnapshot<N>> + '_ {
        self.entries.iter().map(Self::snapshot_entry)
    }

    pub fn reset(&mut self, id: MetricId) -> Result<(), MetricsError> {
        match &mut self.entry_mut(id)?.metric {
            Metric::Counter(counter) => counter.reset(),
            Metric::Gauge(gauge) => gauge.reset(),
            Metric::Histogram(histogram) => histogram.reset(),
        }

        Ok(())
    }

    /// Reset all values without removing registered metrics.
    pub fn reset_all(&mut self) {
        for entry in &mut self.entries {
            match &mut entry.metric {
                Metric::Counter(counter) => counter.reset(),
                Metric::Gauge(gauge) => gauge.reset(),
                Metric::Histogram(histogram) => histogram.reset(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Registry = MetricsRegistry<3>;

    #[test]
    fn rejects_zero_capacity() {
        assert!(matches!(
            Registry::new(0),
            Err(MetricsError::InvalidCapacity)
        ));
    }

    #[test]
    fn assigns_stable_identifiers() {
        let mut registry = Registry::new(3).unwrap();

        let first = registry.register_counter("first_total").unwrap();
        let second = registry.register_gauge("second").unwrap();
        let third = registry.register_histogram("third", [10, 20, 30]).unwrap();

        assert_eq!(first.index(), 0);
        assert_eq!(second.index(), 1);
        assert_eq!(third.index(), 2);
    }

    #[test]
    fn enforces_capacity() {
        let mut registry = Registry::new(1).unwrap();

        registry.register_counter("first_total").unwrap();

        assert_eq!(
            registry.register_counter("second_total"),
            Err(MetricsError::RegistryFull)
        );

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.remaining_capacity(), 0);
    }

    #[test]
    fn rejects_duplicate_names() {
        let mut registry = Registry::new(2).unwrap();

        registry.register_counter("messages_total").unwrap();

        assert_eq!(
            registry.register_gauge("messages_total"),
            Err(MetricsError::DuplicateMetricName)
        );
    }

    #[test]
    fn validates_metric_names() {
        for name in [
            "maxwell_messages_total",
            "_internal",
            "latency_ns",
            "queue2_depth",
        ] {
            assert!(Registry::valid_name(name));
        }

        for name in [
            "",
            "2queue",
            "queue-depth",
            "queue depth",
            "queue.depth",
            "queue\nname",
        ] {
            assert!(!Registry::valid_name(name));
        }
    }

    #[test]
    fn rejects_invalid_name_registration() {
        let mut registry = Registry::new(1).unwrap();

        assert_eq!(
            registry.register_counter("invalid-name"),
            Err(MetricsError::InvalidMetricName)
        );
    }

    #[test]
    fn rejects_histogram_sample_name_collision() {
        let mut registry = Registry::new(3).unwrap();

        registry
            .register_histogram("latency", [10, 20, 30])
            .unwrap();

        assert_eq!(
            registry.register_gauge("latency_count"),
            Err(MetricsError::DuplicateMetricName)
        );
    }

    #[test]
    fn rejects_reverse_histogram_name_collision() {
        let mut registry = Registry::new(3).unwrap();

        registry.register_counter("latency_sum").unwrap();

        assert_eq!(
            registry.register_histogram("latency", [10, 20, 30]),
            Err(MetricsError::DuplicateMetricName)
        );
    }

    #[test]
    fn rejects_unknown_identifier() {
        let registry = Registry::new(1).unwrap();

        assert_eq!(
            registry.counter(MetricId::new(100)),
            Err(MetricsError::UnknownMetric)
        );
    }

    #[test]
    fn rejects_metric_type_mismatch() {
        let mut registry = Registry::new(1).unwrap();
        let id = registry.register_gauge("depth").unwrap();

        assert_eq!(
            registry.increment(id),
            Err(MetricsError::MetricTypeMismatch)
        );
    }

    #[test]
    fn records_counter_values() {
        let mut registry = Registry::new(1).unwrap();
        let id = registry.register_counter("messages_total").unwrap();

        registry.increment(id).unwrap();
        registry.add(id, 99).unwrap();

        assert_eq!(registry.counter(id).unwrap().value(), 100);
    }

    #[test]
    fn records_gauge_values() {
        let mut registry = Registry::new(1).unwrap();
        let id = registry.register_gauge("depth").unwrap();

        registry.set_gauge(id, 100).unwrap();
        registry.gauge_mut(id).unwrap().decrement();

        assert_eq!(registry.gauge(id).unwrap().value(), 99);
    }

    #[test]
    fn records_histogram_values() {
        let mut registry = Registry::new(1).unwrap();

        let id = registry
            .register_histogram("latency", [10, 20, 30])
            .unwrap();

        registry.record(id, 5).unwrap();
        registry.record(id, 15).unwrap();
        registry.record(id, 35).unwrap();

        let histogram = registry.histogram(id).unwrap();

        assert_eq!(histogram.count(), 3);
        assert_eq!(histogram.buckets(), &[1, 1, 0]);
        assert_eq!(histogram.overflow(), 1);
    }

    #[test]
    fn resolves_registered_names() {
        let mut registry = Registry::new(2).unwrap();

        let id = registry.register_counter("messages_total").unwrap();

        assert_eq!(registry.find("messages_total"), Some(id));
        assert_eq!(registry.find("unknown"), None);
    }

    #[test]
    fn snapshots_preserve_registration_order() {
        let mut registry = Registry::new(3).unwrap();

        registry.register_counter("z_total").unwrap();
        registry.register_gauge("a").unwrap();
        registry.register_counter("m_total").unwrap();

        let names: Vec<_> = registry
            .snapshots()
            .map(|snapshot| snapshot.name())
            .collect();

        assert_eq!(names, vec!["z_total", "a", "m_total"]);
    }

    #[test]
    fn reset_preserves_identifiers() {
        let mut registry = Registry::new(1).unwrap();
        let id = registry.register_counter("messages_total").unwrap();

        registry.add(id, 100).unwrap();
        registry.reset_all();

        assert_eq!(registry.counter(id).unwrap().value(), 0);
        assert_eq!(registry.find("messages_total"), Some(id));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn recording_does_not_expand_registry_storage() {
        let mut registry = Registry::new(3).unwrap();

        let counter = registry.register_counter("messages_total").unwrap();
        let gauge = registry.register_gauge("depth").unwrap();

        let histogram = registry
            .register_histogram("latency", [10, 20, 30])
            .unwrap();

        let capacity = registry.entries.capacity();

        for value in 0..100_000u64 {
            registry.increment(counter).unwrap();
            registry.set_gauge(gauge, value as i64).unwrap();
            registry.record(histogram, value).unwrap();
        }

        assert_eq!(registry.entries.capacity(), capacity);
        assert_eq!(registry.len(), 3);
    }
}
