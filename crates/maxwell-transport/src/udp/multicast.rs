//! IPv4 UDP multicast transport.
//!
//! The initial implementation supports one multicast
//! group per transport instance.
//!
//! Interface selection is explicit. An unspecified
//! interface allows the operating system to choose.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};

use socket2::{SockAddr, SockRef};

use crate::buffer::ReceiveBuffer;
use crate::config::UdpConfig;
use crate::error::{TransportError, TransportResult};
use crate::socket::{self, SocketBufferSizes};
use crate::stats::TransportStats;

use crate::transport::{
    DatagramReceiver, DatagramSender, ReceivedDatagram, TransportKind, WriteOutcome,
};

use super::{receive_datagram, send_datagram};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MulticastConfig {
    pub udp: UdpConfig,

    pub group: Ipv4Addr,
    pub port: u16,
    pub interface: Ipv4Addr,

    pub multicast_ttl: u32,
    pub loopback: bool,
}

impl MulticastConfig {
    pub fn validate(self) -> TransportResult<()> {
        self.udp.validate()?;

        if !self.group.is_multicast() {
            return Err(TransportError::InvalidMulticastAddress);
        }

        if self.port == 0 {
            return Err(TransportError::InvalidConfiguration(
                "multicast port must be nonzero",
            ));
        }

        if self.multicast_ttl > 255 {
            return Err(TransportError::InvalidConfiguration(
                "multicast TTL must not exceed 255",
            ));
        }

        Ok(())
    }

    #[must_use]
    pub const fn destination(self) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(self.group, self.port))
    }
}

impl Default for MulticastConfig {
    fn default() -> Self {
        Self {
            udp: UdpConfig::default(),
            group: Ipv4Addr::new(239, 255, 0, 1),
            port: 1236,
            interface: Ipv4Addr::UNSPECIFIED,
            multicast_ttl: 1,
            loopback: true,
        }
    }
}

#[derive(Debug)]
pub struct UdpMulticast {
    socket: UdpSocket,
    config: MulticastConfig,
    stats: TransportStats,
    joined: bool,
}

impl UdpMulticast {
    /// Creates a receiving socket and joins the group.
    pub fn subscribe(mut config: MulticastConfig) -> TransportResult<Self> {
        config.validate()?;

        // Multiple subscribers may bind the same port.
        config.udp.socket.reuse_address = true;

        let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.port));

        let socket = socket::create_udp_socket(address, config.udp.socket)?;

        socket.bind(&SockAddr::from(address))?;

        socket.set_multicast_if_v4(&config.interface)?;
        socket.set_multicast_ttl_v4(config.multicast_ttl)?;
        socket.set_multicast_loop_v4(config.loopback)?;

        socket.join_multicast_v4(&config.group, &config.interface)?;

        let socket: UdpSocket = socket.into();

        Ok(Self {
            socket,
            config,
            stats: TransportStats::default(),
            joined: true,
        })
    }

    /// Creates a publishing socket without joining the group.
    pub fn publisher(config: MulticastConfig) -> TransportResult<Self> {
        config.validate()?;

        let address = SocketAddr::V4(SocketAddrV4::new(config.interface, 0));

        let socket = socket::create_udp_socket(address, config.udp.socket)?;

        socket.bind(&SockAddr::from(address))?;

        socket.set_multicast_if_v4(&config.interface)?;
        socket.set_multicast_ttl_v4(config.multicast_ttl)?;
        socket.set_multicast_loop_v4(config.loopback)?;

        let socket: UdpSocket = socket.into();

        Ok(Self {
            socket,
            config,
            stats: TransportStats::default(),
            joined: false,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> TransportKind {
        TransportKind::UdpMulticast
    }

    #[must_use]
    pub const fn config(&self) -> MulticastConfig {
        self.config
    }

    #[must_use]
    pub const fn is_joined(&self) -> bool {
        self.joined
    }

    #[must_use]
    pub const fn stats(&self) -> &TransportStats {
        &self.stats
    }

    #[must_use]
    pub const fn socket(&self) -> &UdpSocket {
        &self.socket
    }

    pub fn buffer_sizes(&self) -> TransportResult<SocketBufferSizes> {
        socket::buffer_sizes(&SockRef::from(&self.socket))
    }

    pub fn create_receive_buffer(&self) -> TransportResult<ReceiveBuffer> {
        ReceiveBuffer::new(self.config.udp.max_datagram_size + 1)
    }

    pub fn join(&mut self) -> TransportResult<()> {
        if !self.joined {
            self.socket
                .join_multicast_v4(&self.config.group, &self.config.interface)?;

            self.joined = true;
        }

        Ok(())
    }

    pub fn leave(&mut self) -> TransportResult<()> {
        if self.joined {
            self.socket
                .leave_multicast_v4(&self.config.group, &self.config.interface)?;

            self.joined = false;
        }

        Ok(())
    }

    #[inline]
    pub fn publish(&mut self, payload: &[u8]) -> TransportResult<WriteOutcome> {
        self.send_to(payload, self.config.destination())
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
}

impl DatagramReceiver for UdpMulticast {
    #[inline]
    fn receive(&mut self, buffer: &mut [u8]) -> TransportResult<Option<ReceivedDatagram>> {
        receive_datagram(
            &self.socket,
            buffer,
            self.config.udp.max_datagram_size,
            &mut self.stats,
        )
    }

    fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }
}

impl DatagramSender for UdpMulticast {
    #[inline]
    fn send_to(
        &mut self,
        payload: &[u8],
        destination: SocketAddr,
    ) -> TransportResult<WriteOutcome> {
        if !matches!(destination.ip(), IpAddr::V4(_)) {
            return Err(TransportError::AddressFamilyMismatch);
        }

        if destination != self.config.destination() {
            return Err(TransportError::InvalidConfiguration(
                "destination differs from configured multicast group",
            ));
        }

        send_datagram(
            &self.socket,
            payload,
            destination,
            self.config.udp.max_datagram_size,
            &mut self.stats,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_multicast_configuration() {
        assert!(MulticastConfig::default().validate().is_ok());
    }

    #[test]
    fn unicast_group_rejected() {
        let config = MulticastConfig {
            group: Ipv4Addr::LOCALHOST,
            ..MulticastConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_port_rejected() {
        let config = MulticastConfig {
            port: 0,
            ..MulticastConfig::default()
        };

        assert!(config.validate().is_err());
    }
}
