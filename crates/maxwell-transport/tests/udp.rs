//! UDP loopback integration tests.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

use std::thread;
use std::time::{Duration, Instant};

use maxwell_transport::{
    DatagramReceiver, DatagramSender, MulticastConfig, ReceiveBuffer, TransportError, UdpConfig,
    UdpMulticast, UdpUnicast, WriteOutcome,
};

const TIMEOUT: Duration = Duration::from_secs(5);

fn address() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn config() -> UdpConfig {
    UdpConfig {
        max_datagram_size: 1024,
        ..UdpConfig::default()
    }
}

fn receive(receiver: &mut UdpUnicast, buffer: &mut ReceiveBuffer) -> usize {
    let deadline = Instant::now() + TIMEOUT;

    loop {
        if let Some(datagram) = receiver.receive_into(buffer).unwrap() {
            return datagram.length;
        }

        assert!(Instant::now() < deadline, "UDP receive timed out",);

        thread::yield_now();
    }
}

#[test]
fn unicast_roundtrip() {
    let mut receiver = UdpUnicast::bind(address(), config()).unwrap();

    let mut sender = UdpUnicast::bind(address(), config()).unwrap();

    let destination = receiver.local_addr().unwrap();

    let payload = b"maxwell market data";

    assert_eq!(
        sender.send_to(payload, destination).unwrap(),
        WriteOutcome::Written(payload.len()),
    );

    let mut buffer = receiver.create_receive_buffer().unwrap();

    assert_eq!(receive(&mut receiver, &mut buffer), payload.len(),);

    assert_eq!(buffer.as_slice(), payload);
    assert_eq!(receiver.stats().datagrams_received, 1);
    assert_eq!(sender.stats().datagrams_sent, 1);
}

#[test]
fn empty_nonblocking_socket() {
    let mut receiver = UdpUnicast::bind(address(), config()).unwrap();

    let mut buffer = receiver.create_receive_buffer().unwrap();

    assert!(receiver.receive_into(&mut buffer).unwrap().is_none());
    assert!(buffer.is_empty());
}

#[test]
fn datagram_boundaries_preserved() {
    let mut receiver = UdpUnicast::bind(address(), config()).unwrap();

    let mut sender = UdpUnicast::bind(address(), config()).unwrap();

    let destination = receiver.local_addr().unwrap();

    sender.send_to(b"first", destination).unwrap();
    sender.send_to(b"second", destination).unwrap();

    let mut buffer = receiver.create_receive_buffer().unwrap();

    let mut received = Vec::new();

    for _ in 0..2 {
        receive(&mut receiver, &mut buffer);
        received.push(buffer.as_slice().to_vec());
    }

    // UDP does not guarantee delivery order.
    received.sort_unstable();

    assert_eq!(received, vec![b"first".to_vec(), b"second".to_vec()],);
}

#[test]
fn maximum_size_datagram() {
    let mut receiver = UdpUnicast::bind(address(), config()).unwrap();

    let mut sender = UdpUnicast::bind(address(), config()).unwrap();

    let payload = vec![42; 1024];

    sender
        .send_to(&payload, receiver.local_addr().unwrap())
        .unwrap();

    let mut buffer = receiver.create_receive_buffer().unwrap();

    assert_eq!(receive(&mut receiver, &mut buffer), 1024);
    assert_eq!(buffer.as_slice(), payload.as_slice());
}

#[test]
fn oversized_outgoing_datagram_rejected() {
    let mut sender = UdpUnicast::bind(address(), config()).unwrap();

    let receiver = UdpUnicast::bind(address(), config()).unwrap();

    let result = sender.send_to(&[0; 1025], receiver.local_addr().unwrap());

    assert!(matches!(
        result,
        Err(TransportError::DatagramTooLarge { .. }),
    ));
}

#[test]
fn oversized_incoming_datagram_rejected() {
    let mut receiver = UdpUnicast::bind(address(), config()).unwrap();

    // Bypass Maxwell's outgoing size validation.
    let sender = UdpSocket::bind(address()).unwrap();

    sender
        .send_to(&[42; 2048], receiver.local_addr().unwrap())
        .unwrap();

    let mut buffer = receiver.create_receive_buffer().unwrap();

    let deadline = Instant::now() + TIMEOUT;

    loop {
        match receiver.receive_into(&mut buffer) {
            Err(TransportError::DatagramTooLarge { .. }) => {
                break;
            }

            Ok(None) => {
                assert!(Instant::now() < deadline);
                thread::yield_now();
            }

            other => {
                panic!("unexpected receive result: {other:?}");
            }
        }
    }

    assert!(buffer.is_empty());
    assert_eq!(receiver.stats().rejected_datagrams, 1);
}

#[test]
fn receive_buffer_requires_guard_byte() {
    let mut receiver = UdpUnicast::bind(address(), config()).unwrap();

    let mut buffer = [0; 1024];

    assert!(matches!(
        receiver.receive(&mut buffer),
        Err(TransportError::BufferTooSmall {
            required: 1025,
            available: 1024,
        }),
    ));
}

#[test]
fn zero_length_udp_datagram() {
    let mut receiver = UdpUnicast::bind(address(), config()).unwrap();

    let mut sender = UdpUnicast::bind(address(), config()).unwrap();

    sender.send_to(&[], receiver.local_addr().unwrap()).unwrap();

    let mut buffer = receiver.create_receive_buffer().unwrap();

    assert_eq!(receive(&mut receiver, &mut buffer), 0);
    assert_eq!(receiver.stats().datagrams_received, 1);
}

#[test]
fn actual_socket_buffer_sizes_available() {
    let socket = UdpUnicast::bind(address(), config()).unwrap();

    let sizes = socket.buffer_sizes().unwrap();

    assert!(sizes.receive > 0);
    assert!(sizes.send > 0);
}

// Multicast support depends on the host's network
// configuration, routing, and firewall rules.
//
// Run explicitly on a multicast-capable host:
//
// cargo test -p maxwell-transport --test udp \
//     multicast_loopback -- --ignored

#[test]
#[ignore = "requires multicast-capable host networking"]
fn multicast_loopback() {
    let reservation = UdpSocket::bind(address()).unwrap();
    let port = reservation.local_addr().unwrap().port();

    drop(reservation);

    let config = MulticastConfig {
        group: Ipv4Addr::new(239, 255, 0, 99),
        interface: Ipv4Addr::LOCALHOST,
        port,
        loopback: true,
        ..MulticastConfig::default()
    };

    let mut receiver = UdpMulticast::subscribe(config).unwrap();

    let mut sender = UdpMulticast::publisher(config).unwrap();

    let mut buffer = receiver.create_receive_buffer().unwrap();

    sender.publish(b"multicast data").unwrap();

    let deadline = Instant::now() + TIMEOUT;

    loop {
        if receiver.receive_into(&mut buffer).unwrap().is_some() {
            break;
        }

        assert!(Instant::now() < deadline, "multicast receive timed out",);

        thread::yield_now();
    }

    assert_eq!(buffer.as_slice(), b"multicast data");

    receiver.leave().unwrap();

    assert!(!receiver.is_joined());
}
