# Video Pipeline

SimDeck streams the iOS Simulator over WebRTC using browser-native H.264 video playout, with H.264 over WebSocket as the fallback for Safari and networks where peer media negotiation fails. This page walks through the encoder choices, fallback transport, keyframe handshake, and metrics you can use to tune them.

## Codec selection

The server can encode the simulator display in three modes, picked at startup with `--video-codec`:

| Value              | Encoder                              | When to use it                                                                              |
| ------------------ | ------------------------------------ | ------------------------------------------------------------------------------------------- |
| `auto` _(default)_ | VideoToolbox chooses the encoder     | Normal local and remote preview. Falls back to software when hardware encode is overloaded. |
| `hardware`         | Required hardware H.264              | Use only when the hardware encoder is known to be available.                                |
| `software`         | Software-only H.264 via VideoToolbox | Use when hardware encode stalls, is unavailable, or must be avoided.                        |

Restart the daemon to change encoder mode:

```sh
simdeck daemon restart --video-codec software
```

For slower runners, add `--low-latency` with software H.264:

```sh
simdeck daemon start --video-codec software --low-latency
```

Low-latency mode caps software H.264 at 15 fps, keeps a single in-flight frame,
scales the longest edge to 1170 pixels, and backs off FPS more aggressively when
encode pressure rises. WebRTC refresh pacing uses the same 15 fps floor so the
server does not keep waking capture/encode faster than the stream can consume.
It is CLI-only because it is meant for less capable machines where freshness
matters more than maximum smoothness.

The requested encoder mode is reported to clients in the JSON `videoCodec` field on `GET /api/health`.
In `auto`, active simulator encoder metrics also report `activeEncoderMode`.
When the hardware encoder is overloaded, `auto` rebuilds the active compression
session as software H.264, then periodically retries hardware and stays there
when the measured encode latency returns to budget.
If all current browser viewers for a simulator are backgrounded or unfocused,
the active session also switches to software H.264 until at least one viewer is
foreground again.
The browser UI exposes stream controls for encoder, FPS, transport, and resolution. H264 resolution choices are `full` (4096 px at 60 fps), `balanced` (1280 px at 60 fps), `economy` (1080 px at 30 fps), `low` (720 px at 30 fps), and `tiny` (540 px at 30 fps). Local H264 WebSocket sessions default to full resolution at 60 fps. Remote browser sessions default to software H.264, 30 fps, and adaptive quality.

## Remote WebRTC ICE

By default SimDeck advertises Google's public STUN server to both the Rust
WebRTC peer and the browser. For remote browsers, especially Safari connecting
to a GitHub Actions runner, provide a TURN server and force relay candidates:

```sh
SIMDECK_WEBRTC_ICE_SERVERS=turns:turn.example.com:5349?transport=tcp \
SIMDECK_WEBRTC_ICE_USERNAME=simdeck \
SIMDECK_WEBRTC_ICE_CREDENTIAL=secret \
SIMDECK_WEBRTC_ICE_TRANSPORT_POLICY=relay \
simdeck daemon start --video-codec software --low-latency
```

The browser reads these settings from `GET /api/health` before creating its
peer connection, so the local and remote peers use the same ICE configuration.
Use `SIMDECK_WEBRTC_ICE_TRANSPORT_POLICY=all` or leave it unset for local LAN
and localhost sessions.

## H264 WebSocket fallback

The browser UI defaults to `?stream=auto`: it tries WebRTC first and falls back
to H264 over WebSocket if a decoded frame still does not render.
For remote browser sessions, SimDeck also falls back immediately when the
browser's WebRTC offer contains no local `host` ICE candidates, which covers
Safari privacy/network settings that suppress direct candidates. The stream
settings menu includes a transport picker for Auto, WebRTC, and H264 WS. You
can also force a mode while testing:

```text
http://127.0.0.1:4310?stream=webrtc
http://127.0.0.1:4310?stream=h264
```

H264 WS uses the same native H.264 encoder as WebRTC, but sends each encoded
sample on a binary WebSocket at:

```http
GET /api/simulators/{udid}/h264?profile=full&fps=60&videoCodec=auto
```

Each message starts with a compact SimDeck header, followed by optional AVC
decoder config and the encoded sample bytes. The browser decodes with
WebCodecs, keeps only the latest decoded frame, and paints on
`requestAnimationFrame` so stale frames do not build latency. Input stays on
the separate `/api/simulators/{udid}/input` WebSocket so large video frames do
not block touch and keyboard messages. Stream-quality updates and client stats
use the WebRTC data channel or the H264 WS video socket instead of repeatedly
posting REST requests. H264 WS defaults to the `full` profile on loopback and
`auto` quality for remote sessions. H264 `Auto` starts at `full` on loopback;
remote `Auto` starts lower but can climb through the internal `smooth` step
(1170 px), `balanced`, and `full` after sustained low decode/render pressure.

```http
GET /api/simulators/{udid}/input
```

That WebSocket accepts the same normalized control JSON used by the WebRTC data
channel and coalesces high-frequency touch `moved` events.

## Keyframe handshake

When a browser connects through `/api/simulators/{udid}/webrtc/offer`:

1. The server ensures the `SimulatorSession` is started and asks the encoder for an immediate refresh.
2. It waits up to 3 seconds for the next keyframe.
3. As soon as a keyframe arrives, it answers the browser's SDP offer and starts writing H.264 samples to a WebRTC video track.
4. Subsequent frames stream until the peer connection closes.

If the encoder cannot deliver a keyframe within 3 seconds, the server tears the session down with a clear error so the client can retry. This usually happens only when CoreSimulator is itself stuck.

## Drop and lag handling

The transport hub uses a tokio broadcast channel to fan out frames. If a slow client misses frames the hub:

1. Increments `frames_dropped_server` on the metrics counter.
2. Sets a "waiting for keyframe" flag and skips non-keyframes until a fresh one arrives.
3. Calls `request_refresh()` on the session so the encoder forces a keyframe.

The WebRTC path favors freshness: stale frames are dropped and the sender requests a new keyframe after discontinuities.

## Picking a codec

A few practical guidelines:

- **Start on the default for local preview.** Browser realtime mode uses VideoToolbox H.264 with full resolution at 60 fps. Auto mode moves active sessions to software when hardware H.264 is overloaded, then retries hardware after a cooldown.
- **Backgrounded web clients use software.** The browser sends page visibility/focus over the stream control channel. If every active viewer for a simulator is hidden or unfocused, the native session uses software H.264 until a viewer returns foreground.
- **Use `--local-stream-fps` above 60 only for local high-refresh testing.** The local quality stream defaults to 60 fps; higher targets pace both capture refresh and hardware encode submission so the stream does not build delay by pushing unbounded frames.
- **Switch to `software` when the hardware encoder stalls or is unavailable.** The encoder scales the longest edge to 1600 pixels, can climb toward 60 fps, and backs off dynamically under encode latency.
- **Studio providers default to software H.264 plus `--stream-quality smooth`.** `smooth` is an internal/provider profile, not a browser picker item. It uses a 1170-pixel longest edge, allows up to 60 fps, raises the bitrate budget to reduce compression artifacts, and lets multiple provider sessions share CPU cores without depending on one hardware encoder.
- **The remote browser renders WebRTC as a native `<video>` element and H264 WS through WebCodecs.** The canvas remains for input geometry and WebCodecs presentation, and fallback mode keeps simulator controls on the WebSocket input channel.
- **Use `--stream-quality ci-software` for denser virtualized CI Macs.** This profile uses software H.264 at a 960-pixel longest edge, targets 24 fps, lowers bitrate pressure, and favors fresh frames over full-resolution sharpness.
- **Use `simdeck studio expose --video-codec hardware` only when a dedicated hardware encoder is preferable.** The normal Studio default stays on software H.264 so future multi-simulator provider hosts can scale across CPU cores.
- **Use `software --low-latency` only when you need the older extra-conservative software profile.** It caps at 15 fps, uses a single pending frame, reduces the longest edge to 1170 pixels, and backs off before software encode latency turns into seconds of stream delay.

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
| `active_streams`        | Number of WebRTC streams currently subscribed.                                    |

`encoders[].encoder.overloadState` reports native encoder pressure for each
active simulator session. `strained` means encode latency is approaching the
active frame budget; `overloaded` means smoothed latency is over budget or
multiple frames in a row exceeded the budget. For hardware H.264 this usually
means the shared VideoToolbox encoder is saturated; lower resolution/FPS or
switch to software H.264. In `auto`, `activeEncoderMode`,
`autoSoftwareFallbackActive`, `autoSoftwareFallbacks`, and
`autoHardwareRetries` show whether the active session is temporarily using
software H.264 while the hardware path cools down. `clientForeground` shows
whether any current browser viewer is visible and focused; when it is `false`,
the active encoder is software regardless of the requested mode.

Clients can also push their decoder/renderer stats back to the server. Browser
clients normally send these over the WebRTC telemetry data channel or the H264
WS stream socket; REST remains available for simple clients:

```http
POST /api/client-stream-stats
Content-Type: application/json

{
  "clientId": "browser-ABC",
  "kind": "viewport",
  "codec": "h264",
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
