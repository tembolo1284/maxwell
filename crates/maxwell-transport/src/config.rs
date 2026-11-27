//! Transport configuration and resource limits.

use std::time::Duration;

use crate::error::{TransportError, TransportResult};

pub const DEFAULT_SOCKET_BUFFER_SIZE: usize = 4 * 1024 * 1024;

pub const DEFAULT_DATAGRAM_SIZE: usize = 1_472;
pub const MAX_UDP_DATAGRAM_SIZE: usize = 65_507;

pub const DEFAULT_MAX_FRAME_SIZE: usize = 1024 * 1024;
pub const MAX_FRAME_SIZE: usize = 64 * 1024 * 1024;

pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketConfig {
    pub nonblocking: bool,
    pub reuse_address: bool,
    pub receive_buffer_size: usize,
    pub send_buffer_size: usize,
}

impl SocketConfig {
    pub fn validate(self) -> TransportResult<()> {
        if self.receive_buffer_size == 0 || self.send_buffer_size == 0 {
            return Err(TransportError::InvalidConfiguration(
                "socket buffer sizes must be positive",
            ));
        }

        if self.receive_buffer_size > i32::MAX as usize || self.send_buffer_size > i32::MAX as usize
        {
            return Err(TransportError::InvalidConfiguration(
                "socket buffer size exceeds supported range",
            ));
        }

        Ok(())
    }
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            nonblocking: true,
            reuse_address: false,
            receive_buffer_size: DEFAULT_SOCKET_BUFFER_SIZE,
            send_buffer_size: DEFAULT_SOCKET_BUFFER_SIZE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpConfig {
    pub socket: SocketConfig,

    /// Maximum accepted application datagram size.
    pub max_datagram_size: usize,

    /// IP time-to-live for outgoing unicast traffic.
    pub ttl: u32,
}

impl UdpConfig {
    pub fn validate(self) -> TransportResult<()> {
        self.socket.validate()?;

        if self.max_datagram_size == 0 || self.max_datagram_size > MAX_UDP_DATAGRAM_SIZE {
            return Err(TransportError::InvalidConfiguration(
                "invalid maximum UDP datagram size",
            ));
        }

        if self.ttl == 0 || self.ttl > 255 {
            return Err(TransportError::InvalidConfiguration(
                "UDP TTL must be between 1 and 255",
            ));
        }

        Ok(())
    }
}

impl Default for UdpConfig {
    fn default() -> Self {
        Self {
            socket: SocketConfig::default(),
            max_datagram_size: DEFAULT_DATAGRAM_SIZE,
            ttl: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpConfig {
    pub socket: SocketConfig,

    /// Disables Nagle's algorithm.
    pub no_delay: bool,

    /// Enables operating-system TCP keepalive.
    pub keepalive: bool,

    /// Timeout for establishing a connection.
    pub connect_timeout: Duration,

    /// Requested pending connection backlog.
    pub listen_backlog: i32,
}

impl TcpConfig {
    pub fn validate(self) -> TransportResult<()> {
        self.socket.validate()?;

        if self.connect_timeout.is_zero() {
            return Err(TransportError::InvalidConfiguration(
                "TCP connect timeout must be positive",
            ));
        }

        if self.listen_backlog <= 0 {
            return Err(TransportError::InvalidConfiguration(
                "TCP listen backlog must be positive",
            ));
        }

        Ok(())
    }
}

impl Default for TcpConfig {
    fn default() -> Self {
        Self {
            socket: SocketConfig::default(),
            no_delay: true,
            keepalive: true,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            listen_backlog: 128,
        }
    }
}

/// TCP framing uses a four-byte unsigned big-endian
/// payload length, excluding the header itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramingConfig {
    pub max_frame_size: usize,
    pub allow_empty_frames: bool,
}

impl FramingConfig {
    pub fn validate(self) -> TransportResult<()> {
        if self.max_frame_size == 0 || self.max_frame_size > MAX_FRAME_SIZE {
            return Err(TransportError::InvalidConfiguration(
                "invalid maximum TCP frame size",
            ));
        }

        Ok(())
    }
}

impl Default for FramingConfig {
    fn default() -> Self {
        Self {
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            allow_empty_frames: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(SocketConfig::default().validate().is_ok());
        assert!(UdpConfig::default().validate().is_ok());
        assert!(TcpConfig::default().validate().is_ok());
        assert!(FramingConfig::default().validate().is_ok());
    }

    #[test]
    fn invalid_socket_buffer() {
        let config = SocketConfig {
            receive_buffer_size: 0,
            ..SocketConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn oversized_datagram_configuration() {
        let config = UdpConfig {
            max_datagram_size: MAX_UDP_DATAGRAM_SIZE + 1,
            ..UdpConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_frame_configuration() {
        let config = FramingConfig {
            max_frame_size: 0,
            ..FramingConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_connect_timeout() {
        let config = TcpConfig {
            connect_timeout: Duration::ZERO,
            ..TcpConfig::default()
        };

        assert!(config.validate().is_err());
    }
}
