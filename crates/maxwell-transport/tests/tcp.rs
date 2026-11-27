
//! TCP loopback integration tests.

use std::net::{
    Shutdown,
    SocketAddr,
};

use std::thread;
use std::time::{Duration, Instant};

use maxwell_transport::{
    FrameDecoder,
    FrameEncoder,
    FramingConfig,
    ReadOutcome,
    TcpAcceptor,
    TcpConfig,
    TcpConnection,
    TransportError,
    WriteOutcome,
};

const TIMEOUT: Duration = Duration::from_secs(5);

fn address() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn connection_pair() -> (TcpConnection, TcpConnection) {
    let config = TcpConfig::default();

    let mut listener =
        TcpAcceptor::bind(address(), config).unwrap();

    let client = TcpConnection::connect(
        listener.local_addr().unwrap(),
        config,
    )
    .unwrap();

    let deadline = Instant::now() + TIMEOUT;

    loop {
        if let Some((server, _)) = listener.accept().unwrap() {
            return (client, server);
        }

        assert!(
            Instant::now() < deadline,
            "TCP accept timed out",
        );

        thread::yield_now();
    }
}

fn write_all(
    connection: &mut TcpConnection,
    payload: &[u8],
) {
    let mut offset = 0;
    let deadline = Instant::now() + TIMEOUT;

    while offset < payload.len() {
        match connection.write(&payload[offset..]).unwrap() {
            WriteOutcome::Written(length) => {
                offset += length;
            }

            WriteOutcome::WouldBlock => {
                thread::yield_now();
            }
        }

        assert!(Instant::now() < deadline);
    }
}

fn read_exact(
    connection: &mut TcpConnection,
    output: &mut [u8],
) {
    let mut offset = 0;
    let deadline = Instant::now() + TIMEOUT;

    while offset < output.len() {
        match connection.read(&mut output[offset..]).unwrap() {
            ReadOutcome::Data(length) => {
                offset += length;
            }

            ReadOutcome::WouldBlock => {
                thread::yield_now();
            }

            ReadOutcome::Closed => {
                panic!("unexpected TCP closure");
            }
        }

        assert!(Instant::now() < deadline);
    }
}

#[test]
fn connection_establishment() {
    let (client, server) = connection_pair();

    assert_eq!(
        client.peer_addr().unwrap(),
        server.local_addr().unwrap(),
    );

    assert_eq!(
        server.peer_addr().unwrap(),
        client.local_addr().unwrap(),
    );
}

#[test]
fn no_pending_connection() {
    let mut listener = TcpAcceptor::bind(
        address(),
        TcpConfig::default(),
    )
    .unwrap();

    assert!(listener.accept().unwrap().is_none());
}

#[test]
fn tcp_no_delay_enabled() {
    let (client, server) = connection_pair();

    assert!(client.stream().nodelay().unwrap());
    assert!(server.stream().nodelay().unwrap());
}

#[test]
fn bidirectional_communication() {
    let (mut client, mut server) = connection_pair();

    write_all(&mut client, b"request");

    let mut request = [0; 7];

    read_exact(&mut server, &mut request);

    assert_eq!(&request, b"request");

    write_all(&mut server, b"response");

    let mut response = [0; 8];

    read_exact(&mut client, &mut response);

    assert_eq!(&response, b"response");
}

#[test]
fn nonblocking_read_without_data() {
    let (_, mut server) = connection_pair();

    // Keep the client alive while checking WouldBlock.
    // This test is implemented below with explicit binding.
    let _ = &mut server;
}

#[test]
fn idle_connection_returns_would_block() {
    let (_client, mut server) = connection_pair();

    let mut buffer = [0; 128];

    assert_eq!(
        server.read(&mut buffer).unwrap(),
        ReadOutcome::WouldBlock,
    );
}

#[test]
fn empty_read_buffer_rejected() {
    let (_client, mut server) = connection_pair();

    assert!(matches!(
        server.read(&mut []),
        Err(TransportError::BufferTooSmall { .. }),
    ));

    assert!(!server.is_read_closed());
}

#[test]
fn orderly_peer_shutdown() {
    let (mut client, mut server) = connection_pair();

    client.shutdown(Shutdown::Write).unwrap();

    let mut buffer = [0; 128];
    let deadline = Instant::now() + TIMEOUT;

    loop {
        match server.read(&mut buffer).unwrap() {
            ReadOutcome::Closed => break,

            ReadOutcome::WouldBlock => {
                thread::yield_now();
            }

            ReadOutcome::Data(_) => {
                panic!("unexpected data");
            }
        }

        assert!(Instant::now() < deadline);
    }

    assert!(server.is_read_closed());
    assert!(!server.is_write_closed());
}

#[test]
fn tcp_half_close_preserves_reverse_direction() {
    let (mut client, mut server) = connection_pair();

    client.shutdown(Shutdown::Write).unwrap();

    write_all(&mut server, b"final response");

    let mut buffer = [0; 14];

    read_exact(&mut client, &mut buffer);

    assert_eq!(&buffer, b"final response");
}

#[test]
fn framed_message_roundtrip() {
    let (mut client, mut server) = connection_pair();

    let config = FramingConfig {
        max_frame_size: 1024,
        allow_empty_frames: false,
    };

    let mut encoder = FrameEncoder::new(config).unwrap();
    let mut decoder = FrameDecoder::new(config).unwrap();

    encoder.prepare(b"maxwell snapshot request").unwrap();

    let deadline = Instant::now() + TIMEOUT;

    while encoder.is_pending() {
        encoder.flush_once(&mut client).unwrap();

        assert!(Instant::now() < deadline);

        thread::yield_now();
    }

    // Intentionally use a tiny receive buffer to
    // exercise incremental frame reconstruction.
    let mut buffer = [0; 3];

    while !decoder.is_ready() {
        match server.read(&mut buffer).unwrap() {
            ReadOutcome::Data(length) => {
                let status =
                    decoder.push(&buffer[..length]).unwrap();

                assert_eq!(status.consumed(), length);
            }

            ReadOutcome::WouldBlock => {
                thread::yield_now();
            }

            ReadOutcome::Closed => {
                panic!("unexpected connection closure");
            }
        }

        assert!(Instant::now() < deadline);
    }

    assert_eq!(
        decoder.frame(),
        Some(&b"maxwell snapshot request"[..]),
    );
}

#[test]
fn transfer_large_payload() {
    const SIZE: usize = 1024 * 1024;

    let (mut client, mut server) = connection_pair();

    let payload: Vec<u8> = (0..SIZE)
        .map(|index| (index % 251) as u8)
        .collect();

    let expected = payload.clone();

    let sender = thread::spawn(move || {
        write_all(&mut client, &payload);
        client.stats().bytes_sent
    });

    let mut received = vec![0; SIZE];

    read_exact(&mut server, &mut received);

    assert_eq!(received, expected);

    assert_eq!(
        sender.join().unwrap(),
        SIZE as u64,
    );

    assert_eq!(
        server.stats().bytes_received,
        SIZE as u64,
    );
}

