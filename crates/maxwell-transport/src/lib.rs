#![forbid(unsafe_code)]

//! Low-latency networking primitives for Maxwell.
//!
//! This crate provides transport-level functionality
//! independently of market data protocol semantics.
//!
//! Design principles:
//!
//! - Nonblocking networking by default.
//! - Explicit backpressure handling.
//! - Reusable receive and transmit buffers.
//! - Bounded memory consumption.
//! - No implicit background threads.
//! - No asynchronous runtime dependency.
//! - No unsafe code.
//! - No market data decoding.
//! - No automatic retransmission.
//!
//! UDP preserves datagram boundaries.
//! TCP requires explicit application-level framing.
//!
//! Sequence validation and recovery are responsibilities
//! of higher-level Maxwell components.

pub mod buffer;
pub mod config;
pub mod error;
pub mod socket;
pub mod stats;
pub mod tcp;
pub mod transport;
pub mod udp;

pub use config::{FramingConfig, SocketConfig, TcpConfig, UdpConfig};

pub use error::{TransportError, TransportResult};

pub use transport::{
    DatagramReceiver, DatagramSender, ReadOutcome, ReceivedDatagram, TransportKind, WriteOutcome,
};

pub use buffer::ReceiveBuffer;
pub use stats::TransportStats;

pub use socket::SocketBufferSizes;

pub use udp::{MulticastConfig, UdpMulticast, UdpUnicast};

pub use tcp::{FrameDecoder, FrameEncoder, FrameStatus, TcpAcceptor, TcpConnection};
