# Packet Format

The Simdeck WebTransport video stream uses a small fixed-header binary packet format. Every packet has the same shape regardless of codec.

## Layout

| Offset | Size | Field               | Notes                                                                                     |
| ------ | ---- | ------------------- | ----------------------------------------------------------------------------------------- |
| `0`    | `1`  | `version`           | Protocol version. Currently `1`.                                                          |
| `1`    | `1`  | `flags`             | Bitmask. See [Flags](#flags).                                                             |
| `2`    | `2`  | `reserved`          | Always `0`. Reserved for future use.                                                      |
| `4`    | `8`  | `frameSequence`     | Big-endian `u64`. Strictly monotonic per session.                                         |
| `12`   | `8`  | `timestampUs`       | Big-endian `u64`. Monotonic, in microseconds.                                             |
| `20`   | `4`  | `width`             | Big-endian `u32`. Frame width in pixels.                                                  |
| `24`   | `4`  | `height`            | Big-endian `u32`. Frame height in pixels.                                                 |
| `28`   | `4`  | `descriptionLength` | Big-endian `u32`. Bytes of codec description that follow the header.                      |
| `32`   | `4`  | `dataLength`        | Big-endian `u32`. Bytes of compressed video that follow the description.                  |
| `36`   | `N`  | `description`       | Optional codec config blob. May be empty.                                                 |
| `36+N` | `M`  | `data`              | Compressed video data. May be empty for keyframes that only carry a configuration update. |

The header is always exactly 36 bytes. Both `descriptionLength` and `dataLength` can be zero. The total packet length is `36 + descriptionLength + dataLength`.

## Flags

| Bit | Constant             | Meaning                                                              |
| --- | -------------------- | -------------------------------------------------------------------- |
| `0` | `FLAG_KEYFRAME`      | Frame is a keyframe (an IDR for H.264, an IDR or CRA for HEVC).      |
| `1` | `FLAG_CONFIG`        | A codec description blob is present.                                 |
| `2` | `FLAG_DISCONTINUITY` | The previous packet was dropped — the client must flush its decoder. |

Other bits are reserved and must be ignored by the client.

## Description blob

When `FLAG_CONFIG` is set the description bytes carry codec-specific configuration:

- **HEVC.** Concatenated VPS, SPS, and PPS NAL units in Annex-B form (`00 00 00 01` start codes).
- **H.264 hardware.** Concatenated SPS and PPS NAL units in Annex-B form.
- **H.264 software.** SPS and PPS as Annex-B start-code units.

A keyframe almost always also has `FLAG_CONFIG` so a client connecting mid-stream can build a fresh decoder without waiting for the next configuration update.

## Frame data

The `data` segment is the actual compressed frame:

- **HEVC.** Annex-B HEVC bitstream.
- **H.264 (hardware or software).** Annex-B H.264 bitstream.

There is no codec inside the packet to identify the codec — the `codec` field in the [`ControlHello`](/api/webtransport#control-hello) message at session start tells you which decoder to wire up.

## Sequence numbers and timestamps

`frameSequence` increments by one for each frame the encoder produces. The client should use it to detect drops and to validate the discontinuity flag.

`timestampUs` is monotonic but does not necessarily match wall-clock time — it is meant for relative scheduling and pacing, not for synchronising with anything outside the stream.

## Decoding a packet (pseudocode)

```ts
function readPacket(view: DataView, offset: number) {
  const version = view.getUint8(offset);
  if (version !== 1) {
    throw new Error(`Unsupported packet version ${version}`);
  }

  const flags = view.getUint8(offset + 1);
  const sequence = view.getBigUint64(offset + 4, false);
  const timestampUs = view.getBigUint64(offset + 12, false);
  const width = view.getUint32(offset + 20, false);
  const height = view.getUint32(offset + 24, false);
  const descriptionLength = view.getUint32(offset + 28, false);
  const dataLength = view.getUint32(offset + 32, false);
  const headerEnd = offset + 36;

  const description = new Uint8Array(view.buffer, headerEnd, descriptionLength);
  const data = new Uint8Array(
    view.buffer,
    headerEnd + descriptionLength,
    dataLength,
  );

  return {
    version,
    isKeyframe: (flags & 0x01) !== 0,
    hasConfig: (flags & 0x02) !== 0,
    discontinuity: (flags & 0x04) !== 0,
    sequence,
    timestampUs,
    width,
    height,
    description,
    data,
  };
}
```

## Versioning

`PACKET_VERSION` is currently `1`. The server bumps this version when the wire layout changes in any incompatible way. Clients should:

- Read the protocol version from `GET /api/health` (`webTransport.packetVersion`) at startup.
- Refuse to process packets whose `version` byte does not match the negotiated version.
- Surface an upgrade message rather than guessing how to parse the new layout.
