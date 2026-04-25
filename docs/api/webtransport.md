# WebTransport

Live video and the per-session control handshake travel over WebTransport. The hub listens on `port + 1` (so `4311` when HTTP is on `4310`) and serves one path per simulator UDID.

## URL template

The exact URL is reported by `GET /api/health`. The template looks like:

```text
https://<advertise-host>:<wt-port>/wt/simulators/{udid}?simdeckToken=<token>
```

Replace `{udid}` with the simulator UDID from `GET /api/simulators`. The `simdeckToken` query parameter is included in the authenticated `/api/health` response and is required by the WebTransport server.

## Certificate pinning

The server generates a self-signed certificate at startup and exposes its SHA-256 hash on `GET /api/health`:

```json
{
  "webTransport": {
    "certificateHash": {
      "algorithm": "sha-256",
      "value": "3f...e9"
    }
  }
}
```

Pass the hash to the WebTransport API as `serverCertificateHashes`:

```ts
const health = await fetch("/api/health").then((r) => r.json());
const url = health.webTransport.urlTemplate.replace("{udid}", udid);
const transport = new WebTransport(url, {
  serverCertificateHashes: [
    {
      algorithm: "sha-256",
      value: hexToUint8Array(health.webTransport.certificateHash.value),
    },
  ],
});
await transport.ready;
```

## Session handshake

Once a WebTransport session connects to `/wt/simulators/{udid}`, the server:

1. Resolves the per-UDID `SimulatorSession` (lazily creating one if needed).
2. Calls `ensure_started_async()` to spin up the encoder.
3. Calls `request_refresh()` to force a fresh keyframe.
4. Waits up to 3 seconds for the keyframe.
5. Opens a unidirectional **control** stream and writes one JSON object: the `ControlHello`.
6. Opens a unidirectional **video** stream and starts writing binary frame packets.

Both streams are unidirectional from server to client. The client never writes back over WebTransport — control commands round-trip through the HTTP API instead.

If no keyframe arrives within 3 seconds, the server returns an error and the connection is torn down. The client should retry after a short backoff.

## Control hello

The first stream the server opens carries one JSON object:

```json
{
  "version": 1,
  "simulatorUdid": "9D7E5BB7-...",
  "width": 1170,
  "height": 2532,
  "codec": "hevc",
  "packetFormat": "binary-video-v1"
}
```

| Field              | Notes                                                                      |
| ------------------ | -------------------------------------------------------------------------- |
| `version`          | The packet protocol version. Always matches `webTransport.packetVersion`.  |
| `simulatorUdid`    | Echo of the URL UDID for client-side sanity checks.                        |
| `width` / `height` | Frame dimensions in pixels.                                                |
| `codec`            | One of `hevc`, `h264`, `h264-software`, or absent if the encoder is unset. |
| `packetFormat`     | Always `binary-video-v1` for the current protocol.                         |

The control stream then closes — no further messages are sent on it.

## Video stream

The video stream is a continuous sequence of frame packets. Each packet is three concatenated regions:

| Region        | Size             | Purpose                                                              |
| ------------- | ---------------- | -------------------------------------------------------------------- |
| `header`      | 36 bytes         | Fixed-size metadata, including the lengths of the two regions below. |
| `description` | optional N bytes | Codec configuration blob (present on keyframes / config updates).    |
| `frame data`  | M bytes          | Compressed video for this frame.                                     |

Both `N` and `M` are big-endian `u32`s in the header, so the client can frame the next packet without any additional state.

For the exact byte layout, see [Packet Format](/api/packet-format).

## Reconnect behaviour

When a client disconnects (browser tab closed, network drop, server restart) and reconnects:

- The hub starts a new session from scratch — there is no resume token.
- The server forces a fresh keyframe and re-emits a control hello.
- Client decoder state should be flushed before the first packet of the new session is decoded.

If the server has been restarted in the meantime, the certificate hash on `/api/health` will have changed. Always refetch `/api/health` before retrying.

## Backpressure and discontinuities

Each WebTransport session subscribes to a server-side broadcast channel. If the client is too slow to keep up, the channel drops the oldest pending frames and the hub:

1. Increments `frames_dropped_server` on the metrics counter.
2. Sets a "waiting for keyframe" flag and ignores subsequent non-keyframes until a fresh one arrives.
3. Calls `request_refresh()` so the encoder forces a keyframe.
4. Sets `FLAG_DISCONTINUITY` on the next packet header so the client knows to flush.

If the client wants to force a keyframe explicitly (for example after a tab regains focus), it can call `POST /api/simulators/{udid}/refresh`.
