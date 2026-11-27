//! Owned metric snapshots for telemetry collection.

use crate::histogram::HistogramSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricSnapshot<const N: usize> {
    Counter {
        name: &'static str,
        value: u64,
    },

    Gauge {
        name: &'static str,
        value: i64,
    },

    Histogram {
        name: &'static str,
        value: HistogramSnapshot<N>,
    },
}

impl<const N: usize> MetricSnapshot<N> {
    #[inline]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Counter { name, .. }
            | Self::Gauge { name, .. }
            | Self::Histogram { name, .. } => name,
        }
    }

    #[inline]
    pub const fn is_counter(&self) -> bool {
        matches!(self, Self::Counter { .. })
    }

    #[inline]
    pub const fn is_gauge(&self) -> bool {
        matches!(self, Self::Gauge { .. })
    }

    #[inline]
    pub const fn is_histogram(&self) -> bool {
        matches!(self, Self::Histogram { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::histogram::Histogram;

    #[test]
    fn identifies_counter_snapshot() {
        let snapshot = MetricSnapshot::<3>::Counter {
            name: "messages_total",
            value: 100,
        };

        assert_eq!(snapshot.name(), "messages_total");
        assert!(snapshot.is_counter());
        assert!(!snapshot.is_gauge());
        assert!(!snapshot.is_histogram());
    }

    #[test]
    fn identifies_gauge_snapshot() {
        let snapshot = MetricSnapshot::<3>::Gauge {
            name: "queue_depth",
            value: 10,
        };

        assert_eq!(snapshot.name(), "queue_depth");
        assert!(snapshot.is_gauge());
    }

    #[test]
    fn identifies_histogram_snapshot() {
        let histogram = Histogram::new([10, 20, 30]).unwrap();

        let snapshot = MetricSnapshot::Histogram {
            name: "latency_nanoseconds",
            value: histogram.snapshot(),
        };

        assert_eq!(snapshot.name(), "latency_nanoseconds");
        assert!(snapshot.is_histogram());
    }
}
