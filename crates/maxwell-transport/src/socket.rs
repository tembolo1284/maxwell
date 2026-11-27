//! Shared socket creation and configuration utilities.

use std::net::SocketAddr;

use socket2::{Domain, Protocol, Socket, Type};

use crate::config::SocketConfig;
use crate::error::TransportResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketBufferSizes {
    pub receive: usize,
    pub send: usize,
}

#[must_use]
pub(crate) const fn domain(address: SocketAddr) -> Domain {
    match address {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    }
}

pub(crate) fn create_udp_socket(
    address: SocketAddr,
    config: SocketConfig,
) -> TransportResult<Socket> {
    config.validate()?;

    let socket = Socket::new(domain(address), Type::DGRAM, Some(Protocol::UDP))?;

    configure_socket(&socket, config)?;

    Ok(socket)
}

pub(crate) fn create_tcp_socket(
    address: SocketAddr,
    config: SocketConfig,
) -> TransportResult<Socket> {
    config.validate()?;

    let socket = Socket::new(domain(address), Type::STREAM, Some(Protocol::TCP))?;

    configure_socket(&socket, config)?;

    Ok(socket)
}

pub(crate) fn configure_socket(socket: &Socket, config: SocketConfig) -> TransportResult<()> {
    socket.set_reuse_address(config.reuse_address)?;

    socket.set_recv_buffer_size(config.receive_buffer_size)?;

    socket.set_send_buffer_size(config.send_buffer_size)?;

    socket.set_nonblocking(config.nonblocking)?;

    Ok(())
}

pub(crate) fn buffer_sizes(socket: &socket2::SockRef<'_>) -> TransportResult<SocketBufferSizes> {
    Ok(SocketBufferSizes {
        receive: socket.recv_buffer_size()?,
        send: socket.send_buffer_size()?,
    })
}
