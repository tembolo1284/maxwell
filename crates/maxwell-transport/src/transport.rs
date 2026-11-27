//! Common transport interfaces and I/O outcomes.

use std::net::SocketAddr;

use crate::error::TransportResult;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    UdpUnicast = 0,
    UdpMulticast = 1,
    Tcp = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceivedDatagram {
    pub source: SocketAddr,
    pub length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// Number of bytes received.
    Data(usize),

    /// No data currently available.
    WouldBlock,

    /// TCP peer performed an orderly shutdown.
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Number of bytes accepted by the socket.
    Written(usize),

    /// Socket cannot currently accept more data.
    WouldBlock,
}

pub trait DatagramReceiver {
    /// Receives one complete datagram.
    ///
    /// None indicates that no datagram is available.
    /// A zero-length datagram is valid UDP data.
    ///
    /// Implementations must reject oversized datagrams
    /// rather than expose truncated data to the decoder.
    ///
    /// # Errors
    ///
    /// Returns an error if the datagram does not fit in `buffer`
    /// or the socket receive fails.
    fn receive(&mut self, buffer: &mut [u8]) -> TransportResult<Option<ReceivedDatagram>>;

    /// Returns the local socket address.
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system cannot report it.
    fn local_addr(&self) -> TransportResult<SocketAddr>;
}

pub trait DatagramSender {
    /// Sends one complete datagram.
    ///
    /// Datagram writes must not be resumed from an offset.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload exceeds the datagram limit
    /// or the socket send fails.
    fn send_to(&mut self, payload: &[u8], destination: SocketAddr)
    -> TransportResult<WriteOutcome>;
}
