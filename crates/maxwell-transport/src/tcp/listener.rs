//! Nonblocking TCP connection acceptance.

use std::io::ErrorKind;

use std::net::{SocketAddr, TcpListener};

use socket2::SockAddr;

use super::connection::TcpConnection;
use crate::config::TcpConfig;
use crate::error::TransportResult;
use crate::socket;

#[derive(Debug)]
pub struct TcpAcceptor {
    listener: TcpListener,
    config: TcpConfig,

    accepted_connections: u64,
    accept_errors: u64,
}

impl TcpAcceptor {
    pub fn bind(address: SocketAddr, config: TcpConfig) -> TransportResult<Self> {
        config.validate()?;

        let socket = socket::create_tcp_socket(address, config.socket)?;

        socket.bind(&SockAddr::from(address))?;
        socket.listen(config.listen_backlog)?;

        let listener: TcpListener = socket.into();

        Ok(Self {
            listener,
            config,
            accepted_connections: 0,
            accept_errors: 0,
        })
    }

    #[must_use]
    pub const fn config(&self) -> TcpConfig {
        self.config
    }

    #[must_use]
    pub const fn listener(&self) -> &TcpListener {
        &self.listener
    }

    #[must_use]
    pub const fn accepted_connections(&self) -> u64 {
        self.accepted_connections
    }

    #[must_use]
    pub const fn accept_errors(&self) -> u64 {
        self.accept_errors
    }

    pub fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    /// Attempts to accept one pending connection.
    ///
    /// None indicates that no connection is available.
    pub fn accept(&mut self) -> TransportResult<Option<(TcpConnection, SocketAddr)>> {
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => match TcpConnection::from_stream(stream, self.config) {
                    Ok(connection) => {
                        self.accepted_connections = self.accepted_connections.saturating_add(1);

                        return Ok(Some((connection, peer)));
                    }

                    Err(error) => {
                        self.accept_errors = self.accept_errors.saturating_add(1);

                        return Err(error);
                    }
                },

                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    return Ok(None);
                }

                Err(error) if error.kind() == ErrorKind::Interrupted => {
                    continue;
                }

                Err(error) => {
                    self.accept_errors = self.accept_errors.saturating_add(1);

                    return Err(error.into());
                }
            }
        }
    }

    pub fn set_nonblocking(&mut self, enabled: bool) -> TransportResult<()> {
        self.listener.set_nonblocking(enabled)?;
        self.config.socket.nonblocking = enabled;

        Ok(())
    }
}
