import { describe, expect, it } from "vitest";

import {
  consumeBinaryVideoPackets,
  decoderDescriptionBytes,
  decoderDescriptionKey,
  hexToUint8Array,
  readUInt32BE,
  readUInt64BE
} from "./streamProtocol";

function encodeBinaryPacket({
  description = [],
  frameSequence,
  height,
  isKeyFrame,
  payload,
  timestampUs,
  width
}: {
  description?: number[];
  frameSequence: number;
  height: number;
  isKeyFrame: boolean;
  payload: number[];
  timestampUs: number;
  width: number;
}) {
  const flags = (isKeyFrame ? 1 : 0) | (description.length ? 2 : 0);
  const packet = new Uint8Array(36 + description.length + payload.length);
  const view = new DataView(packet.buffer);
  packet[0] = 1;
  packet[1] = flags;
  view.setBigUint64(4, BigInt(frameSequence), false);
  view.setBigUint64(12, BigInt(timestampUs), false);
  view.setUint32(20, width, false);
  view.setUint32(24, height, false);
  view.setUint32(28, description.length, false);
  view.setUint32(32, payload.length, false);
  packet.set(description, 36);
  packet.set(payload, 36 + description.length);
  return packet;
}

describe("streamProtocol", () => {
  it("reads big-endian lengths", () => {
    const bytes = new Uint8Array([0x00, 0x00, 0x01, 0x02]);
    expect(readUInt32BE(bytes, 0)).toBe(258);
  });

  it("reads big-endian u64 values", () => {
    const bytes = new Uint8Array([0, 0, 0, 0, 0, 0, 1, 2]);
    expect(readUInt64BE(bytes, 0)).toBe(258);
  });

  it("parses binary video packets and preserves raw decoder config bytes", () => {
    const packet = encodeBinaryPacket({
      description: [1, 2, 3, 4],
      frameSequence: 9,
      height: 200,
      isKeyFrame: true,
      payload: [5, 6, 7],
      timestampUs: 1234,
      width: 100
    });

    const parsed = consumeBinaryVideoPackets(packet);
    expect(parsed.packets).toHaveLength(1);
    expect(parsed.remainder).toHaveLength(0);
    expect(parsed.packets[0]?.metadata.frameSequence).toBe(9);
    expect(parsed.packets[0]?.metadata.isKeyFrame).toBe(true);
    expect(
      Array.from(decoderDescriptionBytes(parsed.packets[0]?.metadata.description) ?? [])
    ).toEqual([1, 2, 3, 4]);
    expect(Array.from(parsed.packets[0]?.payload ?? [])).toEqual([5, 6, 7]);
  });

  it("converts decoder description inputs consistently", () => {
    const bytes = new Uint8Array([1, 2, 3]);
    expect(Array.from(decoderDescriptionBytes(bytes) ?? [])).toEqual([1, 2, 3]);
    expect(decoderDescriptionKey(bytes)).toBe("1,2,3");
    expect(Array.from(decoderDescriptionBytes("AQID") ?? [])).toEqual([1, 2, 3]);
    expect(decoderDescriptionKey("AQID")).toBe("AQID");
  });

  it("parses certificate hashes from hex", () => {
    expect(Array.from(hexToUint8Array("0a10ff"))).toEqual([0x0a, 0x10, 0xff]);
  });
});
