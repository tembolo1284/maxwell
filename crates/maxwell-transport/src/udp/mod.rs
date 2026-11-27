//! UDP unicast and multicast transports.

pub mod multicast;
pub mod unicast;

pub use unicast::UdpUnicast;

pub use multicast::{MulticastConfig, UdpMulticast};

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};

use crate::error::{TransportError, TransportResult};
use crate::stats::TransportStats;

use crate::transport::{ReceivedDatagram, WriteOutcome};

pub(crate) fn receive_datagram(
    socket: &UdpSocket,
    buffer: &mut [u8],
    maximum: usize,
    stats: &mut TransportStats,
) -> TransportResult<Option<ReceivedDatagram>> {
    let required = maximum + 1;

    if buffer.len() < required {
        return Err(TransportError::BufferTooSmall {
            required,
            available: buffer.len(),
        });
    }

    loop {
        match socket.recv_from(&mut buffer[..required]) {
            Ok((length, source)) => {
                if length > maximum {
                    stats.record_rejected_datagram();

                    // The actual datagram may have been
                    // larger than the reported length.
                    return Err(TransportError::DatagramTooLarge {
                        actual: length,
                        maximum,
                    });
                }

                stats.record_datagram_received(length);

                return Ok(Some(ReceivedDatagram { source, length }));
            }

            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                stats.record_read_would_block();

                return Ok(None);
            }

            Err(error) if error.kind() == ErrorKind::Interrupted => {
                stats.record_interrupted();
            }

            // Windows may report WSAEMSGSIZE rather than
            // returning a truncated datagram.
            #[cfg(windows)]
            Err(error) if error.raw_os_error() == Some(10040) => {
                stats.record_rejected_datagram();

                return Err(TransportError::DatagramTooLarge {
                    actual: required,
                    maximum,
                });
            }

            Err(error) => {
                stats.record_error();

                return Err(error.into());
            }
        }
    }
}

pub(crate) fn send_datagram(
    socket: &UdpSocket,
    payload: &[u8],
    destination: SocketAddr,
    maximum: usize,
    stats: &mut TransportStats,
) -> TransportResult<WriteOutcome> {
    if payload.len() > maximum {
        stats.record_rejected_datagram();

        return Err(TransportError::DatagramTooLarge {
            actual: payload.len(),
            maximum,
        });
    }

    loop {
        match socket.send_to(payload, destination) {
            Ok(length) => {
                if length != payload.len() {
                    stats.record_partial_write();

                    return Err(TransportError::PartialDatagramWrite {
                        expected: payload.len(),
                        actual: length,
                    });
                }

                stats.record_datagram_sent(length);

                return Ok(WriteOutcome::Written(length));
            }

            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                stats.record_write_would_block();

                return Ok(WriteOutcome::WouldBlock);
            }

            Err(error) if error.kind() == ErrorKind::Interrupted => {
                stats.record_interrupted();
            }

            Err(error) => {
                stats.record_error();

                return Err(error.into());
            }
        }
    }
}
