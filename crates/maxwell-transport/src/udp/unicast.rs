
//! UDP unicast transport.
//!
//! Suitable for direct market data delivery between
//! exchange components and AWS EC2 instances.

use std::net::{
    SocketAddr,
    UdpSocket,
};

use socket2::{
    SockAddr,
    SockRef,
};

use crate::buffer::ReceiveBuffer;
use crate::config::UdpConfig;
use crate::error::TransportResult;
use crate::socket::{self, SocketBufferSizes};
use crate::stats::TransportStats;

use crate::transport::{
    DatagramReceiver,
    DatagramSender,
    ReceivedDatagram,
    TransportKind,
    WriteOutcome,
};

use super::{
    receive_datagram,
    send_datagram,
};

#[derive(Debug)]
pub struct UdpUnicast {
    socket: UdpSocket,
    config: UdpConfig,
    stats: TransportStats,
}

impl UdpUnicast {
    pub fn bind(
        address: SocketAddr,
        config: UdpConfig,
    ) -> TransportResult<Self> {
        config.validate()?;

        let socket = socket::create_udp_socket(
            address,
            config.socket,
        )?;

        socket.bind(&SockAddr::from(address))?;

        let socket: UdpSocket = socket.into();

        socket.set_ttl(config.ttl)?;

        Ok(Self {
            socket,
            config,
            stats: TransportStats::default(),
        })
    }

    #[must_use]
    pub const fn kind(&self) -> TransportKind {
        TransportKind::UdpUnicast
    }

    #[must_use]
    pub const fn config(&self) -> UdpConfig {
        self.config
    }

    #[must_use]
    pub const fn stats(&self) -> &TransportStats {
        &self.stats
    }

    #[must_use]
    pub const fn socket(&self) -> &UdpSocket {
        &self.socket
    }

    #[must_use]
    pub const fn required_receive_capacity(&self) -> usize {
        self.config.max_datagram_size + 1
    }

    pub fn buffer_sizes(&self) -> TransportResult<SocketBufferSizes> {
        socket::buffer_sizes(&SockRef::from(&self.socket))
    }

    pub fn create_receive_buffer(&self) -> TransportResult<ReceiveBuffer> {
        ReceiveBuffer::new(self.required_receive_capacity())
    }

    #[inline]
    pub fn receive_into(
        &mut self,
        buffer: &mut ReceiveBuffer,
    ) -> TransportResult<Option<ReceivedDatagram>> {
        let result = self.receive(buffer.writable())?;

        if let Some(datagram) = result {
            buffer.commit(datagram.length)?;
        }

        Ok(result)
    }

    pub fn set_nonblocking(
        &mut self,
        enabled: bool,
    ) -> TransportResult<()> {
        self.socket.set_nonblocking(enabled)?;
        self.config.socket.nonblocking = enabled;

        Ok(())
    }

    pub fn reset_stats(&mut self) {
        self.stats.reset();
    }
}

impl DatagramReceiver for UdpUnicast {
    #[inline]
    fn receive(
        &mut self,
        buffer: &mut [u8],
    ) -> TransportResult<Option<ReceivedDatagram>> {
        receive_datagram(
            &self.socket,
            buffer,
            self.config.max_datagram_size,
            &mut self.stats,
        )
    }

    fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }
}

impl DatagramSender for UdpUnicast {
    #[inline]
    fn send_to(
        &mut self,
        payload: &[u8],
        destination: SocketAddr,
    ) -> TransportResult<WriteOutcome> {
        send_datagram(
            &self.socket,
            payload,
            destination,
            self.config.max_datagram_size,
            &mut self.stats,
        )
    }
}

