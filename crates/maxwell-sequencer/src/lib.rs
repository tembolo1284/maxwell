
#![forbid(unsafe_code)]

//! Deterministic market data sequencing for Maxwell.
//!
//! Responsible for:
//!
//! - Per-stream sequence tracking.
//! - Feed session isolation.
//! - Duplicate suppression.
//! - Gap detection.
//! - Bounded out-of-order buffering.
//! - Contiguous event delivery.
//! - Recovery coordination.
//!
//! Design principles:
//!
//! - Single-writer ownership per feed.
//! - No allocation during steady-state event processing.
//! - No locks or atomic operations on the sequencing path.
//! - Explicit sequence exhaustion handling.
//! - No silent sequence advancement across gaps.
//! - No implicit session transitions.
//! - No dependency on networking or wire formats.
//!
//! The caller owns transport, recovery orchestration,
//! thread scheduling, and downstream event delivery.

pub mod config;
pub mod error;
pub mod state;
pub mod gap;
pub mod dedup;
pub mod reorder;
pub mod tracker;
pub mod stream;
pub mod sequencer;
pub mod metrics;

// ============================================================
// Configuration
// ============================================================

pub use config::SequencerConfig;

// ============================================================
// Errors
// ============================================================

pub use error::{
    SequencerError,
    SequencerResult,
};

// ============================================================
// State and sequence tracking
// ============================================================

pub use state::SequencerState;
pub use tracker::SequenceTracker;
pub use gap::SequenceGap;
pub use dedup::SequenceDisposition;

// ============================================================
// Buffering
// ============================================================

pub use reorder::ReorderBuffer;

// ============================================================
// Stream and sequencer
// ============================================================

pub use stream::{
    StreamSequencer,
    IngestOutcome,
};

pub use sequencer::Sequencer;

// ============================================================
// Metrics
// ============================================================

pub use metrics::SequencerMetrics;

