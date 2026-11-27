//! Unified operational command-line interface for Maxwell.
//!
//! The CLI provides local simulation, historical recording,
//! journal inspection, validation, replay, and benchmarking.
//!
//! Domain logic remains within the underlying Maxwell crates.
//! This application is responsible for configuration,
//! orchestration, argument parsing, and operational reporting.
//!
//! Remote connectivity and deployment management are outside
//! the scope of the initial local CLI.

#![forbid(unsafe_code)]

pub mod args;
pub mod commands;
pub mod config;
pub mod error;
pub mod output;

pub use args::{Command, parse_args};
pub use commands::execute;
pub use error::{CliError, CliResult};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
