//! CLI parsing and end-to-end operational tests.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use std::sync::atomic::{AtomicU64, Ordering};

use maxwell_cli::{Command, execute, parse_args};

use maxwell_replay::{JournalMode, JournalReader, PlaybackMode, ReaderConfig};

use maxwell_simulator::scenario::Scenario;

// ============================================================
// Test resources
// ============================================================

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct TestJournal {
    path: PathBuf,
}

impl TestJournal {
    fn new() -> Self {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);

        let path =
            std::env::temp_dir().join(format!("maxwell-cli-{}-{id}.mxj", std::process::id(),));

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestJournal {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn run_command(command: Command) -> String {
    let mut output = Vec::new();

    execute(command, &mut output).unwrap();

    String::from_utf8(output).unwrap()
}

fn run_arguments(arguments: &[&str]) -> String {
    let command = parse_args(arguments.iter().copied()).unwrap();

    run_command(command)
}

fn journal_arguments(command: &str, journal: &TestJournal, extra: &[&str]) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from(command),
        journal.path().as_os_str().to_owned(),
    ];

    arguments.extend(extra.iter().map(OsString::from));

    arguments
}

fn record_journal(journal: &TestJournal, events: u64, scenario: &str, raw: bool) -> String {
    let mut arguments = vec![
        OsString::from("record"),
        OsString::from("--output"),
        journal.path().as_os_str().to_owned(),
        OsString::from("--scenario"),
        OsString::from(scenario),
        OsString::from("--events"),
        OsString::from(events.to_string()),
        OsString::from("--seed"),
        OsString::from("42"),
    ];

    if raw {
        arguments.push(OsString::from("--raw"));
    }

    run_command(parse_args(arguments).unwrap())
}

// ============================================================
// Argument parsing
// ============================================================

#[test]
fn empty_arguments_display_help() {
    let command = parse_args(std::iter::empty::<String>()).unwrap();

    assert!(matches!(command, Command::Help));
}

#[test]
fn version_command_is_recognized() {
    assert!(matches!(
        parse_args(["--version"]).unwrap(),
        Command::Version,
    ));
}

#[test]
fn simulation_options_are_parsed() {
    let command = parse_args([
        "simulate",
        "--scenario",
        "smoke",
        "--events",
        "10000",
        "--seed",
        "42",
        "--stream-id",
        "7",
        "--session-id",
        "9",
    ])
    .unwrap();

    let Command::Simulate(options) = command else {
        panic!("expected simulation command");
    };

    assert_eq!(options.scenario, Scenario::Smoke);
    assert_eq!(options.events, Some(10_000));
    assert_eq!(options.seed, Some(42));
    assert_eq!(options.stream_id, 7);
    assert_eq!(options.session_id, 9);
}

#[test]
fn unknown_command_is_rejected() {
    let error = parse_args(["unknown"]).unwrap_err();

    assert_eq!(error.exit_code(), 2);
}

#[test]
fn unknown_option_is_rejected() {
    assert!(parse_args(["simulate", "--unknown", "123",]).is_err());
}

#[test]
fn duplicate_option_is_rejected() {
    assert!(parse_args(["simulate", "--events", "100", "--events", "200",]).is_err());
}

#[test]
fn missing_option_value_is_rejected() {
    assert!(parse_args(["simulate", "--events",]).is_err());
}

#[test]
fn missing_value_before_another_option_is_rejected() {
    assert!(parse_args(["simulate", "--events", "--seed", "42",]).is_err());
}

#[test]
fn invalid_event_count_is_rejected() {
    for value in ["0", "-1", "abc"] {
        assert!(parse_args(["simulate", "--events", value,]).is_err());
    }
}

#[test]
fn invalid_scenario_is_rejected() {
    assert!(parse_args(["simulate", "--scenario", "unknown",]).is_err());
}

#[test]
fn recording_requires_output_path() {
    assert!(parse_args(["record", "--events", "100",]).is_err());
}

#[test]
fn conflicting_durability_options_are_rejected() {
    assert!(
        parse_args([
            "record",
            "--output",
            "market.mxj",
            "--flush-every",
            "100",
            "--sync-every",
            "100",
        ])
        .is_err()
    );
}

#[test]
fn scaled_replay_speed_is_parsed() {
    let command = parse_args(["replay", "market.mxj", "--speed", "5/2"]).unwrap();

    let Command::Replay(options) = command else {
        panic!("expected replay command");
    };

    assert_eq!(
        options.playback_mode,
        PlaybackMode::Scaled {
            numerator: 5,
            denominator: 2,
        },
    );
}

#[test]
fn zero_replay_speed_is_rejected() {
    for speed in ["0", "0/1", "1/0"] {
        assert!(parse_args(["replay", "market.mxj", "--speed", speed,]).is_err());
    }
}

#[test]
fn conflicting_seek_options_are_rejected() {
    assert!(
        parse_args([
            "replay",
            "market.mxj",
            "--seek-sequence",
            "100",
            "--seek-time",
            "200",
        ])
        .is_err()
    );
}

#[test]
fn seek_without_index_is_rejected() {
    assert!(
        parse_args([
            "replay",
            "market.mxj",
            "--no-index",
            "--seek-sequence",
            "100",
        ])
        .is_err()
    );
}

#[test]
fn authoritative_replay_cannot_be_filtered() {
    assert!(
        parse_args([
            "replay",
            "market.mxj",
            "--authoritative",
            "--instrument",
            "1",
        ])
        .is_err()
    );
}

#[test]
fn authoritative_replay_rejects_incomplete_tail_policy() {
    assert!(
        parse_args([
            "replay",
            "market.mxj",
            "--authoritative",
            "--allow-incomplete-tail",
        ])
        .is_err()
    );
}

#[test]
fn zero_benchmark_iterations_are_rejected() {
    assert!(parse_args(["benchmark", "--iterations", "0",]).is_err());
}

// ============================================================
// Operational commands
// ============================================================

#[test]
fn help_lists_operational_commands() {
    let output = run_arguments(&["help"]);

    for command in [
        "simulate",
        "record",
        "inspect",
        "validate",
        "replay",
        "benchmark",
    ] {
        assert!(output.contains(command));
    }
}

#[test]
fn scenario_command_lists_predefined_scenarios() {
    let output = run_arguments(&["scenarios"]);

    for scenario in Scenario::ALL {
        assert!(output.contains(scenario.as_str()),);
    }
}

#[test]
fn smoke_simulation_completes() {
    let output = run_arguments(&[
        "simulate",
        "--scenario",
        "smoke",
        "--events",
        "1000",
        "--seed",
        "42",
    ]);

    assert!(output.contains("Events generated:  1000"),);

    assert!(output.contains("Status: COMPLETED"),);
}

#[test]
fn authoritative_recording_creates_valid_journal() {
    let journal = TestJournal::new();

    let output = record_journal(&journal, 1_000, "smoke", false);

    assert!(journal.path().exists());

    assert!(output.contains("Records written:   1000"),);

    let mut reader = JournalReader::open(journal.path(), ReaderConfig::default()).unwrap();

    assert_eq!(reader.header().mode, JournalMode::Authoritative,);

    let mut expected_sequence = 1_u64;

    while let Some(record) = reader.next_record().unwrap() {
        assert_eq!(record.sequence(), expected_sequence,);

        expected_sequence += 1;
    }

    assert_eq!(expected_sequence, 1_001);
}

#[test]
fn inspect_reports_header_metadata() {
    let journal = TestJournal::new();

    record_journal(&journal, 100, "smoke", false);

    let arguments = journal_arguments("inspect", &journal, &[]);

    let output = run_command(parse_args(arguments).unwrap());

    assert!(output.contains("Journal metadata"));
    assert!(output.contains("Authoritative"));

    assert!(output.contains("record contents have not been scanned",),);
}

#[test]
fn validation_scans_all_records() {
    let journal = TestJournal::new();

    record_journal(&journal, 1_000, "smoke", false);

    let arguments = journal_arguments("validate", &journal, &[]);

    let output = run_command(parse_args(arguments).unwrap());

    assert!(output.contains("Records validated: 1000"),);

    assert!(output.contains("Status: VALID"));
}

#[test]
fn replay_delivers_every_record() {
    let journal = TestJournal::new();

    record_journal(&journal, 1_000, "smoke", false);

    let arguments = journal_arguments(
        "replay",
        &journal,
        &["--speed", "virtual", "--authoritative"],
    );

    let output = run_command(parse_args(arguments).unwrap());

    assert!(output.contains("Records delivered: 1000"),);

    assert!(output.contains("Records filtered:  0"),);

    assert!(output.contains("Status: COMPLETED"),);
}

#[test]
fn replay_sequence_filter_is_inclusive() {
    let journal = TestJournal::new();

    record_journal(&journal, 1_000, "smoke", false);

    let arguments = journal_arguments(
        "replay",
        &journal,
        &["--from-sequence", "101", "--to-sequence", "200"],
    );

    let output = run_command(parse_args(arguments).unwrap());

    assert!(output.contains("Records delivered: 100"),);

    assert!(output.contains("Records filtered:  900"),);
}

#[test]
fn replay_can_print_individual_events() {
    let journal = TestJournal::new();

    record_journal(&journal, 10, "smoke", false);

    let arguments = journal_arguments("replay", &journal, &["--print-events"]);

    let output = run_command(parse_args(arguments).unwrap());

    assert!(output.contains("sequence=1 "));
    assert!(output.contains("sequence=10 "));
}

#[test]
fn raw_capture_supports_fault_scenarios() {
    let journal = TestJournal::new();

    record_journal(&journal, 1_000, "duplicates", true);

    let reader = JournalReader::open(journal.path(), ReaderConfig::default()).unwrap();

    assert_eq!(reader.header().mode, JournalMode::RawCapture,);
}

#[test]
fn authoritative_recording_rejects_fault_scenario() {
    let journal = TestJournal::new();

    let arguments = vec![
        OsString::from("record"),
        OsString::from("--output"),
        journal.path().as_os_str().to_owned(),
        OsString::from("--scenario"),
        OsString::from("packet-loss"),
        OsString::from("--events"),
        OsString::from("1000"),
    ];

    let command = parse_args(arguments).unwrap();

    let mut output = Vec::new();

    assert!(execute(command, &mut output).is_err(),);

    assert!(!journal.path().exists());
}

#[test]
fn authoritative_replay_rejects_raw_capture() {
    let journal = TestJournal::new();

    record_journal(&journal, 1_000, "duplicates", true);

    let arguments = journal_arguments("replay", &journal, &["--authoritative"]);

    let command = parse_args(arguments).unwrap();

    let mut output = Vec::new();

    assert!(execute(command, &mut output).is_err(),);
}

#[test]
fn existing_journal_is_not_overwritten() {
    let journal = TestJournal::new();

    fs::write(journal.path(), b"existing data").unwrap();

    let arguments = vec![
        OsString::from("record"),
        OsString::from("--output"),
        journal.path().as_os_str().to_owned(),
        OsString::from("--scenario"),
        OsString::from("smoke"),
        OsString::from("--events"),
        OsString::from("100"),
    ];

    let command = parse_args(arguments).unwrap();

    let mut output = Vec::new();

    assert!(execute(command, &mut output).is_err(),);

    assert_eq!(fs::read(journal.path()).unwrap(), b"existing data",);
}

#[test]
fn strict_validation_rejects_truncated_journal() {
    let journal = TestJournal::new();

    record_journal(&journal, 100, "smoke", false);

    let mut bytes = fs::read(journal.path()).unwrap();

    bytes.pop();

    fs::write(journal.path(), bytes).unwrap();

    let arguments = journal_arguments("validate", &journal, &[]);

    let command = parse_args(arguments).unwrap();

    let mut output = Vec::new();

    assert!(execute(command, &mut output).is_err(),);
}

#[test]
fn tolerant_validation_reports_incomplete_tail() {
    let journal = TestJournal::new();

    record_journal(&journal, 100, "smoke", false);

    let mut bytes = fs::read(journal.path()).unwrap();

    bytes.pop();

    fs::write(journal.path(), bytes).unwrap();

    let arguments = journal_arguments("validate", &journal, &["--allow-incomplete-tail"]);

    let output = run_command(parse_args(arguments).unwrap());

    assert!(output.contains("VALID PREFIX - INCOMPLETE TAIL",),);
}

#[test]
fn benchmark_produces_summary() {
    let output = run_arguments(&[
        "benchmark",
        "--scenario",
        "smoke",
        "--events",
        "100",
        "--iterations",
        "2",
        "--warmup",
        "0",
    ]);

    assert!(output.contains("Benchmark summary"));
    assert!(output.contains("Median:"));
    assert!(output.contains("Aggregate:"));

    assert!(output.contains("Total events:      200"),);
}

// ============================================================
// Executable behavior
// ============================================================

#[test]
fn executable_version_succeeds() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_maxwell"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.starts_with("maxwell "));
}

#[test]
fn executable_usage_error_returns_two() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_maxwell"))
        .arg("unknown-command")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));

    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(stderr.contains("unknown command"));
}

#[test]
fn executable_operational_error_returns_one() {
    let journal = TestJournal::new();

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_maxwell"))
        .arg("validate")
        .arg(journal.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
}
