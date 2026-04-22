import type { StreamPacket, StreamPacketMetadata } from "./streamTypes";

const BINARY_VIDEO_PACKET_HEADER_BYTES = 36;
const BINARY_VIDEO_PACKET_VERSION = 1;
const FLAG_KEYFRAME = 1 << 0;
const FLAG_CONFIG = 1 << 1;

export function appendBytes(
  existing: Uint8Array<ArrayBufferLike>,
  incoming: Uint8Array<ArrayBufferLike>
): Uint8Array<ArrayBufferLike> {
  if (!existing.length) {
    return incoming;
  }
  const combined = new Uint8Array(existing.length + incoming.length);
  combined.set(existing);
  combined.set(incoming, existing.length);
  return combined;
}

export function readUInt32BE(bytes: Uint8Array<ArrayBufferLike>, offset: number): number {
  return (
    (bytes[offset] << 24) |
    (bytes[offset + 1] << 16) |
    (bytes[offset + 2] << 8) |
    bytes[offset + 3]
  ) >>> 0;
}

export function readUInt64BE(bytes: Uint8Array<ArrayBufferLike>, offset: number): number {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return Number(view.getBigUint64(offset, false));
}

export function base64ToUint8Array(value: string): Uint8Array<ArrayBufferLike> {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

export function hexToUint8Array(value: string): Uint8Array {
  const normalized = value.trim();
  if (normalized.length % 2 !== 0) {
    throw new Error("Invalid hex string.");
  }

  const bytes = new Uint8Array(normalized.length / 2);
  for (let index = 0; index < normalized.length; index += 2) {
    bytes[index / 2] = Number.parseInt(normalized.slice(index, index + 2), 16);
  }
  return bytes;
}

export function decoderDescriptionBytes(
  value: StreamPacketMetadata["description"]
): Uint8Array | undefined {
  if (!value) {
    return undefined;
  }
  return typeof value === "string" ? base64ToUint8Array(value) : value;
}

export function decoderDescriptionKey(value: StreamPacketMetadata["description"]): string {
  if (!value) {
    return "";
  }

  if (typeof value === "string") {
    return value;
  }

  return Array.from(value).join(",");
}

export function consumeBinaryVideoPackets(
  buffer: Uint8Array<ArrayBufferLike>
): { packets: StreamPacket[]; remainder: Uint8Array<ArrayBufferLike> } {
  const packets: StreamPacket[] = [];
  let offset = 0;

  while (buffer.length - offset >= BINARY_VIDEO_PACKET_HEADER_BYTES) {
    const packetOffset = offset;
    const version = buffer[packetOffset];
    if (version !== BINARY_VIDEO_PACKET_VERSION) {
      throw new Error(`Unsupported binary video packet version ${version}.`);
    }

    const flags = buffer[packetOffset + 1] ?? 0;
    const frameSequence = readUInt64BE(buffer, packetOffset + 4);
    const timestampUs = readUInt64BE(buffer, packetOffset + 12);
    const width = readUInt32BE(buffer, packetOffset + 20);
    const height = readUInt32BE(buffer, packetOffset + 24);
    const descriptionLength = readUInt32BE(buffer, packetOffset + 28);
    const payloadLength = readUInt32BE(buffer, packetOffset + 32);
    const packetLength = BINARY_VIDEO_PACKET_HEADER_BYTES + descriptionLength + payloadLength;
    if (buffer.length - packetOffset < packetLength) {
      break;
    }

    offset += BINARY_VIDEO_PACKET_HEADER_BYTES;
    const description =
      (flags & FLAG_CONFIG) !== 0 && descriptionLength > 0
        ? buffer.subarray(offset, offset + descriptionLength)
        : undefined;
    offset += descriptionLength;
    const payload = buffer.subarray(offset, offset + payloadLength);
    offset += payloadLength;

    packets.push({
      metadata: {
        description,
        frameSequence,
        height,
        isKeyFrame: (flags & FLAG_KEYFRAME) !== 0,
        timestampUs,
        width
      },
      payload
    });
  }

  return {
    packets,
    remainder: buffer.subarray(offset)
  };
}
