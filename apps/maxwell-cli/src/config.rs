//! Command-specific configuration.

use std::path::PathBuf;

use maxwell_core::{FeedId, SessionId, StreamId};

use maxwell_replay::{
    DurabilityPolicy, JournalMode, PlaybackMode, ReaderConfig, ReplayConfig, ReplayFilter,
    TailPolicy, WriterConfig,
};

use maxwell_simulator::SimulatorConfig;
use maxwell_simulator::config::PacingMode;
use maxwell_simulator::scenario::Scenario;
use maxwell_simulator::simulator::RuntimeLimits;

use crate::error::{CliError, CliResult};

// ============================================================
// Simulation
// ============================================================

#[derive(Debug, Clone, Copy)]
pub struct SimulationOptions {
    pub scenario: Scenario,
    pub events: Option<u64>,
    pub seed: Option<u64>,
    pub stream_id: u32,
    pub session_id: u64,
}

impl Default for SimulationOptions {
    fn default() -> Self {
        Self {
            scenario: Scenario::Clean,
            events: None,
            seed: None,
            stream_id: 1,
            session_id: 1,
        }
    }
}

impl SimulationOptions {
    pub fn simulator_config(&self) -> CliResult<SimulatorConfig> {
        let mut config = match self.seed {
            Some(seed) => self.scenario.config_with_seed(seed)?,
            None => self.scenario.config()?,
        };

        if let Some(events) = self.events {
            config.run.event_count = events;
        }

        config.validate()?;

        Ok(config)
    }

    #[must_use]
    pub const fn feed(&self) -> FeedId {
        FeedId::new(
            StreamId::new(self.stream_id),
            SessionId::new(self.session_id),
        )
    }

    pub fn runtime_limits(&self, config: &SimulatorConfig) -> CliResult<RuntimeLimits> {
        let limits = RuntimeLimits {
            history_capacity: config.resources.history_capacity,
            delivery_capacity: config.resources.delivery_capacity,
            snapshot_capacity: config.resources.snapshot_capacity,
            ..RuntimeLimits::default()
        };

        limits.validate()?;

        Ok(limits)
    }
}

// ============================================================
// Journal inspection and validation
// ============================================================

#[derive(Debug, Clone)]
pub struct JournalOptions {
    pub path: PathBuf,
    pub allow_incomplete_tail: bool,
}

impl JournalOptions {
    #[must_use]
    pub fn reader_config(&self) -> ReaderConfig {
        ReaderConfig {
            tail_policy: if self.allow_incomplete_tail {
                TailPolicy::AllowIncompleteTail
            } else {
                TailPolicy::Strict
            },
            ..ReaderConfig::default()
        }
    }
}

// ============================================================
// Recording
// ============================================================

#[derive(Debug, Clone)]
pub struct RecordOptions {
    pub simulation: SimulationOptions,
    pub output: PathBuf,
    pub mode: JournalMode,
    pub durability: DurabilityPolicy,
    pub durable_finish: bool,
    pub recording_id: u64,
}

impl RecordOptions {
    pub fn writer_config(&self) -> CliResult<WriterConfig> {
        let mut config = WriterConfig::default();

        config.journal.mode = self.mode;
        config.durability = self.durability;

        config.validate()?;

        Ok(config)
    }

    pub fn simulator_config(&self) -> CliResult<SimulatorConfig> {
        let mut config = self.simulation.simulator_config()?;

        if self.mode == JournalMode::Authoritative && config.faults.is_enabled() {
            return Err(CliError::usage(
                "authoritative recording requires a fault-free \
                 scenario; use --raw to capture delivery faults",
            ));
        }

        // Recording uses deterministic synthetic reception
        // timestamps rather than wall-clock scheduling.
        config.pacing.mode = PacingMode::Virtual;

        config.validate()?;

        Ok(config)
    }
}

// ============================================================
// Replay
// ============================================================

#[derive(Debug, Clone)]
pub struct ReplayOptions {
    pub journal: JournalOptions,
    pub playback_mode: PlaybackMode,
    pub filter: ReplayFilter,
    pub seek_sequence: Option<u64>,
    pub seek_timestamp_ns: Option<u64>,
    pub print_events: bool,
    pub require_authoritative: bool,
    pub index_enabled: bool,
    pub max_records_per_step: usize,
}

impl ReplayOptions {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        let defaults = ReplayConfig::default();

        Self {
            journal: JournalOptions {
                path,
                allow_incomplete_tail: false,
            },
            playback_mode: PlaybackMode::Unbounded,
            filter: ReplayFilter::default(),
            seek_sequence: None,
            seek_timestamp_ns: None,
            print_events: false,
            require_authoritative: false,
            index_enabled: defaults.index.enabled,
            max_records_per_step: defaults.max_records_per_step,
        }
    }

    pub fn replay_config(&self) -> CliResult<ReplayConfig> {
        if self.seek_sequence.is_some() && self.seek_timestamp_ns.is_some() {
            return Err(CliError::usage(
                "sequence and timestamp seeking are mutually exclusive",
            ));
        }

        if !self.index_enabled && (self.seek_sequence.is_some() || self.seek_timestamp_ns.is_some())
        {
            return Err(CliError::usage("seeking requires the journal index"));
        }

        if self.require_authoritative
            && (!self.filter.is_unrestricted()
                || self.seek_sequence.is_some()
                || self.seek_timestamp_ns.is_some()
                || self.journal.allow_incomplete_tail)
        {
            return Err(CliError::usage(
                "authoritative replay cannot use filtering, \
                 seeking, or incomplete-tail recovery",
            ));
        }

        self.filter.validate()?;

        let mut config = ReplayConfig {
            reader: self.journal.reader_config(),
            playback_mode: self.playback_mode,
            max_records_per_step: self.max_records_per_step,
            ..ReplayConfig::default()
        };

        config.index.enabled = self.index_enabled;

        config.validate()?;

        Ok(config)
    }
}

// ============================================================
// Benchmark
// ============================================================

pub const DEFAULT_BENCHMARK_ITERATIONS: u32 = 5;
pub const MAX_BENCHMARK_ITERATIONS: u32 = 100;

#[derive(Debug, Clone, Copy)]
pub struct BenchmarkOptions {
    pub simulation: SimulationOptions,
    pub iterations: u32,
    pub warmup: u32,
}

impl Default for BenchmarkOptions {
    fn default() -> Self {
        Self {
            simulation: SimulationOptions {
                scenario: Scenario::HighThroughput,
                ..SimulationOptions::default()
            },
            iterations: DEFAULT_BENCHMARK_ITERATIONS,
            warmup: 1,
        }
    }
}

impl BenchmarkOptions {
    pub fn validate(&self) -> CliResult<()> {
        if self.iterations == 0 || self.iterations > MAX_BENCHMARK_ITERATIONS {
            return Err(CliError::usage(format!(
                "benchmark iterations must be between 1 and \
                 {MAX_BENCHMARK_ITERATIONS}",
            )));
        }

        if self.warmup > MAX_BENCHMARK_ITERATIONS {
            return Err(CliError::usage(format!(
                "benchmark warmup must not exceed \
                 {MAX_BENCHMARK_ITERATIONS}",
            )));
        }

        Ok(())
    }

    pub fn simulator_config(&self) -> CliResult<SimulatorConfig> {
        self.validate()?;

        let mut config = self.simulation.simulator_config()?;

        if config.faults.is_enabled() {
            return Err(CliError::usage(
                "simulator throughput benchmarking requires \
                 a fault-free scenario",
            ));
        }

        config.pacing.mode = PacingMode::Unbounded;

        config.validate()?;

        Ok(config)
    }
}
