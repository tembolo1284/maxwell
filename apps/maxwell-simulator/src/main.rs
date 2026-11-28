//! Maxwell simulator command-line executable.

#![forbid(unsafe_code)]

use std::env;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;

use maxwell_core::{FeedId, SessionId, StreamId};

use maxwell_simulator::config::PacingMode;
use maxwell_simulator::network::{NetworkConfig, NetworkRunner};
use maxwell_simulator::scenario::Scenario;
use maxwell_simulator::simulator::{NullSink, RuntimeLimits, Simulator};
use maxwell_simulator::{SimulatorConfig, SimulatorError, SimulatorResult};

/// Default generation batch when delivering over UDP.
const NETWORK_GENERATION_BUDGET: usize = 8;

// ============================================================
// Command-line arguments
// ============================================================

#[derive(Debug, Default)]
struct Arguments {
    scenario: Scenario,
    seed: Option<u64>,
    events: Option<u64>,
    instruments: Option<usize>,
    max_orders: Option<usize>,
    history_capacity: Option<usize>,
    delivery_capacity: Option<usize>,
    snapshot_capacity: Option<usize>,
    generation_budget: Option<usize>,
    delivery_budget: Option<usize>,
    events_per_second: Option<u64>,
    pacing: Option<PacingMode>,
    target: Option<SocketAddr>,
    bind: Option<SocketAddr>,
    heartbeat_ms: Option<u64>,
    linger_ms: Option<u64>,
    help: bool,
    version: bool,
    list_scenarios: bool,
    check_config: bool,
}

impl Arguments {
    fn parse(mut args: impl Iterator<Item = String>) -> SimulatorResult<Self> {
        let mut result = Self::default();

        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--help" | "-h" => {
                    result.help = true;
                }

                "--version" | "-V" => {
                    result.version = true;
                }

                "--list-scenarios" => {
                    result.list_scenarios = true;
                }

                "--check-config" => {
                    result.check_config = true;
                }

                "--scenario" => {
                    result.scenario = next_value(&mut args)?.parse()?;
                }

                "--seed" => {
                    result.seed = Some(parse_value(&mut args)?);
                }

                "--events" => {
                    result.events = Some(parse_value(&mut args)?);
                }

                "--instruments" => {
                    result.instruments = Some(parse_value(&mut args)?);
                }

                "--max-orders" => {
                    result.max_orders = Some(parse_value(&mut args)?);
                }

                "--history-capacity" => {
                    result.history_capacity = Some(parse_value(&mut args)?);
                }

                "--delivery-capacity" => {
                    result.delivery_capacity = Some(parse_value(&mut args)?);
                }

                "--snapshot-capacity" => {
                    result.snapshot_capacity = Some(parse_value(&mut args)?);
                }

                "--generation-budget" => {
                    result.generation_budget = Some(parse_value(&mut args)?);
                }

                "--delivery-budget" => {
                    result.delivery_budget = Some(parse_value(&mut args)?);
                }

                "--rate" => {
                    result.events_per_second = Some(parse_value(&mut args)?);
                }

                "--pacing" => {
                    let value = next_value(&mut args)?;

                    result.pacing = Some(match value.as_str() {
                        "virtual" => PacingMode::Virtual,
                        "unbounded" => PacingMode::Unbounded,
                        "real-time" => PacingMode::RealTime,

                        _ => {
                            return Err(SimulatorError::InvalidArgument("invalid pacing mode"));
                        }
                    });
                }

                "--target" => {
                    result.target = Some(parse_value(&mut args)?);
                }

                "--bind" => {
                    result.bind = Some(parse_value(&mut args)?);
                }

                "--heartbeat-ms" => {
                    result.heartbeat_ms = Some(parse_value(&mut args)?);
                }

                "--linger-ms" => {
                    result.linger_ms = Some(parse_value(&mut args)?);
                }

                _ => {
                    eprintln!("Unknown argument: {argument}");

                    return Err(SimulatorError::InvalidArgument(
                        "unrecognized command-line argument",
                    ));
                }
            }
        }

        Ok(result)
    }

    // --------------------------------------------------------
    // Configuration construction
    // --------------------------------------------------------

    fn configuration(&self) -> SimulatorResult<(SimulatorConfig, RuntimeLimits)> {
        let mut config = self.scenario.config()?;

        if let Some(seed) = self.seed {
            config.run.seed = seed;
        }

        if let Some(events) = self.events {
            config.run.event_count = events;
        }

        if let Some(instruments) = self.instruments {
            config.market.instrument_count = instruments;
        }

        if let Some(max_orders) = self.max_orders {
            config.market.max_orders_per_instrument = max_orders;
        }

        if let Some(capacity) = self.history_capacity {
            config.resources.history_capacity = capacity;
        }

        if let Some(capacity) = self.delivery_capacity {
            config.resources.delivery_capacity = capacity;
        }

        if let Some(capacity) = self.snapshot_capacity {
            config.resources.snapshot_capacity = capacity;
        }

        if let Some(pacing) = self.pacing {
            config.pacing.mode = pacing;
        }

        if let Some(rate) = self.events_per_second {
            if rate == 0 {
                return Err(SimulatorError::InvalidArgument(
                    "event rate must be positive",
                ));
            }

            config.pacing.events_per_second = rate;

            config.pacing.event_interval_ns = (1_000_000_000 / rate).max(1);
        }

        let mut limits = RuntimeLimits {
            history_capacity: config.resources.history_capacity,

            delivery_capacity: config.resources.delivery_capacity,

            snapshot_capacity: config.resources.snapshot_capacity,

            ..RuntimeLimits::default()
        };

        if let Some(budget) = self.generation_budget {
            limits.generation_budget = budget;
        } else if self.target.is_some() {
            // Events are sent after each generation batch. A
            // small batch avoids bursts on the network.
            limits.generation_budget = NETWORK_GENERATION_BUDGET;
        }

        if let Some(budget) = self.delivery_budget {
            limits.delivery_budget = budget;
        }

        config.validate()?;
        limits.validate()?;

        Ok((config, limits))
    }
}

// ============================================================
// Argument helpers
// ============================================================

impl Arguments {
    /// Builds the network delivery configuration, if a
    /// consumer address was supplied.
    fn network(&self) -> SimulatorResult<Option<NetworkConfig>> {
        let Some(target) = self.target else {
            if self.bind.is_some() || self.heartbeat_ms.is_some() || self.linger_ms.is_some() {
                return Err(SimulatorError::InvalidArgument(
                    "network options require --target",
                ));
            }

            return Ok(None);
        };

        let bind = match self.bind {
            Some(bind) => bind,
            None if target.is_ipv4() => SocketAddr::from(([0, 0, 0, 0], 0)),
            None => SocketAddr::from(([0u16; 8], 0)),
        };

        let mut network = NetworkConfig::new(bind, target);

        if let Some(interval) = self.heartbeat_ms {
            network.heartbeat_interval = Duration::from_millis(interval);
        }

        if let Some(linger) = self.linger_ms {
            network.linger = Duration::from_millis(linger);
        }

        network.validate()?;

        Ok(Some(network))
    }
}

/// Delivers the simulation over UDP and reports the outcome.
fn run_network(
    simulator: &mut Simulator,
    network: NetworkConfig,
    feed: FeedId,
) -> SimulatorResult<()> {
    let mut runner = NetworkRunner::bind(network, feed)?;

    println!("Producer socket:      {}", runner.local_addr()?);
    println!();

    let stats = runner.run(simulator)?;

    println!("Network delivery");
    println!("  Events sent:            {}", stats.events_sent);
    println!("  Sends retried:          {}", stats.send_blocked);
    println!("  Heartbeats sent:        {}", stats.heartbeats_sent);
    println!("  Requests received:      {}", stats.requests_received);
    println!("  Requests unavailable:   {}", stats.requests_unavailable);
    println!("  Events retransmitted:   {}", stats.events_retransmitted);
    println!("  Rejected datagrams:     {}", stats.rejected);
    println!("  Consumer complete:      {}", stats.consumer_complete);
    println!();

    Ok(())
}

fn next_value(args: &mut impl Iterator<Item = String>) -> SimulatorResult<String> {
    args.next().ok_or(SimulatorError::InvalidArgument(
        "missing command-line argument value",
    ))
}

fn parse_value<T: FromStr>(args: &mut impl Iterator<Item = String>) -> SimulatorResult<T> {
    let value = next_value(args)?;

    value
        .parse()
        .map_err(|_| SimulatorError::InvalidArgument("invalid command-line argument value"))
}

// ============================================================
// Help and scenario listing
// ============================================================

fn print_help() {
    println!(
        "\
Maxwell Simulator {}

Deterministic market data generation and fault injection.

USAGE:
    maxwell-simulator [OPTIONS]

OPTIONS:
    -h, --help
        Display this help message.

    -V, --version
        Display the executable version.

    --list-scenarios
        List available predefined workloads.

    --check-config
        Validate configuration without running a simulation.

    --scenario <NAME>
        Select a predefined workload. Default: clean.

    --seed <N>
        Override the deterministic random seed.

    --events <N>
        Number of authoritative events to generate.

    --instruments <N>
        Number of simulated instruments.

    --max-orders <N>
        Maximum resting orders per instrument.

    --history-capacity <N>
        Maximum retained authoritative events.

    --delivery-capacity <N>
        Maximum queued delivery messages.

    --snapshot-capacity <N>
        Maximum authoritative snapshot entries.

    --generation-budget <N>
        Maximum generated events per service iteration.
        Default with --target: 8.

    --delivery-budget <N>
        Maximum delivered messages per service iteration.

    --pacing <MODE>
        virtual, unbounded, or real-time.

    --rate <N>
        Target events per second and virtual timestamp rate.

    --target <ADDR>
        Send events as UDP datagrams to this consumer address.
        Without it, events go to an in-process discard sink.

    --bind <ADDR>
        Local UDP address for sending and for retransmission
        requests. Default: an ephemeral port on all interfaces.

    --heartbeat-ms <N>
        Interval between watermark heartbeats. Default: 50.

    --linger-ms <N>
        Time to wait for further retransmission requests after
        the last event. Default: 2000.

EXAMPLES:
    maxwell-simulator --scenario smoke
    maxwell-simulator --scenario mixed-faults --seed 123
    maxwell-simulator --scenario high-throughput
    maxwell-simulator --events 10000 --pacing real-time --rate 50000
    maxwell-simulator --scenario smoke --pacing real-time --rate 2000 \\
        --target 127.0.0.1:9100 --bind 127.0.0.1:9101

Without --target, reported throughput measures simulator
execution, not network or end-to-end Maxwell server
performance. Use real-time pacing with --target, because an
unpaced sender overruns the consumer's socket buffer.",
        env!("CARGO_PKG_VERSION"),
    );
}

fn print_scenarios() {
    println!("Available Maxwell simulation scenarios:\n");

    for scenario in Scenario::ALL {
        println!("  {:<20} {}", scenario.as_str(), scenario.description(),);
    }
}

// ============================================================
// Configuration reporting
// ============================================================

fn print_configuration(
    scenario: Scenario,
    config: &SimulatorConfig,
    limits: RuntimeLimits,
    target: Option<SocketAddr>,
) {
    println!("Maxwell Simulator {}", env!("CARGO_PKG_VERSION"));
    println!("================================");

    println!("Scenario:             {scenario}");
    println!("Seed:                 {}", config.run.seed);
    println!("Events:               {}", config.run.event_count);
    println!("Instruments:          {}", config.market.instrument_count);

    println!(
        "Orders/instrument:    {}",
        config.market.max_orders_per_instrument,
    );

    println!("Pacing:               {:?}", config.pacing.mode);
    println!("Target rate:          {}", config.pacing.events_per_second);

    println!("Drop probability:     {} bps", config.faults.drop_bps);
    println!("Duplicate probability:{} bps", config.faults.duplicate_bps);
    println!("Reorder probability:  {} bps", config.faults.reorder_bps);

    println!("History capacity:     {}", limits.history_capacity);
    println!("Delivery capacity:    {}", limits.delivery_capacity);
    println!("Snapshot capacity:    {}", limits.snapshot_capacity);
    println!("Generation budget:    {}", limits.generation_budget);
    println!("Delivery budget:      {}", limits.delivery_budget);

    match target {
        Some(target) => println!("Delivery sink:        UDP {target}"),
        None => println!("Delivery sink:        NullSink"),
    }
    println!();
}

// ============================================================
// Execution
// ============================================================

fn run() -> SimulatorResult<()> {
    let arguments = Arguments::parse(env::args().skip(1))?;

    if arguments.help {
        print_help();

        return Ok(());
    }

    if arguments.version {
        println!("maxwell-simulator {}", env!("CARGO_PKG_VERSION"),);

        return Ok(());
    }

    if arguments.list_scenarios {
        print_scenarios();

        return Ok(());
    }

    let (config, limits) = arguments.configuration()?;

    if arguments.check_config {
        println!("Simulator configuration is valid.");

        return Ok(());
    }

    print_configuration(arguments.scenario, &config, limits, arguments.target);

    let feed = FeedId::new(StreamId::new(1), SessionId::new(1));

    let mut simulator = Simulator::new(config, feed, limits)?;

    if let Some(network) = arguments.network()? {
        run_network(&mut simulator, network, feed)?;
    } else {
        let mut sink = NullSink;

        let report = simulator.run_until_blocked(&mut sink)?;

        if !report.is_completed() {
            return Err(SimulatorError::InvariantViolation(
                "null-sink simulation did not complete",
            ));
        }
    }

    simulator.validate_consistency()?;

    let stats = simulator.stats();

    if !stats.is_fully_accounted() {
        return Err(SimulatorError::InvariantViolation(
            "final simulation accounting mismatch",
        ));
    }

    println!("{stats}");
    println!();
    println!("Final upstream watermark: {}", simulator.watermark().raw());
    println!("Simulation completed successfully.");

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,

        Err(error) => {
            eprintln!("maxwell-simulator: {error}");

            ExitCode::FAILURE
        }
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> SimulatorResult<Arguments> {
        Arguments::parse(args.iter().map(|value| (*value).to_owned()))
    }

    #[test]
    fn default_arguments_select_clean_scenario() {
        let arguments = parse(&[]).unwrap();

        assert_eq!(arguments.scenario, Scenario::Clean);
    }

    #[test]
    fn scenario_and_seed_are_configurable() {
        let arguments =
            parse(&["--scenario", "smoke", "--seed", "123", "--events", "500"]).unwrap();

        let (config, _) = arguments.configuration().unwrap();

        assert_eq!(config.run.seed, 123);
        assert_eq!(config.run.event_count, 500);
        assert_eq!(config.market.instrument_count, 2);
    }

    #[test]
    fn missing_argument_value_is_rejected() {
        assert!(parse(&["--events"]).is_err());
    }

    #[test]
    fn invalid_numeric_value_is_rejected() {
        assert!(parse(&["--events", "invalid"]).is_err());
    }

    #[test]
    fn invalid_scenario_is_rejected() {
        assert!(parse(&["--scenario", "invalid"]).is_err());
    }

    #[test]
    fn invalid_pacing_mode_is_rejected() {
        assert!(parse(&["--pacing", "invalid"]).is_err());
    }

    #[test]
    fn unknown_argument_is_rejected() {
        assert!(parse(&["--unknown"]).is_err());
    }

    #[test]
    fn zero_event_rate_is_rejected() {
        let arguments = parse(&["--rate", "0"]).unwrap();

        assert!(arguments.configuration().is_err());
    }

    #[test]
    fn rate_updates_virtual_timestamp_interval() {
        let arguments = parse(&["--pacing", "real-time", "--rate", "100000"]).unwrap();

        let (config, _) = arguments.configuration().unwrap();

        assert_eq!(config.pacing.mode, PacingMode::RealTime);
        assert_eq!(config.pacing.events_per_second, 100_000);
        assert_eq!(config.pacing.event_interval_ns, 10_000);
    }

    #[test]
    fn resource_overrides_are_consistent() {
        let arguments = parse(&[
            "--history-capacity",
            "1024",
            "--delivery-capacity",
            "512",
            "--snapshot-capacity",
            "32768",
        ])
        .unwrap();

        let (config, limits) = arguments.configuration().unwrap();

        assert_eq!(config.resources.history_capacity, limits.history_capacity,);

        assert_eq!(config.resources.delivery_capacity, limits.delivery_capacity,);

        assert_eq!(config.resources.snapshot_capacity, limits.snapshot_capacity,);
    }

    #[test]
    fn processing_budgets_are_configurable() {
        let arguments = parse(&["--generation-budget", "32", "--delivery-budget", "64"]).unwrap();

        let (_, limits) = arguments.configuration().unwrap();

        assert_eq!(limits.generation_budget, 32);
        assert_eq!(limits.delivery_budget, 64);
    }
}
