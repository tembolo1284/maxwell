//! Maxwell market data client.
//!
//! Provides deterministic client lifecycle management,
//! snapshot synchronization, transport sequence validation,
//! incremental processing and heartbeat monitoring.
//!
//! Socket ownership, wire decoding and authoritative
//! market state are delegated to application components.

#![forbid(unsafe_code)]

pub mod client;
pub mod config;
pub mod error;
pub mod handler;
pub mod message;
pub mod sequence;
pub mod state;
pub mod stats;

pub use client::{Client, ReceiveOutcome};
pub use config::ClientConfig;
pub use error::ClientError;
pub use handler::MarketDataHandler;
pub use message::{InboundFrame, InboundMessage};
pub use sequence::{SequenceStatus, SequenceTracker};
pub use state::{ClientState, RecoveryReason};
pub use stats::ClientStats;
