
#![forbid(unsafe_code)]

//! Deterministic market data order book reconstruction.
//!
//! Maxwell reconstructs exchange state from authoritative,
//! sequenced market data events.
//!
//! Design principles:
//!
//! - Single-writer ownership.
//! - Fixed-point integer arithmetic.
//! - Bounded memory consumption.
//! - Preallocated price ladders.
//! - No steady-state heap allocations.
//! - Explicit arithmetic failure handling.
//! - Deterministic snapshot generation.
//! - No matching or order execution logic.
//! - No transport or networking dependencies.
//!
//! Sequence continuity is enforced upstream by
//! maxwell-sequencer. Individual instrument books may
//! receive nonconsecutive stream sequence numbers.

pub mod config;
pub mod error;
pub mod level;
pub mod ladder;
pub mod order;
pub mod registry;
pub mod book;
pub mod update;
pub mod snapshot;
pub mod manager;
pub mod depth;

pub use config::BookConfig;

pub use error::{
    BookError,
    BookResult,
};

pub use level::PriceLevel;
pub use ladder::PriceLadder;
pub use order::RestingOrder;
pub use registry::OrderRegistry;

pub use book::{
    OrderBook,
    BookState,
};

pub use update::{
    BookUpdate,
    BookUpdateKind,
};

pub use snapshot::BookSnapshot;
pub use manager::BookManager;

pub use depth::{
    Level1,
    Level2,
    Level3,
};

