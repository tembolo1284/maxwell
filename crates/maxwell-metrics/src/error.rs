//! Metrics-specific error definitions.

use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsError {
    InvalidCapacity,
    InvalidHistogramBounds,
    IncompatibleHistogramBounds,
    InvalidPercentile,
    InvalidMetricName,
    DuplicateMetricName,
    RegistryFull,
    UnknownMetric,
    MetricTypeMismatch,
    AllocationFailed,
}

impl fmt::Display for MetricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCapacity => {
                write!(f, "metric capacity must be greater than zero")
            }

            Self::InvalidHistogramBounds => {
                write!(
                    f,
                    "histogram bounds must be nonempty and strictly increasing"
                )
            }

            Self::IncompatibleHistogramBounds => {
                write!(f, "histogram bounds are incompatible")
            }

            Self::InvalidPercentile => {
                write!(f, "percentile must be between 0 and 10000 basis points")
            }

            Self::InvalidMetricName => {
                write!(f, "invalid metric name")
            }

            Self::DuplicateMetricName => {
                write!(f, "duplicate metric name")
            }

            Self::RegistryFull => {
                write!(f, "metrics registry capacity exceeded")
            }

            Self::UnknownMetric => {
                write!(f, "unknown metric identifier")
            }

            Self::MetricTypeMismatch => {
                write!(f, "metric type mismatch")
            }

            Self::AllocationFailed => {
                write!(f, "metrics resource allocation failed")
            }
        }
    }
}

impl Error for MetricsError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_registry_full() {
        assert_eq!(
            MetricsError::RegistryFull.to_string(),
            "metrics registry capacity exceeded"
        );
    }

    #[test]
    fn formats_type_mismatch() {
        assert_eq!(
            MetricsError::MetricTypeMismatch.to_string(),
            "metric type mismatch"
        );
    }
}
