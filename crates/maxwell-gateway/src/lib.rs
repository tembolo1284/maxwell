//! Maxwell market data gateway.
//!
//! Provides deterministic session management, subscription
//! lifecycle management, snapshot synchronization, heartbeat
//! processing and bounded downstream delivery.
//!
//! Network I/O and wire encoding are delegated to the
//! application through the GatewaySink abstraction.

#![forbid(unsafe_code)]

pub mod config;
pub mod error;
pub mod gateway;
pub mod message;
pub mod session;
pub mod sink;
pub mod stats;

pub use config::GatewayConfig;
pub use error::GatewayError;
pub use gateway::{Gateway, ServiceReport};
pub use message::{ClientRequest, ServerMessage};
pub use session::{Session, SessionId, SessionState};
pub use sink::GatewaySink;
pub use stats::GatewayStats;
