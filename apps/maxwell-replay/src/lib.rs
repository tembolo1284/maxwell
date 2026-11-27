#![forbid(unsafe_code)]

//! Persistent market data journaling and deterministic replay.
//!
//! Maxwell Replay separates historical market data persistence
//! from event production and consumption.
//!
//! # Journal semantics
//!
//! Authoritative journals contain contiguous events belonging
//! to a single upstream feed.
//!
//! Raw capture journals preserve arrival order, including
//! duplicates, gaps, and reordered events.
//!
//! Neither journal mode modifies upstream sequence numbers.
//!
//! # Reconstruction
//!
//! Consumers must establish an appropriate market state
//! baseline before reconstructing an order book.
//!
//! # Persistence
//!
//! Buffered writes, flushed writes, and synchronized writes
//! represent distinct persistence guarantees.
//!
//! # Resource management
//!
//! Journal processing uses bounded record sizes, buffered I/O,
//! bounded sparse indexing, and bounded replay processing steps.

pub mod clock;
pub mod config;
pub mod error;
pub mod filter;
pub mod format;
pub mod index;
pub mod reader;
pub mod record;
pub mod recorder;
pub mod replay;
pub mod stats;
pub mod writer;

pub use clock::ReplayClock;

pub use config::{
    DurabilityPolicy, IndexConfig, JournalConfig, JournalMode, PlaybackMode, ReaderConfig,
    ReplayConfig, TailPolicy, WriterConfig,
};

pub use error::{ReplayError, ReplayResult};

pub use filter::ReplayFilter;

pub use format::JournalHeader;

pub use index::{IndexEntry, JournalIndex};

pub use reader::JournalReader;

pub use record::JournalRecord;

pub use recorder::Recorder;

pub use replay::{ReplayEngine, ReplaySink, ReplayStatus, SinkStatus};

pub use stats::{RecordingStats, ReplayStats};

pub use writer::JournalWriter;
