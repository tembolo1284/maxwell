//! Persistent recording of simulated market events.

use std::io::{self, Write};

use maxwell_core::{EventEnvelope, MonotonicTimestamp};

use maxwell_replay::{JournalMode, Recorder};

use maxwell_simulator::{SimulatorError, SimulatorResult};

use maxwell_simulator::simulator::{DeliveryOutcome, EventSink, Simulator};

use crate::commands::drive_simulator;
use crate::config::RecordOptions;
use crate::error::{CliError, CliResult};

use crate::output::{print_recording_stats, print_simulation_stats};

// ============================================================
// Recording sink
// ============================================================

struct RecordingSink<'a> {
    recorder: &'a mut Recorder,
    origin_ns: Option<u64>,
}

impl<'a> RecordingSink<'a> {
    fn new(recorder: &'a mut Recorder) -> Self {
        Self {
            recorder,
            origin_ns: None,
        }
    }
}

impl EventSink for RecordingSink<'_> {
    fn try_send(
        &mut self,
        event: &EventEnvelope,
        received_at: MonotonicTimestamp,
    ) -> SimulatorResult<DeliveryOutcome> {
        let timestamp = received_at.raw();

        let origin = self.origin_ns.unwrap_or(timestamp);

        let relative_timestamp = timestamp.checked_sub(origin).ok_or_else(|| {
            SimulatorError::Io(io::Error::other(
                "recording timestamp precedes recording origin",
            ))
        })?;

        self.recorder
            .record_at(*event, relative_timestamp)
            .map_err(|error| SimulatorError::Io(io::Error::other(error)))?;

        self.origin_ns = Some(origin);

        Ok(DeliveryOutcome::Accepted)
    }
}

// ============================================================
// Command
// ============================================================

pub fn run<W: Write>(options: RecordOptions, output: &mut W) -> CliResult<()> {
    let simulation_config = options.simulator_config()?;
    let writer_config = options.writer_config()?;

    let limits = options.simulation.runtime_limits(&simulation_config)?;

    let feed = options.simulation.feed();

    let mut simulator = Simulator::new(simulation_config, feed, limits)?;

    writeln!(output, "Maxwell recording")?;
    writeln!(output, "-----------------")?;

    writeln!(output, "Output:            {}", options.output.display(),)?;

    writeln!(
        output,
        "Scenario:          {}",
        options.simulation.scenario.as_str(),
    )?;

    writeln!(output, "Mode:              {:?}", options.mode)?;
    writeln!(output, "Feed:              {feed}")?;

    writeln!(output, "Seed:              {}", simulation_config.run.seed,)?;

    writeln!(
        output,
        "Requested events:  {}",
        simulation_config.run.event_count,
    )?;

    writeln!(output, "Durability:        {:?}", options.durability,)?;

    writeln!(output)?;

    let mut recorder =
        Recorder::create(&options.output, writer_config, feed, options.recording_id)?;

    let simulation_stats = {
        let mut sink = RecordingSink::new(&mut recorder);

        drive_simulator(&mut simulator, &mut sink)?
    };

    let recording_stats = recorder.finish(options.durable_finish)?;

    if recording_stats.records_written != simulation_stats.delivery.accepted {
        return Err(CliError::invalid_state(
            "recording",
            "recorded event count differs from accepted deliveries",
        ));
    }

    if options.mode == JournalMode::Authoritative
        && recording_stats.records_written != simulation_stats.generated()
    {
        return Err(CliError::invalid_state(
            "recording",
            "authoritative journal does not contain every \
             generated event",
        ));
    }

    print_simulation_stats(output, &simulation_stats)?;

    writeln!(output)?;

    print_recording_stats(output, &recording_stats)?;

    writeln!(output)?;
    writeln!(output, "Status: COMPLETED")?;

    Ok(())
}
