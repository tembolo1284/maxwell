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
pub mod dedup;
pub mod error;
pub mod gap;
pub mod metrics;
pub mod reorder;
pub mod sequencer;
pub mod state;
pub mod stream;
pub mod tracker;

// ============================================================
// Configuration
// ============================================================

pub use config::SequencerConfig;

// ============================================================
// Errors
// ============================================================

pub use error::{SequencerError, SequencerResult};

// ============================================================
// State and sequence tracking
// ============================================================

pub use dedup::SequenceDisposition;
pub use gap::SequenceGap;
pub use state::SequencerState;
pub use tracker::SequenceTracker;

// ============================================================
// Buffering
// ============================================================

pub use reorder::ReorderBuffer;

// ============================================================
// Stream and sequencer
// ============================================================

pub use stream::{IngestOutcome, StreamSequencer};

pub use sequencer::Sequencer;

// ============================================================
// Metrics
// ============================================================

pub use metrics::SequencerMetrics;
