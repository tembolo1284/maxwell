
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

pub mod config;
pub mod error;
pub mod transport;
pub mod socket;
pub mod buffer;
pub mod stats;
pub mod udp;
pub mod tcp;

pub use config::{
    SocketConfig,
    UdpConfig,
    TcpConfig,
    FramingConfig,
};

pub use error::{
    TransportError,
    TransportResult,
};

pub use transport::{
    TransportKind,
    DatagramReceiver,
    DatagramSender,
    ReceivedDatagram,
    ReadOutcome,
    WriteOutcome,
};

pub use buffer::ReceiveBuffer;
pub use stats::TransportStats;

pub use socket::SocketBufferSizes;

pub use udp::{
    UdpUnicast,
    UdpMulticast,
    MulticastConfig,
};

pub use tcp::{
    TcpConnection,
    TcpAcceptor,
    FrameDecoder,
    FrameEncoder,
    FrameStatus,
};

