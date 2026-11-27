//! Single-owner TCP connection.
//!
//! Connection establishment is a bounded blocking
//! control-path operation.
//!
//! Established connections operate in nonblocking mode
//! by default.

use std::io::{ErrorKind, Read, Write};

use std::net::{Shutdown, SocketAddr, TcpStream};

use socket2::{SockAddr, SockRef};

use crate::buffer::ReceiveBuffer;
use crate::config::TcpConfig;
use crate::error::{TransportError, TransportResult};
use crate::socket::{self, SocketBufferSizes};
use crate::stats::TransportStats;

use crate::transport::{ReadOutcome, TransportKind, WriteOutcome};

#[derive(Debug)]
pub struct TcpConnection {
    stream: TcpStream,
    config: TcpConfig,
    stats: TransportStats,
    read_closed: bool,
    write_closed: bool,
}

impl TcpConnection {
    /// Opens a TCP connection to `address`.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid, the socket
    /// cannot be created or configured, or the connection attempt
    /// fails or times out.
    pub fn connect(address: SocketAddr, config: TcpConfig) -> TransportResult<Self> {
        config.validate()?;

        let mut initial = config.socket;
        initial.nonblocking = false;

        let socket = socket::create_tcp_socket(address, initial)?;

        socket.connect_timeout(&SockAddr::from(address), config.connect_timeout)?;

        let stream: TcpStream = socket.into();

        Self::from_stream(stream, config)
    }

    /// Wraps an established stream and applies `config`.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid or a socket
    /// option cannot be applied.
    pub fn from_stream(stream: TcpStream, config: TcpConfig) -> TransportResult<Self> {
        config.validate()?;

        stream.set_nodelay(config.no_delay)?;

        {
            let socket = SockRef::from(&stream);

            socket.set_keepalive(config.keepalive)?;

            socket.set_recv_buffer_size(config.socket.receive_buffer_size)?;

            socket.set_send_buffer_size(config.socket.send_buffer_size)?;
        }

        stream.set_nonblocking(config.socket.nonblocking)?;

        Ok(Self {
            stream,
            config,
            stats: TransportStats::default(),
            read_closed: false,
            write_closed: false,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> TransportKind {
        TransportKind::Tcp
    }

    #[must_use]
    pub const fn config(&self) -> TcpConfig {
        self.config
    }

    #[must_use]
    pub const fn stats(&self) -> &TransportStats {
        &self.stats
    }

    #[must_use]
    pub const fn stream(&self) -> &TcpStream {
        &self.stream
    }

    #[must_use]
    pub const fn is_read_closed(&self) -> bool {
        self.read_closed
    }

    #[must_use]
    pub const fn is_write_closed(&self) -> bool {
        self.write_closed
    }

    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.read_closed && self.write_closed
    }

    /// Returns the local socket address.
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system cannot report it.
    pub fn local_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.stream.local_addr()?)
    }

    /// Returns the remote socket address.
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system cannot report it.
    pub fn peer_addr(&self) -> TransportResult<SocketAddr> {
        Ok(self.stream.peer_addr()?)
    }

    /// Returns the socket buffer sizes granted by the kernel.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket options cannot be read.
    pub fn buffer_sizes(&self) -> TransportResult<SocketBufferSizes> {
        socket::buffer_sizes(&SockRef::from(&self.stream))
    }

    /// Switches the socket between blocking and nonblocking mode.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket mode cannot be changed.
    pub fn set_nonblocking(&mut self, enabled: bool) -> TransportResult<()> {
        self.stream.set_nonblocking(enabled)?;
        self.config.socket.nonblocking = enabled;

        Ok(())
    }

    /// Reads available bytes without assuming that a
    /// complete application message has arrived.
    ///
    /// # Errors
    ///
    /// Returns `BufferTooSmall` if `buffer` is empty, or an I/O error
    /// if the socket read fails.
    #[inline]
    pub fn read(&mut self, buffer: &mut [u8]) -> TransportResult<ReadOutcome> {
        if self.read_closed {
            return Ok(ReadOutcome::Closed);
        }

        if buffer.is_empty() {
            return Err(TransportError::BufferTooSmall {
                required: 1,
                available: 0,
            });
        }

        loop {
            match self.stream.read(buffer) {
                Ok(0) => {
                    self.read_closed = true;
                    self.stats.record_connection_closed();

                    return Ok(ReadOutcome::Closed);
                }

                Ok(length) => {
                    self.stats.record_read(length);

                    return Ok(ReadOutcome::Data(length));
                }

                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    self.stats.record_read_would_block();

                    return Ok(ReadOutcome::WouldBlock);
                }

                Err(error) if error.kind() == ErrorKind::Interrupted => {
                    self.stats.record_interrupted();
                }

                Err(error) => {
                    self.stats.record_error();

                    if matches!(
                        error.kind(),
                        ErrorKind::ConnectionReset
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::NotConnected
                    ) {
                        self.read_closed = true;
                        self.write_closed = true;
                    }

                    return Err(error.into());
                }
            }
        }
    }

    /// Reads available bytes into `buffer` and commits their length.
    ///
    /// # Errors
    ///
    /// Returns an error if the buffer has no capacity or the socket
    /// read fails.
    #[inline]
    pub fn read_into(&mut self, buffer: &mut ReceiveBuffer) -> TransportResult<ReadOutcome> {
        let outcome = self.read(buffer.writable())?;

        if let ReadOutcome::Data(length) = outcome {
            buffer.commit(length)?;
        }

        Ok(outcome)
    }

    /// Attempts to write available bytes.
    ///
    /// A successful write may consume only a prefix of
    /// the supplied buffer.
    ///
    /// The caller must retain and resume the remainder.
    ///
    /// # Errors
    ///
    /// Returns `ConnectionClosed` if the write side is closed, or an
    /// I/O error if the socket write fails.
    #[inline]
    pub fn write(&mut self, payload: &[u8]) -> TransportResult<WriteOutcome> {
        if self.write_closed {
            return Err(TransportError::ConnectionClosed);
        }

        if payload.is_empty() {
            return Ok(WriteOutcome::Written(0));
        }

        loop {
            match self.stream.write(payload) {
                Ok(0) => {
                    self.write_closed = true;
                    self.stats.record_connection_closed();

                    return Err(TransportError::ConnectionClosed);
                }

                Ok(length) => {
                    self.stats.record_write(length);

                    if length < payload.len() {
                        self.stats.record_partial_write();
                    }

                    return Ok(WriteOutcome::Written(length));
                }

                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    self.stats.record_write_would_block();

                    return Ok(WriteOutcome::WouldBlock);
                }

                Err(error) if error.kind() == ErrorKind::Interrupted => {
                    self.stats.record_interrupted();
                }

                Err(error) => {
                    self.stats.record_error();

                    match error.kind() {
                        ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::NotConnected => {
                            self.read_closed = true;
                            self.write_closed = true;
                        }

                        ErrorKind::BrokenPipe => {
                            self.write_closed = true;
                        }

                        _ => {}
                    }

                    return Err(error.into());
                }
            }
        }
    }

    /// Shuts down one or both directions of the connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket shutdown fails.
    pub fn shutdown(&mut self, direction: Shutdown) -> TransportResult<()> {
        self.stream.shutdown(direction)?;

        match direction {
            Shutdown::Read => {
                self.read_closed = true;
            }

            Shutdown::Write => {
                self.write_closed = true;
            }

            Shutdown::Both => {
                self.read_closed = true;
                self.write_closed = true;
            }
        }

        Ok(())
    }

    pub fn reset_stats(&mut self) {
        self.stats.reset();
    }
}
