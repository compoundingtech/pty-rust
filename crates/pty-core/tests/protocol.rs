//! Port of the pty project's `tests/protocol.test.ts`.

use pty_core::protocol::{
    decode_attach_flags, decode_exit, decode_size, encode_attach, encode_data, encode_detach,
    encode_exit, encode_packet, encode_resize, encode_screen, encode_status,
    encode_status_response, MessageType, PacketReader, ATTACH_FLAG_GEOMETRY_NEUTRAL,
    MAX_PACKET_LENGTH,
};

// ── encodePacket / PacketReader round-trips ──

#[test]
fn round_trips_data() {
    let mut reader = PacketReader::new();
    let packets = reader.feed(&encode_data(b"hello world")).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Data);
    assert_eq!(packets[0].payload, b"hello world");
}

#[test]
fn round_trips_attach() {
    let mut reader = PacketReader::new();
    let packets = reader.feed(&encode_attach(24, 80, false)).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Attach);
    assert_eq!(decode_size(&packets[0].payload), (24, 80));
}

#[test]
fn legacy_attach_byte_identical_and_neutral_flag() {
    let legacy = encode_attach(24, 80, false);
    assert_eq!(
        legacy,
        encode_packet(MessageType::Attach, &[0, 24, 0, 80])
    );

    let mut reader = PacketReader::new();
    let neutral = reader.feed(&encode_attach(24, 80, true)).unwrap();
    assert_eq!(neutral[0].payload, vec![0, 24, 0, 80, 1]);
    assert_eq!(decode_size(&neutral[0].payload), (24, 80));
    assert_eq!(
        decode_attach_flags(&neutral[0].payload) & ATTACH_FLAG_GEOMETRY_NEUTRAL,
        1
    );
    assert_eq!(decode_attach_flags(&[0, 24, 0, 80]), 0);
}

#[test]
fn round_trips_detach() {
    let mut reader = PacketReader::new();
    let packets = reader.feed(&encode_detach()).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Detach);
    assert_eq!(packets[0].payload.len(), 0);
}

#[test]
fn round_trips_resize() {
    let mut reader = PacketReader::new();
    let packets = reader.feed(&encode_resize(48, 120)).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(decode_size(&packets[0].payload), (48, 120));
}

#[test]
fn round_trips_exit() {
    let mut reader = PacketReader::new();
    let packets = reader.feed(&encode_exit(42)).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Exit);
    assert_eq!(decode_exit(&packets[0].payload), 42);
}

#[test]
fn round_trips_screen() {
    let mut reader = PacketReader::new();
    let screen = "\x1b[2J\x1b[H$ hello\r\nworld";
    let packets = reader.feed(&encode_screen(screen.as_bytes())).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Screen);
    assert_eq!(packets[0].payload, screen.as_bytes());
}

// ── streaming ──

#[test]
fn multiple_packets_in_one_chunk() {
    let mut reader = PacketReader::new();
    let mut buf = Vec::new();
    buf.extend_from_slice(&encode_data(b"hello"));
    buf.extend_from_slice(&encode_data(b"world"));
    buf.extend_from_slice(&encode_detach());
    let packets = reader.feed(&buf).unwrap();
    assert_eq!(packets.len(), 3);
    assert_eq!(packets[0].payload, b"hello");
    assert_eq!(packets[1].payload, b"world");
    assert_eq!(packets[2].type_, MessageType::Detach);
}

#[test]
fn packets_split_across_chunks() {
    let mut reader = PacketReader::new();
    let full = encode_data(b"hello world");
    assert_eq!(reader.feed(&full[0..3]).unwrap().len(), 0);
    assert_eq!(reader.feed(&full[3..8]).unwrap().len(), 0);
    let packets = reader.feed(&full[8..]).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].payload, b"hello world");
}

#[test]
fn packet_split_at_header_boundary() {
    let mut reader = PacketReader::new();
    let full = encode_data(b"test");
    assert_eq!(reader.feed(&full[0..5]).unwrap().len(), 0);
    let packets = reader.feed(&full[5..]).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].payload, b"test");
}

#[test]
fn empty_payload() {
    let mut reader = PacketReader::new();
    let packets = reader.feed(&encode_packet(MessageType::Detach, &[])).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Detach);
    assert_eq!(packets[0].payload.len(), 0);
}

#[test]
fn large_payloads() {
    let mut reader = PacketReader::new();
    let big = vec![b'x'; 100_000];
    let packets = reader.feed(&encode_data(&big)).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].payload, big);
}

#[test]
fn ignores_unknown_message_types() {
    let mut reader = PacketReader::new();
    let mut raw = vec![99u8];
    raw.extend_from_slice(&3u32.to_be_bytes());
    raw.extend_from_slice(b"abc");
    let packets = reader.feed(&raw).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Unknown(99));
    assert_eq!(packets[0].payload, b"abc");
}

// ── decode edge cases ──

#[test]
fn decode_size_defaults() {
    assert_eq!(decode_size(&[0, 0]), (24, 80));
    assert_eq!(decode_size(&[]), (24, 80));
}

#[test]
fn decode_exit_defaults() {
    assert_eq!(decode_exit(&[0, 0]), -1);
    assert_eq!(decode_exit(&[]), -1);
}

// ── STATUS + oversize ──

#[test]
fn round_trips_status_request() {
    let mut reader = PacketReader::new();
    let packets = reader.feed(&encode_status()).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Status);
    assert_eq!(packets[0].payload.len(), 0);
}

#[test]
fn rejects_oversize_length() {
    let mut reader = PacketReader::new();
    let mut header = vec![MessageType::Data.as_u8()];
    header.extend_from_slice(&((MAX_PACKET_LENGTH as u32) + 1).to_be_bytes());
    assert!(reader.feed(&header).is_err());
}

#[test]
fn rejects_max_uint32_length() {
    let mut reader = PacketReader::new();
    let mut header = vec![MessageType::Data.as_u8()];
    header.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
    assert!(reader.feed(&header).is_err());
}

#[test]
fn poisons_buffer_after_oversize() {
    let mut reader = PacketReader::new();
    let mut header = vec![MessageType::Data.as_u8()];
    header.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
    let _ = reader.feed(&header);
    let packets = reader.feed(&encode_data(b"hi")).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].payload, b"hi");
}

#[test]
fn round_trips_status_response() {
    let mut reader = PacketReader::new();
    let json = r#"{"name":"test","terminal":{"cols":80,"rows":24}}"#;
    let packets = reader.feed(&encode_status_response(json)).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].type_, MessageType::Status);
    assert_eq!(String::from_utf8_lossy(&packets[0].payload), json);
}
