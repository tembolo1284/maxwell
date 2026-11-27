//! Command dispatch and shared orchestration.

pub mod benchmark;
pub mod inspect;
pub mod record;
pub mod replay;
pub mod simulate;
pub mod validate;

use std::io::Write;

use maxwell_simulator::simulator::{EventSink, Simulator};

use maxwell_simulator::stats::StatsReport;

use maxwell_simulator::scenario::Scenario;

use crate::VERSION;
use crate::args::{Command, HELP};
use crate::error::{CliError, CliResult};

pub fn execute<W: Write>(command: Command, output: &mut W) -> CliResult<()> {
    match command {
        Command::Help => {
            write!(output, "{HELP}")?;
            Ok(())
        }

        Command::Version => {
            writeln!(output, "maxwell {VERSION}")?;
            Ok(())
        }

        Command::Scenarios => print_scenarios(output),

        Command::Simulate(options) => simulate::run(options, output),

        Command::Record(options) => record::run(options, output),

        Command::Inspect(options) => inspect::run(options, output),

        Command::Validate(options) => validate::run(options, output),

        Command::Replay(options) => replay::run(options, output),

        Command::Benchmark(options) => benchmark::run(options, output),
    }
}

fn print_scenarios<W: Write>(output: &mut W) -> CliResult<()> {
    writeln!(output, "Available simulation scenarios")?;
    writeln!(output, "------------------------------")?;

    for scenario in Scenario::ALL {
        writeln!(
            output,
            "{:<20} {}",
            scenario.as_str(),
            scenario.description(),
        )?;
    }

    Ok(())
}

/// Drives a local simulation until completion.
///
/// The CLI uses sinks that accept every successful delivery.
/// Unexpected backpressure is treated as an operational
/// failure instead of spinning indefinitely.
pub(crate) fn drive_simulator<S: EventSink>(
    simulator: &mut Simulator,
    sink: &mut S,
) -> CliResult<StatsReport> {
    let report = simulator.run_until_blocked(sink)?;

    if !report.is_completed() {
        return Err(CliError::invalid_state(
            "simulation",
            format!("simulation stopped before completion: {report:?}",),
        ));
    }

    simulator.validate_consistency()?;

    let stats = simulator.stats();

    if !stats.is_fully_accounted() {
        return Err(CliError::invalid_state(
            "simulation",
            "generated and delivered events are not fully accounted",
        ));
    }

    Ok(stats)
}
