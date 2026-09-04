//! Wire protocol between the `pty` client and the per-session daemon. Port of
//! the pty project's `src/protocol.ts`.
//!
//! Frame: `[type: u8][length: u32 BE][payload: length bytes]`.

use std::io::{self, Read};

/// Message types (byte tag on the wire). Unknown bytes are preserved as
/// [`MessageType::Unknown`] so a peer's newer message types pass through the
/// framing unharmed (matching the TS reader, which keeps the numeric type).
/// Values 8 and 9 are reserved for independent protocol extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    /// Terminal data (bidirectional).
    Data,
    /// Client → Server: attaching with terminal size.
    Attach,
    /// Client → Server: detach. Machine stream → caller: intentional detach.
    Detach,
    /// Client → Server: terminal resized.
    Resize,
    /// Server → Client: process exited.
    Exit,
    /// Server → Client: screen buffer replay on attach.
    Screen,
    /// Client → Server: read-only attach (no input, no resize).
    Peek,
    /// Bidirectional: request/response for JSON stats.
    Status,
    /// Server → Client: effective shared rows/cols (wire value 10).
    Geometry,
    /// An unrecognized wire byte, preserved verbatim.
    Unknown(u8),
}

impl MessageType {
    /// Convert a raw wire byte to a message type (total — unknown bytes become
    /// [`MessageType::Unknown`]).
    pub fn from_u8(b: u8) -> MessageType {
        match b {
            0 => MessageType::Data,
            1 => MessageType::Attach,
            2 => MessageType::Detach,
            3 => MessageType::Resize,
            4 => MessageType::Exit,
            5 => MessageType::Screen,
            6 => MessageType::Peek,
            7 => MessageType::Status,
            10 => MessageType::Geometry,
            other => MessageType::Unknown(other),
        }
    }

    /// The wire byte for this message type.
    pub fn as_u8(self) -> u8 {
        match self {
            MessageType::Data => 0,
            MessageType::Attach => 1,
            MessageType::Detach => 2,
            MessageType::Resize => 3,
            MessageType::Exit => 4,
            MessageType::Screen => 5,
            MessageType::Peek => 6,
            MessageType::Status => 7,
            MessageType::Geometry => 10,
            MessageType::Unknown(b) => b,
        }
    }
}

/// A decoded packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub type_: MessageType,
    pub payload: Vec<u8>,
}

impl Packet {
    /// Re-frame this packet exactly as it arrived.
    pub fn encode(&self) -> Vec<u8> {
        encode_packet(self.type_, &self.payload)
    }
}

const HEADER_SIZE: usize = 5;

/// Cap on a legitimate packet length (32 MiB), matching the TS implementation.
pub const MAX_PACKET_LENGTH: usize = 32 * 1024 * 1024;

/// The message a peer sees when it declares a length above
/// [`MAX_PACKET_LENGTH`] (TS `PacketTooLargeError.message`).
fn too_large_message(length: usize) -> String {
    format!("Packet length {length} exceeds maximum {MAX_PACKET_LENGTH}")
}

/// Encode a packet.
pub fn encode_packet(type_: MessageType, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(HEADER_SIZE + payload.len());
    out.push(type_.as_u8());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Encode terminal DATA.
pub fn encode_data(data: &[u8]) -> Vec<u8> {
    encode_packet(MessageType::Data, data)
}

fn size_payload(rows: u16, cols: u16) -> [u8; 4] {
    let r = rows.to_be_bytes();
    let c = cols.to_be_bytes();
    [r[0], r[1], c[0], c[1]]
}

/// `rows u16BE, cols u16BE, cell_width u16BE, cell_height u16BE`: the size
/// payload with the client's cell pixel metrics appended.
///
/// The four extra bytes are an *optional suffix*, which is what makes this
/// safe to send to any peer: every reader of a size payload takes rows and
/// cols from the first four bytes and the frame carries its own length, so a
/// daemon that predates this (the Node one included) reads the size it
/// always read and ignores the rest.
fn size_cell_payload(rows: u16, cols: u16, cell_width: u16, cell_height: u16) -> [u8; 8] {
    let s = size_payload(rows, cols);
    let w = cell_width.to_be_bytes();
    let h = cell_height.to_be_bytes();
    [s[0], s[1], s[2], s[3], w[0], w[1], h[0], h[1]]
}

/// Encode an ATTACH with a terminal size (4-byte payload).
pub fn encode_attach(rows: u16, cols: u16) -> Vec<u8> {
    encode_packet(MessageType::Attach, &size_payload(rows, cols))
}

/// Encode an ATTACH that also declares the client's cell pixel size
/// (8-byte payload; see [`decode_cell`]).
pub fn encode_attach_with_cell(rows: u16, cols: u16, cell_width: u16, cell_height: u16) -> Vec<u8> {
    encode_packet(
        MessageType::Attach,
        &size_cell_payload(rows, cols, cell_width, cell_height),
    )
}

/// Encode a DETACH.
pub fn encode_detach() -> Vec<u8> {
    encode_packet(MessageType::Detach, &[])
}

/// Encode a RESIZE.
pub fn encode_resize(rows: u16, cols: u16) -> Vec<u8> {
    encode_packet(MessageType::Resize, &size_payload(rows, cols))
}

/// Encode a RESIZE that also declares the client's cell pixel size.
pub fn encode_resize_with_cell(rows: u16, cols: u16, cell_width: u16, cell_height: u16) -> Vec<u8> {
    encode_packet(
        MessageType::Resize,
        &size_cell_payload(rows, cols, cell_width, cell_height),
    )
}

/// Encode a GEOMETRY (effective shared rows/cols, server → client).
pub fn encode_geometry(rows: u16, cols: u16) -> Vec<u8> {
    encode_packet(MessageType::Geometry, &size_payload(rows, cols))
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

/// The cell pixel size a client appended to an ATTACH or RESIZE payload, or
/// `None` when it sent the plain 4-byte size or a degenerate zero.
///
/// Cell metrics are the client's to know — they come from its font, on its
/// host — so a session daemon can only be told. `None` means nobody has, and
/// the reader keeps its own deterministic fallback.
pub fn decode_cell(payload: &[u8]) -> Option<(u16, u16)> {
    if payload.len() < 8 {
        return None;
    }
    let width = u16::from_be_bytes([payload[4], payload[5]]);
    let height = u16::from_be_bytes([payload[6], payload[7]]);
    (width > 0 && height > 0).then_some((width, height))
}

/// Decode a GEOMETRY payload (rows, cols); same layout and fallback as
/// [`decode_size`].
pub fn decode_geometry(payload: &[u8]) -> (u16, u16) {
    decode_size(payload)
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

    /// Feed bytes; return any complete packets. Returns an error (and empties
    /// the buffer, so a later feed cannot continue past the bad header) if a
    /// peer declares a length exceeding [`MAX_PACKET_LENGTH`].
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
                    too_large_message(length),
                ));
            }
            if self.buffer.len() < HEADER_SIZE + length {
                break;
            }
            let payload = self.buffer[HEADER_SIZE..HEADER_SIZE + length].to_vec();
            // Unknown types are preserved (Unknown(byte)), matching the TS reader.
            packets.push(Packet {
                type_: MessageType::from_u8(type_byte),
                payload,
            });
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
            too_large_message(length),
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    Ok(Some(Packet {
        type_: MessageType::from_u8(type_byte),
        payload,
    }))
}
