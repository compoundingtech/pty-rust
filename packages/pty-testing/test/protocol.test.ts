import { describe, expect, it } from "vitest";

import {
  HEADER_SIZE,
  MAX_PACKET_LENGTH,
  MessageType,
  PacketReader,
  decodeExit,
  decodeGeometry,
  encodeAttach,
  encodeData,
  encodeResize,
} from "../src/protocol.js";

describe("the wire format", () => {
  it("writes a header of a type and a big-endian length", () => {
    const frame = encodeData("hi");
    expect(frame.readUInt8(0)).toBe(MessageType.Data);
    expect(frame.readUInt32BE(1)).toBe(2);
    expect(frame.subarray(HEADER_SIZE).toString()).toBe("hi");
  });

  it("round-trips a size through attach and resize", () => {
    for (const frame of [encodeAttach(24, 80), encodeResize(24, 80)]) {
      expect(decodeGeometry(frame.subarray(HEADER_SIZE))).toEqual({ rows: 24, cols: 80 });
    }
  });

  it("reads an exit status, including a negative one", () => {
    const payload = Buffer.alloc(4);
    payload.writeInt32BE(-1, 0);
    expect(decodeExit(payload)).toBe(-1);
  });

  it("waits for the rest of a message that arrives in pieces", () => {
    const reader = new PacketReader();
    const whole = encodeData("split-me");
    expect(reader.feed(whole.subarray(0, 3))).toEqual([]);
    expect(reader.feed(whole.subarray(3, 7))).toEqual([]);
    const packets = reader.feed(whole.subarray(7));
    expect(packets).toHaveLength(1);
    expect(packets[0].payload.toString()).toBe("split-me");
  });

  it("reads several messages out of one chunk, in order", () => {
    const reader = new PacketReader();
    const packets = reader.feed(
      Buffer.concat([encodeData("one"), encodeData("two"), encodeData("three")]),
    );
    expect(packets.map((p) => p.payload.toString())).toEqual(["one", "two", "three"]);
  });

  it("refuses a length above the cap instead of buffering it", () => {
    const header = Buffer.alloc(HEADER_SIZE);
    header.writeUInt8(MessageType.Data, 0);
    header.writeUInt32BE(MAX_PACKET_LENGTH + 1, 1);
    expect(() => new PacketReader().feed(header)).toThrow(/exceeds maximum/);
  });
});
