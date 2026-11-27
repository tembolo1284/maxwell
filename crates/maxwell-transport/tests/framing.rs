
//! TCP framing correctness and fragmentation tests.
//!
//! These tests do not require network sockets.

use maxwell_transport::{
    FrameDecoder,
    FrameEncoder,
    FramingConfig,
    TransportError,
};

fn config() -> FramingConfig {
    FramingConfig {
        max_frame_size: 4096,
        allow_empty_frames: false,
    }
}

fn encode(payload: &[u8]) -> Vec<u8> {
    let mut encoder = FrameEncoder::new(config()).unwrap();

    encoder.prepare(payload).unwrap();

    encoder.pending_bytes().to_vec()
}

fn consume(
    decoder: &mut FrameDecoder,
    input: &[u8],
    output: &mut Vec<Vec<u8>>,
) {
    let mut offset = 0;

    while offset < input.len() {
        let status = decoder.push(&input[offset..]).unwrap();

        offset += status.consumed();

        if status.is_ready() {
            output.push(decoder.frame().unwrap().to_vec());

            assert!(decoder.release());
        } else {
            assert_eq!(offset, input.len());
        }
    }
}

#[test]
fn every_possible_two_part_split() {
    let payload = b"deterministic market data reconstruction";
    let encoded = encode(payload);

    for split in 0..=encoded.len() {
        let mut decoder = FrameDecoder::new(config()).unwrap();

        let mut output = Vec::new();

        consume(
            &mut decoder,
            &encoded[..split],
            &mut output,
        );

        consume(
            &mut decoder,
            &encoded[split..],
            &mut output,
        );

        assert_eq!(
            output,
            vec![payload.to_vec()],
            "failed at split position {split}",
        );

        assert!(decoder.finish().is_ok());
    }
}

#[test]
fn one_byte_at_a_time() {
    let payload = b"fragmented TCP frame";
    let encoded = encode(payload);

    let mut decoder = FrameDecoder::new(config()).unwrap();
    let mut output = Vec::new();

    for byte in encoded {
        consume(&mut decoder, &[byte], &mut output);
    }

    assert_eq!(output, vec![payload.to_vec()]);
}

#[test]
fn multiple_frames_in_one_read() {
    let messages: [&[u8]; 3] = [
        b"first",
        b"second",
        b"third",
    ];

    let mut stream = Vec::new();

    for message in messages {
        stream.extend(encode(message));
    }

    let mut decoder = FrameDecoder::new(config()).unwrap();
    let mut output = Vec::new();

    consume(&mut decoder, &stream, &mut output);

    assert_eq!(
        output,
        messages.iter()
            .map(|message| message.to_vec())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn ready_frame_preserves_unconsumed_input() {
    let mut decoder = FrameDecoder::new(config()).unwrap();

    let first = encode(b"first");
    let second = encode(b"second");

    let mut stream = first.clone();
    stream.extend(&second);

    let status = decoder.push(&stream).unwrap();

    assert!(status.is_ready());
    assert_eq!(status.consumed(), first.len());

    let next = decoder
        .push(&stream[first.len()..])
        .unwrap();

    assert_eq!(next.consumed(), 0);
    assert_eq!(decoder.frame(), Some(&b"first"[..]));

    decoder.release();

    let status = decoder
        .push(&stream[first.len()..])
        .unwrap();

    assert!(status.is_ready());
    assert_eq!(decoder.frame(), Some(&b"second"[..]));
}

#[test]
fn maximum_frame_size() {
    let payload = vec![42; 4096];
    let encoded = encode(&payload);

    let mut decoder = FrameDecoder::new(config()).unwrap();

    assert!(decoder.push(&encoded).unwrap().is_ready());

    assert_eq!(
        decoder.frame(),
        Some(payload.as_slice()),
    );
}

#[test]
fn oversized_header_rejected_before_payload() {
    let mut decoder = FrameDecoder::new(config()).unwrap();

    assert!(matches!(
        decoder.push(&u32::MAX.to_be_bytes()),
        Err(TransportError::FrameTooLarge { .. }),
    ));

    assert!(decoder.is_poisoned());
}

#[test]
fn poisoned_decoder_can_reset_for_new_connection() {
    let mut decoder = FrameDecoder::new(config()).unwrap();

    assert!(decoder.push(&u32::MAX.to_be_bytes()).is_err());

    decoder.reset();

    assert!(!decoder.is_poisoned());

    let frame = encode(b"new connection");

    assert!(decoder.push(&frame).unwrap().is_ready());

    assert_eq!(
        decoder.frame(),
        Some(&b"new connection"[..]),
    );
}

#[test]
fn repeated_encoder_reuse() {
    let mut encoder = FrameEncoder::new(config()).unwrap();

    for index in 0..10_000_u64 {
        let payload = index.to_be_bytes();

        encoder.prepare(&payload).unwrap();

        assert_eq!(
            &encoder.pending_bytes()[..4],
            &8_u32.to_be_bytes(),
        );

        assert_eq!(
            &encoder.pending_bytes()[4..],
            &payload,
        );

        encoder.advance(12).unwrap();

        assert!(!encoder.is_pending());
    }
}

// ============================================================
// Deterministic randomized fragmentation
// ============================================================

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut value = self.state;

        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;

        self.state = value;

        value
    }
}

#[test]
fn randomized_fragmentation() {
    const MESSAGES: usize = 10_000;

    let mut rng = XorShift64 {
        state: 0x1234_5678_ABCD_EF01,
    };

    let mut expected = Vec::with_capacity(MESSAGES);
    let mut stream = Vec::new();

    let mut encoder = FrameEncoder::new(config()).unwrap();

    for _ in 0..MESSAGES {
        let length = (rng.next() % 512 + 1) as usize;

        let payload: Vec<u8> = (0..length)
            .map(|_| rng.next() as u8)
            .collect();

        encoder.prepare(&payload).unwrap();

        stream.extend_from_slice(
            encoder.pending_bytes()
        );

        encoder.advance(encoder.remaining()).unwrap();

        expected.push(payload);
    }

    let mut decoder = FrameDecoder::new(config()).unwrap();
    let mut actual = Vec::with_capacity(MESSAGES);

    let mut offset = 0;

    while offset < stream.len() {
        let chunk_size = (rng.next() % 2048 + 1) as usize;

        let end = (offset + chunk_size).min(stream.len());

        consume(
            &mut decoder,
            &stream[offset..end],
            &mut actual,
        );

        offset = end;
    }

    assert_eq!(actual, expected);
    assert!(decoder.finish().is_ok());
}

// ============================================================
// High-volume frame reconstruction
// ============================================================

#[test]
fn one_million_frames() {
    const MESSAGES: u64 = 1_000_000;

    let mut encoder = FrameEncoder::new(config()).unwrap();
    let mut decoder = FrameDecoder::new(config()).unwrap();

    for sequence in 0..MESSAGES {
        let payload = sequence.to_be_bytes();

        encoder.prepare(&payload).unwrap();

        let status = decoder
            .push(encoder.pending_bytes())
            .unwrap();

        assert!(status.is_ready());
        assert_eq!(status.consumed(), 12);

        assert_eq!(
            decoder.frame(),
            Some(&payload[..]),
        );

        decoder.release();

        encoder.advance(12).unwrap();
    }

    assert!(decoder.finish().is_ok());
}

