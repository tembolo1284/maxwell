//! Persistent market data journaling and deterministic replay for Maxwell.
//!
//! Maxwell Replay provides the infrastructure required to record, persist,
//! validate, inspect, and replay historical market data.
//!
//! # Architecture
//!
//! The application separates journal persistence from market data production
//! and consumption. Producers supply market events to a recorder, while
//! consumers receive historical events through the replay engine.
//!
//! # Journal modes
//!
//! Two journal modes are supported:
//!
//! - Authoritative journals preserve a contiguous upstream event sequence.
//! - Raw capture journals preserve arrival order, including duplicates,
//!   sequence gaps, and reordered events.
//!
//! Neither journal mode silently modifies upstream sequence numbers.
//!
//! # Reconstruction
//!
//! A journal does not necessarily contain a complete market state baseline.
//! Consumers reconstructing an order book must establish a compatible
//! initial state before applying historical events.
//!
//! # Resource management
//!
//! Journal processing uses bounded record sizes, buffered sequential I/O,
//! and bounded sparse indexing. Journals are not required to fit in memory.
//!
//! # Durability
//!
//! Buffered writes, flushed writes, and synchronized writes represent
//! distinct persistence guarantees.
//!
//! # Safety
//!
//! This crate does not permit unsafe Rust.

#![forbid(unsafe_code)]

pub mod config;
pub mod error;

pub use config::{
    DurabilityPolicy,
    IndexConfig,
    JournalConfig,
    JournalMode,
    PlaybackMode,
    ReaderConfig,
    ReplayConfig,
    TailPolicy,
    WriterConfig,
};

pub use error::{
    ReplayError,
    ReplayResult,
};
