//! Wire protocol between the `pty` client and the per-session daemon. Port of
//! the pty project's `src/protocol.ts`.
//!
//! Frame: `[type: u8][length: u32 BE][payload: length bytes]`.

use std::io::{self, Read};

/// Message types (byte tag on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    /// Terminal data (bidirectional).
    Data = 0,
    /// Client → Server: attaching with terminal size.
    Attach = 1,
    /// Client → Server: detaching.
    Detach = 2,
    /// Client → Server: terminal resized.
    Resize = 3,
    /// Server → Client: process exited.
    Exit = 4,
    /// Server → Client: screen buffer replay on attach.
    Screen = 5,
    /// Client → Server: read-only attach (no input, no resize).
    Peek = 6,
    /// Bidirectional: request/response for JSON stats.
    Status = 7,
}

impl MessageType {
    /// Convert a raw wire byte to a message type.
    pub fn from_u8(b: u8) -> Option<MessageType> {
        Some(match b {
            0 => MessageType::Data,
            1 => MessageType::Attach,
            2 => MessageType::Detach,
            3 => MessageType::Resize,
            4 => MessageType::Exit,
            5 => MessageType::Screen,
            6 => MessageType::Peek,
            7 => MessageType::Status,
            _ => return None,
        })
    }
}

/// A decoded packet.
#[derive(Debug, Clone)]
pub struct Packet {
    pub type_: MessageType,
    pub payload: Vec<u8>,
}

const HEADER_SIZE: usize = 5;

/// Cap on a legitimate packet length (32 MiB), matching the TS implementation.
pub const MAX_PACKET_LENGTH: usize = 32 * 1024 * 1024;

/// ATTACH flag: interactive client that does not participate in PTY size
/// negotiation.
pub const ATTACH_FLAG_GEOMETRY_NEUTRAL: u8 = 0x01;

/// Encode a packet.
pub fn encode_packet(type_: MessageType, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(HEADER_SIZE + payload.len());
    out.push(type_ as u8);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Encode terminal DATA.
pub fn encode_data(data: &[u8]) -> Vec<u8> {
    encode_packet(MessageType::Data, data)
}

/// Encode an ATTACH with a terminal size.
pub fn encode_attach(rows: u16, cols: u16, geometry_neutral: bool) -> Vec<u8> {
    let mut payload = vec![0u8; if geometry_neutral { 5 } else { 4 }];
    payload[0..2].copy_from_slice(&rows.to_be_bytes());
    payload[2..4].copy_from_slice(&cols.to_be_bytes());
    if geometry_neutral {
        payload[4] = ATTACH_FLAG_GEOMETRY_NEUTRAL;
    }
    encode_packet(MessageType::Attach, &payload)
}

/// Read the optional ATTACH flag byte.
pub fn decode_attach_flags(payload: &[u8]) -> u8 {
    if payload.len() >= 5 {
        payload[4]
    } else {
        0
    }
}

/// Encode a DETACH.
pub fn encode_detach() -> Vec<u8> {
    encode_packet(MessageType::Detach, &[])
}

/// Encode a RESIZE.
pub fn encode_resize(rows: u16, cols: u16) -> Vec<u8> {
    let mut payload = vec![0u8; 4];
    payload[0..2].copy_from_slice(&rows.to_be_bytes());
    payload[2..4].copy_from_slice(&cols.to_be_bytes());
    encode_packet(MessageType::Resize, &payload)
}

/// Encode an EXIT with a process exit code.
pub fn encode_exit(code: i32) -> Vec<u8> {
    encode_packet(MessageType::Exit, &code.to_be_bytes())
}

/// Encode a PEEK request. `plain` = plain text (bit 0); `full` = full
/// scrollback (bit 1).
pub fn encode_peek(plain: bool, full: bool) -> Vec<u8> {
    let flags = (plain as u8) | ((full as u8) << 1);
    encode_packet(MessageType::Peek, &[flags])
}

/// Decode PEEK flags into `(plain, full)`.
pub fn decode_peek(payload: &[u8]) -> (bool, bool) {
    let flags = payload.first().copied().unwrap_or(0);
    (flags & 1 != 0, flags & 2 != 0)
}

/// Encode a SCREEN replay payload.
pub fn encode_screen(data: &[u8]) -> Vec<u8> {
    encode_packet(MessageType::Screen, data)
}

/// Encode a STATUS request.
pub fn encode_status() -> Vec<u8> {
    encode_packet(MessageType::Status, &[])
}

/// Encode a STATUS JSON response.
pub fn encode_status_response(json: &str) -> Vec<u8> {
    encode_packet(MessageType::Status, json.as_bytes())
}

/// Decode a size payload (rows, cols), defaulting to 24×80.
pub fn decode_size(payload: &[u8]) -> (u16, u16) {
    if payload.len() < 4 {
        return (24, 80);
    }
    let rows = u16::from_be_bytes([payload[0], payload[1]]);
    let cols = u16::from_be_bytes([payload[2], payload[3]]);
    (rows, cols)
}

/// Decode an EXIT payload, defaulting to -1.
pub fn decode_exit(payload: &[u8]) -> i32 {
    if payload.len() < 4 {
        return -1;
    }
    i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
}

/// Streaming packet parser that tolerates partial reads on a stream socket.
#[derive(Default)]
pub struct PacketReader {
    buffer: Vec<u8>,
}

impl PacketReader {
    /// Create an empty reader.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; return any complete packets. Returns an error if a peer
    /// declares a length exceeding [`MAX_PACKET_LENGTH`].
    pub fn feed(&mut self, data: &[u8]) -> io::Result<Vec<Packet>> {
        self.buffer.extend_from_slice(data);
        let mut packets = Vec::new();

        loop {
            if self.buffer.len() < HEADER_SIZE {
                break;
            }
            let type_byte = self.buffer[0];
            let length = u32::from_be_bytes([
                self.buffer[1],
                self.buffer[2],
                self.buffer[3],
                self.buffer[4],
            ]) as usize;

            if length > MAX_PACKET_LENGTH {
                self.buffer.clear();
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("packet length {length} exceeds maximum {MAX_PACKET_LENGTH}"),
                ));
            }
            if self.buffer.len() < HEADER_SIZE + length {
                break;
            }
            let payload = self.buffer[HEADER_SIZE..HEADER_SIZE + length].to_vec();
            // Unknown types are surfaced as-is only if valid; else skip framing.
            if let Some(type_) = MessageType::from_u8(type_byte) {
                packets.push(Packet { type_, payload });
            }
            self.buffer.drain(..HEADER_SIZE + length);
        }
        Ok(packets)
    }
}

/// Read exactly one packet from a blocking reader (used by simple clients).
pub fn read_packet<R: Read>(reader: &mut R) -> io::Result<Option<Packet>> {
    let mut header = [0u8; HEADER_SIZE];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let type_byte = header[0];
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if length > MAX_PACKET_LENGTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "packet too large",
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    match MessageType::from_u8(type_byte) {
        Some(type_) => Ok(Some(Packet { type_, payload })),
        None => Ok(Some(Packet {
            type_: MessageType::Data,
            payload,
        })),
    }
}
