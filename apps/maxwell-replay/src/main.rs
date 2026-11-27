#![forbid(unsafe_code)]

//! Standalone journal inspection and playback application.

use std::env;
use std::process::ExitCode;
use std::thread;

use maxwell_replay::{
    JournalReader, JournalRecord, PlaybackMode, ReaderConfig, ReplayConfig, ReplayEngine,
    ReplayError, ReplayFilter, ReplayResult, ReplayStatus, SinkStatus,
};

fn usage() {
    eprintln!(
        "Usage:\n\
         maxwell-replay inspect <journal>\n\
         maxwell-replay validate <journal>\n\
         maxwell-replay replay <journal> \
         [unbounded|virtual|realtime|N/D]"
    );
}

fn invalid_argument(reason: &'static str) -> ReplayError {
    ReplayError::InvalidConfig {
        field: "command line",
        reason,
    }
}

fn parse_mode(value: Option<&str>) -> ReplayResult<PlaybackMode> {
    match value {
        None | Some("unbounded") => Ok(PlaybackMode::Unbounded),
        Some("virtual") => Ok(PlaybackMode::Virtual),
        Some("realtime") => Ok(PlaybackMode::RealTime),

        Some(value) => {
            let (numerator, denominator) = value
                .split_once('/')
                .ok_or(invalid_argument("invalid playback mode"))?;

            let numerator = numerator
                .parse::<u32>()
                .map_err(|_| invalid_argument("invalid speed numerator"))?;

            let denominator = denominator
                .parse::<u32>()
                .map_err(|_| invalid_argument("invalid speed denominator"))?;

            let mode = PlaybackMode::Scaled {
                numerator,
                denominator,
            };

            mode.validate()?;

            Ok(mode)
        }
    }
}

fn inspect(path: &str) -> ReplayResult<()> {
    let reader = JournalReader::open(path, ReaderConfig::default())?;

    let header = reader.header();

    println!("Journal: {path}");
    println!("Mode: {:?}", header.mode);
    println!("Feed: {:?}", header.feed);
    println!("Recording ID: {}", header.recording_id);
    println!("Created at (ns): {}", header.created_at_ns);
    println!("Maximum payload: {}", header.max_payload_bytes);

    Ok(())
}

fn validate(path: &str) -> ReplayResult<()> {
    let mut reader = JournalReader::open(path, ReaderConfig::default())?;

    let mut first_sequence = None;
    let mut last_sequence = None;

    while let Some(record) = reader.next_record()? {
        first_sequence.get_or_insert(record.sequence());
        last_sequence = Some(record.sequence());
    }

    println!("Journal validation successful");
    println!("Records: {}", reader.records_read());
    println!("Bytes: {}", reader.offset());
    println!("First sequence: {first_sequence:?}");
    println!("Last sequence: {last_sequence:?}");

    Ok(())
}

fn replay(path: &str, mode: PlaybackMode) -> ReplayResult<()> {
    let config = ReplayConfig {
        playback_mode: mode,
        ..ReplayConfig::default()
    };

    let mut engine = ReplayEngine::open(path, config, ReplayFilter::default())?;

    let mut sink = |_record: &JournalRecord| Ok(SinkStatus::Accepted);

    loop {
        match engine.step(&mut sink)? {
            ReplayStatus::Progress => {}

            ReplayStatus::Waiting(duration) => {
                thread::sleep(duration);
            }

            ReplayStatus::Finished => break,

            ReplayStatus::Backpressured | ReplayStatus::BackpressureBudgetExhausted => {
                return Err(ReplayError::InvalidState {
                    operation: "standalone playback",
                    state: "unexpected backpressure",
                });
            }
        }
    }

    println!("Replay completed");
    println!("{:#?}", engine.stats());

    Ok(())
}

fn run() -> ReplayResult<()> {
    let mut arguments = env::args().skip(1);

    let command = arguments
        .next()
        .ok_or(invalid_argument("missing command"))?;

    let path = arguments
        .next()
        .ok_or(invalid_argument("missing journal path"))?;

    let mode = arguments.next();

    if arguments.next().is_some() {
        return Err(invalid_argument("too many arguments"));
    }

    match command.as_str() {
        "inspect" if mode.is_none() => inspect(&path),

        "validate" if mode.is_none() => validate(&path),

        "replay" => replay(&path, parse_mode(mode.as_deref())?),

        _ => Err(invalid_argument("unknown command")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,

        Err(error) => {
            eprintln!("Error: {error}");
            usage();

            ExitCode::FAILURE
        }
    }
}
