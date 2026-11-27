//! Low-overhead instrumentation for Maxwell.
//!
//! Provides single-owner counters, gauges, fixed-capacity
//! latency histograms, atomic counters, deterministic
//! aggregation and Prometheus-compatible exposition.
//!
//! Metric recording does not allocate memory.
//! Formatting and exporting metrics belong outside
//! the latency-sensitive processing path.

#![forbid(unsafe_code)]

pub mod atomic;
pub mod counter;
pub mod error;
pub mod export;
pub mod gauge;
pub mod histogram;
pub mod registry;
pub mod snapshot;

pub use atomic::AtomicCounter;
pub use counter::Counter;
pub use error::MetricsError;
pub use export::write_prometheus;
pub use gauge::Gauge;
pub use histogram::{Histogram, HistogramSnapshot};
pub use registry::{MetricId, MetricsRegistry};
pub use snapshot::MetricSnapshot;
