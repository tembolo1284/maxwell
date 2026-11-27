//! Maxwell market data server.
//!
//! This application serves as the composition root for the
//! Maxwell market data platform, integrating upstream ingestion,
//! sequencing, recovery, order book reconstruction, publication,
//! downstream distribution, and operational instrumentation.
//!
//! # Architecture
//!
//! The server follows a deterministic, single-owner processing
//! model. Mutable processing components are owned by the server
//! runtime rather than shared across independent worker threads.
//!
//! # Design principles
//!
//! - Deterministic event processing.
//! - Explicit component ownership.
//! - Bounded memory consumption.
//! - Explicit backpressure and error handling.
//! - Consistent snapshot and publication watermarks.
//! - Independent upstream and downstream sequence domains.
//! - Correctness before performance optimization.

#![forbid(unsafe_code)]

pub mod config;
pub mod error;
pub mod state;
pub mod server;
pub mod pipeline;
pub mod upstream;
pub mod downstream;
pub mod recovery;
pub mod snapshot;
pub mod metrics;
