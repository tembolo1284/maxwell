//! Prometheus-compatible text exposition.
//!
//! Exporting belongs outside the latency-sensitive
//! processing path.
//!
//! Histogram buckets are converted from internal
//! noncumulative counts to cumulative counts.

use std::io::{self, Write};

use crate::histogram::HistogramSnapshot;
use crate::snapshot::MetricSnapshot;

/// Write metric snapshots using Prometheus text exposition.
///
/// Snapshots are emitted in iterator order.
///
/// Metric names should originate from MetricsRegistry,
/// which validates names and prevents sample collisions.
///
/// Histogram values retain their original integer units.
pub fn write_prometheus<const N: usize, W, I>(writer: &mut W, snapshots: I) -> io::Result<()>
where
    W: Write + ?Sized,
    I: IntoIterator<Item = MetricSnapshot<N>>,
{
    for snapshot in snapshots {
        match snapshot {
            MetricSnapshot::Counter { name, value } => {
                writeln!(writer, "# TYPE {name} counter")?;
                writeln!(writer, "{name} {value}")?;
            }

            MetricSnapshot::Gauge { name, value } => {
                writeln!(writer, "# TYPE {name} gauge")?;
                writeln!(writer, "{name} {value}")?;
            }

            MetricSnapshot::Histogram { name, value } => {
                write_histogram(writer, name, &value)?;
            }
        }
    }

    Ok(())
}

fn write_histogram<const N: usize, W>(
    writer: &mut W,
    name: &str,
    histogram: &HistogramSnapshot<N>,
) -> io::Result<()>
where
    W: Write + ?Sized,
{
    writeln!(writer, "# TYPE {name} histogram")?;

    let mut cumulative = 0u64;

    for index in 0..N {
        cumulative = cumulative.saturating_add(histogram.buckets[index]);

        writeln!(
            writer,
            "{name}_bucket{{le=\"{}\"}} {cumulative}",
            histogram.bounds[index],
        )?;
    }

    writeln!(writer, "{name}_bucket{{le=\"+Inf\"}} {}", histogram.count,)?;

    writeln!(writer, "{name}_sum {}", histogram.sum,)?;

    writeln!(writer, "{name}_count {}", histogram.count,)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::histogram::Histogram;
    use crate::registry::MetricsRegistry;

    #[test]
    fn exports_counter() {
        let snapshot = MetricSnapshot::<3>::Counter {
            name: "maxwell_messages_total",
            value: 100,
        };

        let mut output = Vec::new();

        write_prometheus(&mut output, [snapshot]).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!(
                "# TYPE maxwell_messages_total counter\n",
                "maxwell_messages_total 100\n",
            )
        );
    }

    #[test]
    fn exports_signed_gauge() {
        let snapshot = MetricSnapshot::<3>::Gauge {
            name: "maxwell_position",
            value: -100,
        };

        let mut output = Vec::new();

        write_prometheus(&mut output, [snapshot]).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!("# TYPE maxwell_position gauge\n", "maxwell_position -100\n",)
        );
    }

    #[test]
    fn exports_cumulative_histogram_buckets() {
        let mut histogram = Histogram::new([10, 20, 30]).unwrap();

        for value in [5, 10, 15, 25, 100] {
            histogram.record(value);
        }

        let snapshot = MetricSnapshot::Histogram {
            name: "latency",
            value: histogram.snapshot(),
        };

        let mut output = Vec::new();

        write_prometheus(&mut output, [snapshot]).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!(
                "# TYPE latency histogram\n",
                "latency_bucket{le=\"10\"} 2\n",
                "latency_bucket{le=\"20\"} 3\n",
                "latency_bucket{le=\"30\"} 4\n",
                "latency_bucket{le=\"+Inf\"} 5\n",
                "latency_sum 155\n",
                "latency_count 5\n",
            )
        );
    }

    #[test]
    fn exports_empty_histogram() {
        let histogram = Histogram::new([10, 20]).unwrap();

        let snapshot = MetricSnapshot::Histogram {
            name: "latency",
            value: histogram.snapshot(),
        };

        let mut output = Vec::new();

        write_prometheus(&mut output, [snapshot]).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!(
                "# TYPE latency histogram\n",
                "latency_bucket{le=\"10\"} 0\n",
                "latency_bucket{le=\"20\"} 0\n",
                "latency_bucket{le=\"+Inf\"} 0\n",
                "latency_sum 0\n",
                "latency_count 0\n",
            )
        );
    }

    #[test]
    fn exports_registry_in_registration_order() {
        let mut registry = MetricsRegistry::<3>::new(2).unwrap();

        let counter = registry.register_counter("z_total").unwrap();

        let gauge = registry.register_gauge("a").unwrap();

        registry.add(counter, 100).unwrap();
        registry.set_gauge(gauge, 20).unwrap();

        let mut output = Vec::new();

        write_prometheus(&mut output, registry.snapshots()).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!(
                "# TYPE z_total counter\n",
                "z_total 100\n",
                "# TYPE a gauge\n",
                "a 20\n",
            )
        );
    }

    #[test]
    fn empty_registry_produces_empty_output() {
        let registry = MetricsRegistry::<3>::new(1).unwrap();

        let mut output = Vec::new();

        write_prometheus(&mut output, registry.snapshots()).unwrap();

        assert!(output.is_empty());
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "simulated write failure",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn propagates_writer_errors() {
        let snapshot = MetricSnapshot::<3>::Counter {
            name: "messages_total",
            value: 100,
        };

        let error = write_prometheus(&mut FailingWriter, [snapshot]).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
