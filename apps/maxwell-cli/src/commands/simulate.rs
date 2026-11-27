//! Local deterministic market simulation.

use std::io::Write;

use maxwell_simulator::config::PacingMode;

use maxwell_simulator::simulator::{NullSink, Simulator};

use crate::commands::drive_simulator;
use crate::config::SimulationOptions;
use crate::error::CliResult;
use crate::output::print_simulation_stats;

pub fn run<W: Write>(options: SimulationOptions, output: &mut W) -> CliResult<()> {
    let mut config = options.simulator_config()?;

    // Local CLI simulations do not wait on wall-clock
    // pacing. Preserve the scenario's synthetic timestamps.
    if config.pacing.mode == PacingMode::RealTime {
        config.pacing.mode = PacingMode::Virtual;
    }

    let limits = options.runtime_limits(&config)?;

    writeln!(output, "Maxwell simulation")?;
    writeln!(output, "------------------")?;

    writeln!(output, "Scenario:          {}", options.scenario.as_str(),)?;

    writeln!(output, "Seed:              {}", config.run.seed)?;
    writeln!(output, "Requested events:  {}", config.run.event_count)?;

    writeln!(
        output,
        "Instruments:       {}",
        config.market.instrument_count,
    )?;

    writeln!(output, "Fault injection:   {}", config.faults.is_enabled(),)?;

    writeln!(output, "Feed:              {}", options.feed())?;
    writeln!(output)?;

    let mut simulator = Simulator::new(config, options.feed(), limits)?;

    let mut sink = NullSink;

    let stats = drive_simulator(&mut simulator, &mut sink)?;

    print_simulation_stats(output, &stats)?;

    writeln!(output)?;
    writeln!(output, "Status: COMPLETED")?;

    Ok(())
}
