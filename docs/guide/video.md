# Video Pipeline

SimDeck streams the iOS Simulator over WebTransport using a binary frame protocol. It also has an experimental WebRTC data-channel path for CI hosts that cannot hardware-encode H.264 or HEVC. This page walks through the encoder choices, the keyframe handshake, and the metrics you can use to tune them.

## Codec selection

The server can encode the simulator display in four modes, picked at startup with `--video-codec`:

| Value                       | Encoder                         | When to use it                                                                                       |
| --------------------------- | ------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `hevc`                      | Hardware HEVC via VideoToolbox  | Best quality and bandwidth on modern Apple Silicon when hardware encode is available.                |
| `h264`                      | Hardware H.264 via VideoToolbox | Use when a downstream client cannot decode HEVC and hardware H.264 is available.                     |
| `h264-software` _(default)_ | Software H.264 via VideoToolbox | Compatibility fallback when hardware encode is unavailable, but full-resolution latency may be high. |
| `jpeg`                      | Software JPEG via ImageIO       | Experimental full-resolution CI path. Use with browser query `?transport=webrtc-data`.               |

You can switch at any time by restarting the server with a different flag:

```sh
simdeck daemon stop
simdeck daemon start --video-codec h264-software
```

For GitHub Actions `macos-latest` runners, prefer the experimental JPEG path:

```sh
simdeck daemon stop
simdeck daemon start --video-codec jpeg
# open the UI with ?transport=webrtc-data
```

The chosen codec is reported to clients in two places:

- The JSON `videoCodec` field on `GET /api/health`.
- The `codec` field of the [`ControlHello`](/api/webtransport#control-hello) message that the WebTransport hub sends as soon as a session attaches. The JPEG WebRTC data-channel path does not use `ControlHello`; its frame chunks carry width, height, timestamp, and frame sequence metadata.

## Keyframe handshake

When a browser opens a WebTransport session at `/wt/simulators/{udid}`:

1. The server ensures the `SimulatorSession` is started and asks the encoder for an immediate refresh.
2. It waits up to 3 seconds for the next keyframe.
3. As soon as a keyframe arrives, it writes a JSON `ControlHello` on a control unidirectional stream and the keyframe itself on a video unidirectional stream.
4. Subsequent frames stream until the client disconnects.

If the encoder cannot deliver a keyframe within 3 seconds, the server tears the session down with a clear error so the client can retry. This usually happens only when CoreSimulator is itself stuck.

## Drop and lag handling

The transport hub uses a tokio broadcast channel to fan out frames. If a slow client misses frames the hub:

1. Increments `frames_dropped_server` on the metrics counter.
2. Sets a "waiting for keyframe" flag and skips non-keyframes until a fresh one arrives.
3. Calls `request_refresh()` on the session so the encoder forces a keyframe.

Discontinuities are signalled to the client through the `FLAG_DISCONTINUITY` bit in the packet header. The client reacts by flushing its decoder queue and waiting for the next keyframe.

## Picking a codec

A few practical guidelines:

- **Start on the default for compatibility.** `h264-software` works without requiring the hardware encoder, but full-resolution latency can be high.
- **Switch to `hevc` on local Apple Silicon when hardware encode is available.** HEVC delivers the best quality-per-bit and the lowest CPU on M-series Macs.
- **Switch to `h264` when a remote client cannot decode HEVC.** Some browsers on older Apple devices are H.264-only.
- **Switch to `h264-software` when the hardware encoder stalls and you can tolerate extra latency.** macOS screen recording can monopolise the VideoToolbox HEVC encoder. If you see "encoder unavailable" errors in the server log while QuickTime or `screencapture` is active, switch to `h264-software`.
- **Switch to `jpeg` plus `?transport=webrtc-data` on virtualized CI Macs.** GitHub Actions macOS runners commonly fail hardware-required H.264/HEVC session creation, and software H.264 can be too latent at full simulator resolution. The JPEG path is stateless, full resolution, and drops stale frames instead of waiting for inter-frame video dependencies.

## JPEG tuning

`--jpeg-quality` accepts values from `0.1` to `1.0` and defaults to full quality, `1.0`. It only affects `--video-codec jpeg`.

For full quality on a CI runner, start with:

```sh
simdeck daemon start --video-codec jpeg
```

Lower it only if `/api/metrics` shows server drops or the browser diagnostics show non-zero data-channel backlog.

## Tuning with metrics

`GET /api/metrics` returns a snapshot of every counter the server keeps:

```json
{
  "frames_encoded": 12039,
  "keyframes_encoded": 17,
  "frames_sent": 11982,
  "frames_dropped_server": 21,
  "keyframe_requests": 4,
  "active_streams": 1,
  "subscribers_connected": 3,
  "subscribers_disconnected": 2,
  "max_send_queue_depth": 1,
  "latest_first_frame_ms": 412
}
```

Useful signals:

| Counter                 | What to look at                                                                   |
| ----------------------- | --------------------------------------------------------------------------------- |
| `latest_first_frame_ms` | First-frame latency for the most recent connect. Should be a few hundred ms.      |
| `frames_dropped_server` | If this climbs while a stream is open, the client cannot keep up.                 |
| `keyframe_requests`     | Goes up every time the server forces a refresh. Frequent spikes mean rough seeks. |
| `active_streams`        | Number of WebTransport sessions currently subscribed.                             |

Clients can also push their decoder/renderer stats back to the server:

```http
POST /api/client-stream-stats
Content-Type: application/json

{
  "clientId": "browser-ABC",
  "kind": "viewport",
  "codec": "hevc",
  "decodedFps": 59.7,
  "droppedFps": 0.1,
  "latestRenderMs": 6.2
}
```

The server keeps the last 48 entries per `(clientId, kind)` pair and returns them from `GET /api/client-stream-stats`. The browser client uses these to render the in-app diagnostics overlay.

## Refreshing a stuck stream

If a client suspects it has fallen too far behind, it can call:

```http
POST /api/simulators/{udid}/refresh
```

The server starts the session if needed and asks the encoder to emit a keyframe immediately. The browser client wires this to a "Refresh stream" affordance in its toolbar.
