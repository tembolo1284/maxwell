//! Historical journal playback.

use std::io::Write;
use std::thread;
use std::time::Instant;

use maxwell_replay::{
    JournalRecord, ReplayEngine, ReplayError, ReplayResult, ReplaySink, ReplayStatus, SinkStatus,
};

use crate::config::ReplayOptions;
use crate::error::{CliError, CliResult};
use crate::output::print_replay_stats;

// ============================================================
// Replay sink
// ============================================================

struct OutputSink<'a, W> {
    output: &'a mut W,
    print_events: bool,
}

impl<W: Write> ReplaySink for OutputSink<'_, W> {
    fn deliver(&mut self, record: &JournalRecord) -> ReplayResult<SinkStatus> {
        if self.print_events {
            writeln!(
                self.output,
                "sequence={} timestamp_ns={} instrument={:?} \
                 kind={:?} event={:?}",
                record.sequence(),
                record.recording_timestamp_ns,
                record.event.instrument_id(),
                record.event.kind(),
                record.event,
            )
            .map_err(ReplayError::from)?;
        }

        Ok(SinkStatus::Accepted)
    }
}

// ============================================================
// Command
// ============================================================

pub fn run<W: Write>(options: ReplayOptions, output: &mut W) -> CliResult<()> {
    let config = options.replay_config()?;

    let initialization_started = Instant::now();

    let mut engine = ReplayEngine::open(&options.journal.path, config, options.filter)?;

    if let Some(sequence) = options.seek_sequence {
        engine.seek_sequence(sequence)?;
    }

    if let Some(timestamp) = options.seek_timestamp_ns {
        engine.seek_timestamp(timestamp)?;
    }

    if options.require_authoritative {
        engine.require_authoritative_stream()?;
    }

    let initialization_elapsed = initialization_started.elapsed();

    writeln!(output, "Maxwell replay")?;
    writeln!(output, "--------------")?;

    writeln!(
        output,
        "Journal:           {}",
        options.journal.path.display(),
    )?;

    writeln!(output, "Playback mode:     {:?}", options.playback_mode,)?;

    writeln!(
        output,
        "Authoritative:     {}",
        engine.preserves_authoritative_stream() && !options.journal.allow_incomplete_tail,
    )?;

    writeln!(output, "Index enabled:     {}", options.index_enabled,)?;

    writeln!(
        output,
        "Batch size:        {}",
        options.max_records_per_step,
    )?;

    writeln!(
        output,
        "Initialization:    {:.6} seconds",
        initialization_elapsed.as_secs_f64(),
    )?;

    writeln!(output)?;

    let started = Instant::now();

    {
        let mut sink = OutputSink {
            output,
            print_events: options.print_events,
        };

        loop {
            match engine.step(&mut sink)? {
                ReplayStatus::Progress => {}

                ReplayStatus::Waiting(duration) => {
                    thread::sleep(duration);
                }

                ReplayStatus::Finished => {
                    break;
                }

                ReplayStatus::Backpressured => {
                    return Err(CliError::invalid_state(
                        "replay",
                        "local replay sink unexpectedly \
                         reported backpressure",
                    ));
                }

                ReplayStatus::BackpressureBudgetExhausted => {
                    return Err(CliError::invalid_state(
                        "replay",
                        "replay backpressure budget exhausted",
                    ));
                }
            }
        }
    }

    let elapsed = started.elapsed();

    print_replay_stats(output, engine.stats(), elapsed)?;

    writeln!(output)?;

    if engine.incomplete_tail() {
        writeln!(output, "Status: COMPLETED VALID PREFIX - INCOMPLETE TAIL",)?;
    } else {
        writeln!(output, "Status: COMPLETED")?;
    }

    Ok(())
}
