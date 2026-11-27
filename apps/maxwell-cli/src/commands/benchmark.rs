//! Repeatable local simulator throughput benchmarking.

use std::io::Write;
use std::time::{Duration, Instant};

use maxwell_core::FeedId;

use maxwell_simulator::SimulatorConfig;
use maxwell_simulator::simulator::{NullSink, RuntimeLimits, Simulator};

use crate::config::BenchmarkOptions;
use crate::error::{CliError, CliResult};
use crate::output::events_per_second;

// ============================================================
// Benchmark measurements
// ============================================================

#[derive(Debug, Clone, Copy)]
struct Measurement {
    events: u64,
    elapsed: Duration,
}

impl Measurement {
    fn throughput(self) -> f64 {
        events_per_second(self.events, self.elapsed)
    }
}

#[derive(Debug)]
struct BenchmarkSummary {
    minimum: f64,
    median: f64,
    maximum: f64,
    mean: f64,
    aggregate: f64,
    total_events: u128,
    total_elapsed: Duration,
}

impl BenchmarkSummary {
    fn from_measurements(measurements: &[Measurement]) -> CliResult<Self> {
        if measurements.is_empty() {
            return Err(CliError::invalid_state(
                "benchmark",
                "no measurements were collected",
            ));
        }

        let mut rates: Vec<f64> = measurements
            .iter()
            .map(|measurement| measurement.throughput())
            .collect();

        rates.sort_unstable_by(f64::total_cmp);

        let minimum = rates[0];
        let maximum = rates[rates.len() - 1];

        let midpoint = rates.len() / 2;

        let median = if rates.len().is_multiple_of(2) {
            (rates[midpoint - 1] + rates[midpoint]) / 2.0
        } else {
            rates[midpoint]
        };

        let mean = rates.iter().sum::<f64>() / rates.len() as f64;

        let total_events = measurements
            .iter()
            .map(|measurement| u128::from(measurement.events))
            .sum::<u128>();

        let total_elapsed = measurements
            .iter()
            .map(|measurement| measurement.elapsed)
            .sum::<Duration>();

        let seconds = total_elapsed.as_secs_f64();

        let aggregate = if seconds > 0.0 {
            total_events as f64 / seconds
        } else {
            0.0
        };

        Ok(Self {
            minimum,
            median,
            maximum,
            mean,
            aggregate,
            total_events,
            total_elapsed,
        })
    }
}

// ============================================================
// Benchmark execution
// ============================================================

fn run_iteration(
    config: SimulatorConfig,
    feed: FeedId,
    limits: RuntimeLimits,
) -> CliResult<Measurement> {
    // Initialization is deliberately excluded from
    // the measured execution interval.
    let mut simulator = Simulator::new(config, feed, limits)?;

    let mut sink = NullSink;

    let started = Instant::now();

    let report = simulator.run_until_blocked(&mut sink)?;

    let elapsed = started.elapsed();

    if !report.is_completed() {
        return Err(CliError::invalid_state(
            "benchmark",
            "simulation did not complete",
        ));
    }

    // Full consistency validation is outside the timed region.
    simulator.validate_consistency()?;

    let stats = simulator.stats();

    if !stats.is_fully_accounted()
        || stats.generated() != config.run.event_count
        || stats.delivery.accepted != stats.generated()
    {
        return Err(CliError::invalid_state(
            "benchmark",
            "simulation event accounting mismatch",
        ));
    }

    Ok(Measurement {
        events: stats.generated(),
        elapsed,
    })
}

pub fn run<W: Write>(options: BenchmarkOptions, output: &mut W) -> CliResult<()> {
    let config = options.simulator_config()?;

    let feed = options.simulation.feed();

    let limits = options.simulation.runtime_limits(&config)?;

    writeln!(output, "Maxwell simulator benchmark")?;
    writeln!(output, "---------------------------")?;

    writeln!(
        output,
        "Scenario:          {}",
        options.simulation.scenario.as_str(),
    )?;

    writeln!(output, "Seed:              {}", config.run.seed)?;

    writeln!(output, "Events/iteration:  {}", config.run.event_count,)?;

    writeln!(
        output,
        "Instruments:       {}",
        config.market.instrument_count,
    )?;

    writeln!(output, "Warmup iterations: {}", options.warmup)?;
    writeln!(output, "Measured iterations: {}", options.iterations)?;

    writeln!(output)?;

    for iteration in 0..options.warmup {
        let measurement = run_iteration(config, feed, limits)?;

        writeln!(
            output,
            "Warmup {:>3}: {:>14.2} events/second",
            iteration + 1,
            measurement.throughput(),
        )?;
    }

    let mut measurements = Vec::with_capacity(options.iterations as usize);

    for iteration in 0..options.iterations {
        let measurement = run_iteration(config, feed, limits)?;

        writeln!(
            output,
            "Run    {:>3}: {:>14.2} events/second \
             ({:.6} seconds)",
            iteration + 1,
            measurement.throughput(),
            measurement.elapsed.as_secs_f64(),
        )?;

        measurements.push(measurement);
    }

    let summary = BenchmarkSummary::from_measurements(&measurements)?;

    writeln!(output)?;
    writeln!(output, "Benchmark summary")?;
    writeln!(output, "-----------------")?;

    writeln!(
        output,
        "Minimum:           {:.2} events/second",
        summary.minimum,
    )?;

    writeln!(
        output,
        "Median:            {:.2} events/second",
        summary.median,
    )?;

    writeln!(
        output,
        "Maximum:           {:.2} events/second",
        summary.maximum,
    )?;

    writeln!(
        output,
        "Mean:              {:.2} events/second",
        summary.mean,
    )?;

    writeln!(
        output,
        "Aggregate:         {:.2} events/second",
        summary.aggregate,
    )?;

    writeln!(output, "Total events:      {}", summary.total_events,)?;

    writeln!(
        output,
        "Measured seconds: {:.6}",
        summary.total_elapsed.as_secs_f64(),
    )?;

    writeln!(output)?;

    writeln!(
        output,
        "Scope: simulator generation and local sink delivery.",
    )?;

    writeln!(
        output,
        "Initialization, validation, networking, and journal \
         persistence are excluded.",
    )?;

    writeln!(output, "Status: COMPLETED")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_sample_median() {
        let measurements = [
            Measurement {
                events: 100,
                elapsed: Duration::from_secs(1),
            },
            Measurement {
                events: 300,
                elapsed: Duration::from_secs(1),
            },
            Measurement {
                events: 200,
                elapsed: Duration::from_secs(1),
            },
        ];

        let summary = BenchmarkSummary::from_measurements(&measurements).unwrap();

        assert_eq!(summary.minimum, 100.0);
        assert_eq!(summary.median, 200.0);
        assert_eq!(summary.maximum, 300.0);
        assert_eq!(summary.mean, 200.0);
        assert_eq!(summary.aggregate, 200.0);
        assert_eq!(summary.total_events, 600);
    }

    #[test]
    fn even_sample_median() {
        let measurements = [
            Measurement {
                events: 100,
                elapsed: Duration::from_secs(1),
            },
            Measurement {
                events: 200,
                elapsed: Duration::from_secs(1),
            },
        ];

        let summary = BenchmarkSummary::from_measurements(&measurements).unwrap();

        assert_eq!(summary.median, 150.0);
    }

    #[test]
    fn aggregate_accounts_for_different_durations() {
        let measurements = [
            Measurement {
                events: 100,
                elapsed: Duration::from_secs(1),
            },
            Measurement {
                events: 100,
                elapsed: Duration::from_secs(3),
            },
        ];

        let summary = BenchmarkSummary::from_measurements(&measurements).unwrap();

        assert_eq!(summary.aggregate, 50.0);
        assert_eq!(summary.total_events, 200);
        assert_eq!(summary.total_elapsed, Duration::from_secs(4),);
    }

    #[test]
    fn empty_measurements_are_rejected() {
        assert!(BenchmarkSummary::from_measurements(&[]).is_err());
    }
}
