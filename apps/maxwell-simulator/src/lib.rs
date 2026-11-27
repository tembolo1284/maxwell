//! Deterministic market data simulation for Maxwell.
//!
//! Maxwell Simulator generates authoritative market events,
//! maintains independent market state, and introduces
//! controlled delivery faults for testing market data systems.
//!
//! # Architecture
//!
//! The simulation separates authoritative event generation
//! from delivery behavior. Dropped, duplicated, or reordered
//! messages never alter the authoritative market state.
//!
//! # Design principles
//!
//! - Deterministic execution with reproducible random seeds.
//! - Fixed-point prices and quantities.
//! - Independent authoritative market state.
//! - Bounded history and delivery buffers.
//! - Explicit sequencing and fault injection.
//! - Single-owner mutable state.
//! - No unsafe code.
//!
//! # Components
//!
//! - `config`: simulation configuration and resource limits.
//! - `error`: unified simulation errors.
//! - `rng`: deterministic pseudo-random number generation.
//! - `clock`: virtual timestamps and real-time pacing.
//! - `market`: authoritative instrument and order state.
//! - `generator`: valid market event generation.
//! - `fault`: delivery fault injection.
//! - `history`: bounded authoritative event history.
//! - `snapshot`: authoritative market snapshots.
//! - `scenario`: predefined simulation workloads.
//! - `simulator`: integrated simulation runtime.
//! - `stats`: simulation and delivery statistics.

#![forbid(unsafe_code)]

pub mod clock;
pub mod config;
pub mod error;
pub mod fault;
pub mod generator;
pub mod history;
pub mod market;
pub mod rng;
pub mod scenario;
pub mod simulator;
pub mod snapshot;
pub mod stats;

pub use config::SimulatorConfig;
pub use error::{SimulatorError, SimulatorResult};
