//! Command-line argument parsing.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::str::FromStr;

use maxwell_core::InstrumentId;

use maxwell_replay::{DurabilityPolicy, JournalMode, PlaybackMode};

use maxwell_simulator::scenario::Scenario;

use crate::config::{
    BenchmarkOptions, JournalOptions, RecordOptions, ReplayOptions, SimulationOptions,
};

use crate::error::{CliError, CliResult};

pub const HELP: &str = r#"Maxwell - Market Data Infrastructure

USAGE:
    maxwell <command> [options]

COMMANDS:
    help                    Display this help
    version                 Display application version
    scenarios               List simulation scenarios
    simulate                Run a local market simulation
    record                  Record a simulation to a journal
    inspect <journal>       Display journal header metadata
    validate <journal>      Validate all journal records
    replay <journal>        Replay historical market events
    benchmark               Benchmark simulator throughput

SIMULATION OPTIONS:
    --scenario <name>       Predefined simulation scenario
    --events <count>        Override scenario event count
    --seed <number>         Override deterministic random seed
    --stream-id <number>    Upstream stream identifier
    --session-id <number>   Upstream session identifier

RECORD OPTIONS:
    All simulation options, plus:
    --output <path>         Required output journal path
    --raw                   Preserve raw delivery order
    --recording-id <id>     Journal recording identifier
    --flush-every <count>   Flush after every N records
    --sync-every <count>    Synchronize after every N records
    --durable               Synchronize when recording finishes

VALIDATE OPTIONS:
    --allow-incomplete-tail Accept an incomplete final record

REPLAY OPTIONS:
    --speed <mode>          unbounded, virtual, realtime, N or N/D
    --instrument <id>       Filter by instrument identifier
    --from-sequence <n>     Inclusive sequence filter
    --to-sequence <n>       Inclusive sequence filter
    --from-time <ns>        Inclusive recording timestamp filter
    --to-time <ns>          Inclusive recording timestamp filter
    --seek-sequence <n>     Seek to an exact sequence
    --seek-time <ns>        Seek to a recording timestamp
    --batch-size <count>    Maximum records processed per step
    --print-events          Print each delivered event
    --authoritative         Require an unfiltered authoritative stream
    --no-index              Disable sparse index construction
    --allow-incomplete-tail Accept an incomplete final record

BENCHMARK OPTIONS:
    All simulation options, plus:
    --iterations <count>    Number of measured iterations
    --warmup <count>        Number of unmeasured iterations

EXAMPLES:
    maxwell simulate --scenario smoke --events 10000
    maxwell record --output market.mxj --events 100000 --seed 42
    maxwell inspect market.mxj
    maxwell validate market.mxj
    maxwell replay market.mxj --speed 10
    maxwell benchmark --events 1000000 --iterations 5
"#;

#[derive(Debug, Clone)]
pub enum Command {
    Help,
    Version,
    Scenarios,
    Simulate(SimulationOptions),
    Record(RecordOptions),
    Inspect(JournalOptions),
    Validate(JournalOptions),
    Replay(ReplayOptions),
    Benchmark(BenchmarkOptions),
}

// ============================================================
// Argument cursor
// ============================================================

struct Arguments<I> {
    remaining: I,
    seen: HashSet<String>,
}

impl<I> Arguments<I>
where
    I: Iterator<Item = OsString>,
{
    fn new(remaining: I) -> Self {
        Self {
            remaining,
            seen: HashSet::new(),
        }
    }

    fn next_flag(&mut self) -> CliResult<Option<String>> {
        let Some(argument) = self.remaining.next() else {
            return Ok(None);
        };

        let flag = argument
            .into_string()
            .map_err(|_| CliError::usage("option names must be valid UTF-8"))?;

        if !flag.starts_with("--") {
            return Err(CliError::usage(format!("unexpected argument: {flag}",)));
        }

        if !self.seen.insert(flag.clone()) {
            return Err(CliError::usage(format!("duplicate option: {flag}",)));
        }

        Ok(Some(flag))
    }

    fn value(&mut self, option: &str) -> CliResult<OsString> {
        let value = self
            .remaining
            .next()
            .ok_or_else(|| CliError::usage(format!("missing value for {option}",)))?;

        if value.is_empty() || value.to_str().is_some_and(|text| text.starts_with("--")) {
            return Err(CliError::usage(format!("missing value for {option}",)));
        }

        Ok(value)
    }

    fn text(&mut self, option: &str) -> CliResult<String> {
        self.value(option)?
            .into_string()
            .map_err(|_| CliError::usage(format!("{option} requires a valid UTF-8 value",)))
    }

    fn number<T>(&mut self, option: &str) -> CliResult<T>
    where
        T: FromStr,
    {
        let value = self.text(option)?;

        value
            .parse()
            .map_err(|_| CliError::usage(format!("invalid numeric value for {option}: {value}",)))
    }

    fn path(&mut self, option: &str) -> CliResult<PathBuf> {
        Ok(PathBuf::from(self.value(option)?))
    }

    fn finish(&mut self) -> CliResult<()> {
        if let Some(argument) = self.remaining.next() {
            return Err(CliError::usage(format!(
                "unexpected argument: {}",
                argument.to_string_lossy(),
            )));
        }

        Ok(())
    }
}

// ============================================================
// Shared parsers
// ============================================================

fn unknown_option(option: &str) -> CliError {
    CliError::usage(format!("unknown option: {option}"))
}

fn parse_scenario(value: &str) -> CliResult<Scenario> {
    Scenario::ALL
        .into_iter()
        .find(|scenario| scenario.as_str() == value)
        .ok_or_else(|| CliError::usage(format!("unknown simulation scenario: {value}",)))
}

fn parse_speed(value: &str) -> CliResult<PlaybackMode> {
    match value {
        "unbounded" => Ok(PlaybackMode::Unbounded),
        "virtual" => Ok(PlaybackMode::Virtual),
        "realtime" => Ok(PlaybackMode::RealTime),

        _ => {
            let (numerator, denominator) = value.split_once('/').unwrap_or((value, "1"));

            let numerator: u32 = numerator
                .parse()
                .map_err(|_| CliError::usage(format!("invalid replay speed: {value}",)))?;

            let denominator: u32 = denominator
                .parse()
                .map_err(|_| CliError::usage(format!("invalid replay speed: {value}",)))?;

            if numerator == 0 || denominator == 0 {
                return Err(CliError::usage("replay speed must be positive"));
            }

            if numerator == denominator {
                Ok(PlaybackMode::RealTime)
            } else {
                Ok(PlaybackMode::Scaled {
                    numerator,
                    denominator,
                })
            }
        }
    }
}

fn parse_simulation_option<I>(
    option: &str,
    arguments: &mut Arguments<I>,
    config: &mut SimulationOptions,
) -> CliResult<bool>
where
    I: Iterator<Item = OsString>,
{
    match option {
        "--scenario" => {
            config.scenario = parse_scenario(&arguments.text(option)?)?;
        }

        "--events" => {
            let events = arguments.number::<u64>(option)?;

            if events == 0 {
                return Err(CliError::usage("event count must be positive"));
            }

            config.events = Some(events);
        }

        "--seed" => {
            config.seed = Some(arguments.number(option)?);
        }

        "--stream-id" => {
            config.stream_id = arguments.number(option)?;
        }

        "--session-id" => {
            config.session_id = arguments.number(option)?;
        }

        _ => return Ok(false),
    }

    Ok(true)
}

// ============================================================
// Command parsers
// ============================================================

fn parse_simulate<I>(arguments: &mut Arguments<I>) -> CliResult<Command>
where
    I: Iterator<Item = OsString>,
{
    let mut options = SimulationOptions::default();

    while let Some(option) = arguments.next_flag()? {
        if !parse_simulation_option(&option, arguments, &mut options)? {
            return Err(unknown_option(&option));
        }
    }

    Ok(Command::Simulate(options))
}

fn parse_record<I>(arguments: &mut Arguments<I>) -> CliResult<Command>
where
    I: Iterator<Item = OsString>,
{
    let mut simulation = SimulationOptions::default();

    let mut output = None;
    let mut mode = JournalMode::Authoritative;
    let mut durability = DurabilityPolicy::Buffered;
    let mut durable_finish = false;
    let mut recording_id = 1;

    while let Some(option) = arguments.next_flag()? {
        if parse_simulation_option(&option, arguments, &mut simulation)? {
            continue;
        }

        match option.as_str() {
            "--output" => {
                output = Some(arguments.path(&option)?);
            }

            "--raw" => {
                mode = JournalMode::RawCapture;
            }

            "--recording-id" => {
                recording_id = arguments.number(&option)?;
            }

            "--flush-every" | "--sync-every" => {
                if durability != DurabilityPolicy::Buffered {
                    return Err(CliError::usage(
                        "--flush-every and --sync-every \
                         are mutually exclusive",
                    ));
                }

                let interval = arguments.number::<u64>(&option)?;

                if interval == 0 {
                    return Err(CliError::usage("durability interval must be positive"));
                }

                durability = if option == "--flush-every" {
                    DurabilityPolicy::FlushEveryRecords(interval)
                } else {
                    DurabilityPolicy::SyncEveryRecords(interval)
                };
            }

            "--durable" => {
                durable_finish = true;
            }

            _ => return Err(unknown_option(&option)),
        }
    }

    let output = output.ok_or_else(|| CliError::usage("record requires --output <path>"))?;

    Ok(Command::Record(RecordOptions {
        simulation,
        output,
        mode,
        durability,
        durable_finish,
        recording_id,
    }))
}

fn parse_journal<I>(arguments: &mut Arguments<I>, validate: bool) -> CliResult<Command>
where
    I: Iterator<Item = OsString>,
{
    let path = arguments.path("journal")?;

    let mut options = JournalOptions {
        path,
        allow_incomplete_tail: false,
    };

    while let Some(option) = arguments.next_flag()? {
        match option.as_str() {
            "--allow-incomplete-tail" if validate => {
                options.allow_incomplete_tail = true;
            }

            _ => return Err(unknown_option(&option)),
        }
    }

    if validate {
        Ok(Command::Validate(options))
    } else {
        Ok(Command::Inspect(options))
    }
}

fn parse_replay<I>(arguments: &mut Arguments<I>) -> CliResult<Command>
where
    I: Iterator<Item = OsString>,
{
    let path = arguments.path("journal")?;

    let mut options = ReplayOptions::new(path);

    while let Some(option) = arguments.next_flag()? {
        match option.as_str() {
            "--speed" => {
                options.playback_mode = parse_speed(&arguments.text(&option)?)?;
            }

            "--instrument" => {
                let id = arguments.number::<u32>(&option)?;

                options.filter.instrument = Some(InstrumentId::new(id));
            }

            "--from-sequence" => {
                options.filter.first_sequence = Some(arguments.number(&option)?);
            }

            "--to-sequence" => {
                options.filter.last_sequence = Some(arguments.number(&option)?);
            }

            "--from-time" => {
                options.filter.first_timestamp_ns = Some(arguments.number(&option)?);
            }

            "--to-time" => {
                options.filter.last_timestamp_ns = Some(arguments.number(&option)?);
            }

            "--seek-sequence" => {
                options.seek_sequence = Some(arguments.number(&option)?);
            }

            "--seek-time" => {
                options.seek_timestamp_ns = Some(arguments.number(&option)?);
            }

            "--batch-size" => {
                options.max_records_per_step = arguments.number(&option)?;
            }

            "--print-events" => {
                options.print_events = true;
            }

            "--authoritative" => {
                options.require_authoritative = true;
            }

            "--no-index" => {
                options.index_enabled = false;
            }

            "--allow-incomplete-tail" => {
                options.journal.allow_incomplete_tail = true;
            }

            _ => return Err(unknown_option(&option)),
        }
    }

    options.replay_config()?;

    Ok(Command::Replay(options))
}

fn parse_benchmark<I>(arguments: &mut Arguments<I>) -> CliResult<Command>
where
    I: Iterator<Item = OsString>,
{
    let mut options = BenchmarkOptions::default();

    while let Some(option) = arguments.next_flag()? {
        if parse_simulation_option(&option, arguments, &mut options.simulation)? {
            continue;
        }

        match option.as_str() {
            "--iterations" => {
                options.iterations = arguments.number(&option)?;
            }

            "--warmup" => {
                options.warmup = arguments.number(&option)?;
            }

            _ => return Err(unknown_option(&option)),
        }
    }

    options.validate()?;

    Ok(Command::Benchmark(options))
}

// ============================================================
// Public entry point
// ============================================================

/// Parses arguments excluding the executable name.
pub fn parse_args<I, S>(arguments: I) -> CliResult<Command>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut remaining = arguments.into_iter().map(Into::into);

    let Some(command) = remaining.next() else {
        return Ok(Command::Help);
    };

    let command = command
        .into_string()
        .map_err(|_| CliError::usage("command must be valid UTF-8"))?;

    let mut arguments = Arguments::new(remaining);

    match command.as_str() {
        "help" | "--help" | "-h" => {
            arguments.finish()?;
            Ok(Command::Help)
        }

        "version" | "--version" | "-V" => {
            arguments.finish()?;
            Ok(Command::Version)
        }

        "scenarios" => {
            arguments.finish()?;
            Ok(Command::Scenarios)
        }

        "simulate" => parse_simulate(&mut arguments),

        "record" => parse_record(&mut arguments),

        "inspect" => parse_journal(&mut arguments, false),

        "validate" => parse_journal(&mut arguments, true),

        "replay" => parse_replay(&mut arguments),

        "benchmark" => parse_benchmark(&mut arguments),

        _ => Err(CliError::usage(format!("unknown command: {command}",))),
    }
}
