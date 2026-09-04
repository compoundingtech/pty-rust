/**
 * The session socket's wire format.
 *
 * Every message is a five-byte header — one type byte, then a 32-bit
 * big-endian length — followed by that many bytes of payload.
 *
 * This is written from the format, not from the pty package's own code, so
 * that a test failure here means the two disagree rather than that one file
 * was copied from the other.
 */

/** What a message is. The numbers are the wire values. */
export enum MessageType {
  Data = 0,
  Attach = 1,
  Detach = 2,
  Resize = 3,
  Exit = 4,
  Screen = 5,
  Peek = 6,
  Status = 7,
  /** Server to client: the size every client ended up with. */
  Geometry = 10,
}

export const HEADER_SIZE = 5;

/**
 * The largest payload that will be read. A peer that declares more than this
 * is not buffered; the connection is dropped.
 */
export const MAX_PACKET_LENGTH = 32 * 1024 * 1024;

export interface Packet {
  type: number;
  payload: Buffer;
}

function frame(type: MessageType, payload: Buffer): Buffer {
  const out = Buffer.allocUnsafe(HEADER_SIZE + payload.length);
  out.writeUInt8(type, 0);
  out.writeUInt32BE(payload.length, 1);
  payload.copy(out, HEADER_SIZE);
  return out;
}

/** Terminal input, from a client to the session. */
export function encodeData(data: string | Buffer): Buffer {
  return frame(MessageType.Data, Buffer.isBuffer(data) ? data : Buffer.from(data, "utf8"));
}

/** Join a session at this size. */
export function encodeAttach(rows: number, cols: number): Buffer {
  const payload = Buffer.allocUnsafe(4);
  payload.writeUInt16BE(rows, 0);
  payload.writeUInt16BE(cols, 2);
  return frame(MessageType.Attach, payload);
}

/** Leave, without ending the session. */
export function encodeDetach(): Buffer {
  return frame(MessageType.Detach, Buffer.alloc(0));
}

/** Ask for a new size. The daemon gives every client the smallest asked for. */
export function encodeResize(rows: number, cols: number): Buffer {
  const payload = Buffer.allocUnsafe(4);
  payload.writeUInt16BE(rows, 0);
  payload.writeUInt16BE(cols, 2);
  return frame(MessageType.Resize, payload);
}

/** Read the screen without joining. `plain` drops the escape sequences. */
export function encodePeek(plain: boolean, full: boolean): Buffer {
  const payload = Buffer.allocUnsafe(2);
  payload.writeUInt8(plain ? 1 : 0, 0);
  payload.writeUInt8(full ? 1 : 0, 1);
  return frame(MessageType.Peek, payload);
}

/** Ask for the session's figures as JSON. */
export function encodeStatus(): Buffer {
  return frame(MessageType.Status, Buffer.alloc(0));
}

/** The rows and cols out of a GEOMETRY payload. */
export function decodeGeometry(payload: Buffer): { rows: number; cols: number } {
  if (payload.length < 4) return { rows: 0, cols: 0 };
  return { rows: payload.readUInt16BE(0), cols: payload.readUInt16BE(2) };
}

/** The status out of an EXIT payload. */
export function decodeExit(payload: Buffer): number {
  return payload.length >= 4 ? payload.readInt32BE(0) : 0;
}

/**
 * Turns a byte stream into whole messages.
 *
 * Feed it whatever arrives; it keeps a partial message until the rest of it
 * does. A declared length above the cap throws, because the alternative is
 * buffering whatever a peer claims to be sending.
 */
export class PacketReader {
  #buffer: Buffer = Buffer.alloc(0);

  feed(chunk: Buffer): Packet[] {
    this.#buffer = this.#buffer.length === 0 ? chunk : Buffer.concat([this.#buffer, chunk]);
    const packets: Packet[] = [];
    for (;;) {
      if (this.#buffer.length < HEADER_SIZE) break;
      const type = this.#buffer.readUInt8(0);
      const length = this.#buffer.readUInt32BE(1);
      if (length > MAX_PACKET_LENGTH) {
        this.#buffer = Buffer.alloc(0);
        throw new Error(`Packet length ${length} exceeds maximum ${MAX_PACKET_LENGTH}`);
      }
      if (this.#buffer.length < HEADER_SIZE + length) break;
      packets.push({
        type,
        payload: this.#buffer.subarray(HEADER_SIZE, HEADER_SIZE + length),
      });
      this.#buffer = this.#buffer.subarray(HEADER_SIZE + length);
    }
    return packets;
  }
}
