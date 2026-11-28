//! Maxwell market data server executable.
//!
//! The executable provides configuration validation,
//! deterministic runtime initialization checks, and the `run`
//! command, which consumes a native Maxwell UDP feed.
//!
//! Downstream network sessions and production feed adapters
//! will be introduced after the upstream runtime is validated.

#![forbid(unsafe_code)]

use std::env;
use std::io;
use std::process::ExitCode;

use maxwell_core::{FeedId, MonotonicTimestamp, SessionId, StreamId};

use maxwell_server::config::ServerConfig;
use maxwell_server::error::{ServerError, ServerResult};
use maxwell_server::runtime::{RUN_USAGE, RunOptions};
use maxwell_server::server::Server;

// ============================================================
// Commands
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    CheckConfig,
    SelfCheck,
    Metrics,
    Run,
}

impl Command {
    fn parse() -> ServerResult<Self> {
        let mut args = env::args().skip(1);

        let command = match args.next().as_deref() {
            None | Some("--help") | Some("-h") => Self::Help,

            Some("--version") | Some("-V") => Self::Version,

            Some("--check-config") => Self::CheckConfig,

            Some("--self-check") => Self::SelfCheck,

            Some("--metrics") => Self::Metrics,
            // The run command parses its own options.
            Some("run") => return Ok(Self::Run),

            Some(_) => {
                return Err(ServerError::InvalidConfig("unknown command-line argument"));
            }
        };

        if args.next().is_some() {
            return Err(ServerError::InvalidConfig(
                "unexpected additional command-line argument",
            ));
        }

        Ok(command)
    }
}

// ============================================================
// Output
// ============================================================

fn print_help() {
    println!(
        "Maxwell Market Data Server {}\n\
         \n\
         Usage:\n\
         \x20 maxwell-server [COMMAND]\n\
         \n\
         Commands:\n\
         \x20 --help, -h       Display this help message\n\
         \x20 --version, -V    Display the application version\n\
         \x20 --check-config   Validate default configuration\n\
         \x20 --self-check     Verify runtime initialization\n\
         \x20 --metrics        Export initial Prometheus metrics\n\
         \x20 run [OPTIONS]    Consume a native Maxwell UDP feed\n\
         \n\
         {RUN_USAGE}\n\
         \n\
         Downstream network sessions are not enabled.",
        env!("CARGO_PKG_VERSION"),
    );
}

fn print_version() {
    println!("maxwell-server {}", env!("CARGO_PKG_VERSION"),);
}

// ============================================================
// Runtime construction
// ============================================================

fn default_feed() -> FeedId {
    FeedId::new(StreamId::new(1), SessionId::new(1))
}

fn create_server() -> ServerResult<Server> {
    Server::new(ServerConfig::default(), default_feed())
}

// ============================================================
// Command implementations
// ============================================================

fn check_config() -> ServerResult<()> {
    let config = ServerConfig::default();

    config.validate()?;

    println!("Maxwell server configuration is valid.");

    Ok(())
}

fn self_check() -> ServerResult<()> {
    let mut server = create_server()?;

    println!("Maxwell server initialization check");
    println!("Initial lifecycle: {}", server.state());

    // This is an isolated synthetic feed. No external
    // upstream market is associated with this baseline.
    server.start_empty(MonotonicTimestamp::new(0))?;

    let status = server.status();

    println!("Lifecycle: {}", status.lifecycle);
    println!("Synchronized: {}", status.synchronized);
    println!("Publication watermark: {}", status.publication_watermark,);
    println!("Next upstream sequence: {}", status.next_expected.raw(),);
    println!("Registered instruments: {}", status.registered_instruments,);
    println!("Connected sessions: {}", status.connected_sessions,);

    server.poll(MonotonicTimestamp::new(1))?;

    server.begin_shutdown(MonotonicTimestamp::new(2))?;

    server.finish_shutdown(MonotonicTimestamp::new(3))?;

    println!("Final lifecycle: {}", server.state());
    println!("Runtime initialization check passed.");

    Ok(())
}

fn export_metrics() -> ServerResult<()> {
    let mut server = create_server()?;

    server.start_empty(MonotonicTimestamp::new(0))?;

    let stdout = io::stdout();
    let mut writer = stdout.lock();

    server.metrics().write_prometheus(&mut writer)?;

    Ok(())
}

/// Consumes a UDP feed until a stop condition holds.
fn run_feed() -> ServerResult<()> {
    let options = RunOptions::parse(env::args().skip(2))?;
    let mut runtime = options.bind()?;

    println!(
        "Maxwell server listening on {} for feed {}",
        runtime.local_addr()?,
        options.runtime.feed,
    );

    let stdout = io::stdout();
    let summary = runtime.run(&mut stdout.lock())?;
    let stats = summary.stats;

    println!("Stop reason: {}", summary.reason.as_str());
    println!("Elapsed: {} ms", summary.elapsed.as_millis());
    println!("Synchronized: {}", summary.synchronized);
    println!("Next upstream sequence: {}", summary.next_expected.raw());
    println!("Publication watermark: {}", summary.publication_watermark);
    println!("Datagrams received: {}", stats.datagrams);
    println!("Events consumed: {}", stats.events);
    println!("Duplicate events: {}", stats.duplicates);
    println!("Buffered events: {}", stats.buffered);
    println!("Heartbeats: {}", stats.heartbeats);
    println!("Rejected datagrams: {}", stats.rejected);
    println!("Foreign datagrams: {}", stats.foreign);
    println!("Blocked events: {}", stats.blocked);
    println!("Retransmission requests: {}", stats.retransmit_requests);
    println!("Resynchronizations: {}", stats.resynchronizations);

    Ok(())
}

// ============================================================
// Application entry point
// ============================================================

fn run() -> ServerResult<()> {
    match Command::parse()? {
        Command::Help => {
            print_help();

            Ok(())
        }

        Command::Version => {
            print_version();

            Ok(())
        }

        Command::CheckConfig => check_config(),

        Command::SelfCheck => self_check(),

        Command::Metrics => export_metrics(),

        Command::Run => run_feed(),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,

        Err(error) => {
            eprintln!("maxwell-server: {error}");

            ExitCode::FAILURE
        }
    }
}
