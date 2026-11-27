#![forbid(unsafe_code)]

//! Deterministic market data recovery for Maxwell.
//!
//! Responsibilities:
//!
//! - Detect missing event sequences.
//! - Buffer out-of-order live events.
//! - Coordinate bounded retransmission requests.
//! - Escalate failed recovery to snapshots.
//! - Validate feed and session identity.
//! - Resume contiguous event application.
//! - Track recovery deadlines and statistics.
//!
//! Networking, protocol serialization, and concrete
//! order book mutation remain external responsibilities.
//!
//! Each manager has a single owner and operates on
//! exactly one feed session at a time.

pub mod buffer;
pub mod config;
pub mod error;
pub mod manager;
pub mod policy;
pub mod replay;
pub mod request;
pub mod retransmit;
pub mod snapshot;
pub mod state;
pub mod stats;
pub mod timeout;

pub use buffer::{InsertOutcome, RecoveryBuffer};
pub use config::RecoveryConfig;
pub use error::{RecoveryError, RecoveryResult};
pub use manager::{IngestOutcome, RecoveryManager};
pub use policy::{RecoveryDecision, RecoveryPolicy};
pub use replay::ReplayCursor;
pub use request::{RecoveryRequest, RequestId, RetransmissionRequest, SnapshotRequest};
pub use retransmit::RetransmissionAttempt;
pub use snapshot::SnapshotWatermark;
pub use state::{RecoveryReason, RecoveryState};
pub use stats::RecoveryStats;
pub use timeout::RecoveryDeadline;
