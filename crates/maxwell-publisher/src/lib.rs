//! Deterministic market data publication for Maxwell.
//!
//! Provides bounded subscriber queues, subscription filtering,
//! deterministic fan-out and explicit backpressure handling.
//!
//! The publisher is designed around single-owner mutable state.
//! Threading and network delivery are managed by downstream crates.

#![forbid(unsafe_code)]

pub mod config;
pub mod error;
pub mod event;
pub mod filter;
pub mod publisher;
pub mod queue;
pub mod sink;
pub mod stats;
pub mod subscription;

pub use config::{BackpressurePolicy, PublisherConfig};
pub use error::PublisherError;
pub use event::{EventKind, Publication};
pub use filter::SubscriptionFilter;
pub use publisher::{PublishReport, Publisher};
pub use queue::BoundedQueue;
pub use sink::{DrainReport, PublicationSink};
pub use stats::PublisherStats;
pub use subscription::{SubscriberId, SubscriberState, Subscription};
