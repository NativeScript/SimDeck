# Video and streaming

SimDeck streams live device video to the browser. Local sessions default to full-resolution 60 fps. Remote or constrained sessions can trade detail for lower CPU and latency.

iOS simulator H.264 uses VideoToolbox for hardware encoding and x264 for software encoding.
Android emulator H.264 uses the emulator `-share-vid` display surface. SimDeck reads BGRA frames from the `videmulator<console-port>` shared memory region and encodes them on the Mac, so normal Android live video stays on the native shared display path.

## When encoding runs

SimDeck starts encoding when a browser stream needs H.264 frames. For iOS, the
server requests an initial keyframe to answer the WebRTC viewer, then keeps a
shared refresh pump active while frame subscribers exist.
For Android, SimDeck starts emulators with `-share-vid`, maps the shared display
region, and feeds changed BGRA frames into the native host H.264 encoder.
SimDeck-owned Android boots also default to `-gpu host`, matching the native
emulator app's accelerated renderer while staying in headless shared-video mode.

The browser reports whether the page and stream canvas are foreground. When all
known viewers are hidden or the last frame subscriber disconnects, the native
session pauses encoder input and releases the active compression session. A
visible viewer, explicit refresh, or stream reconnect asks for a fresh keyframe.

## Pick a stream quality

Start with the default:

```sh
simdeck
```

Lower quality when the stream stutters, the machine is under load, or you are using a remote browser:

```sh
simdeck service restart --stream-quality low
simdeck service restart --stream-quality tiny
simdeck service restart --stream-quality ci-software
```

Common profiles:

| Profile       | Use it for                              |
| ------------- | --------------------------------------- |
| `full`        | Default local full-resolution 60 fps    |
| `smooth`      | Full-size 60 fps with lower bitrate     |
| `balanced`    | Good local quality with less bandwidth  |
| `economy`     | Remote browser or busy machine          |
| `low`         | Slower Wi-Fi or shared hosts            |
| `tiny`        | Pull request previews and low bandwidth |
| `ci-software` | Virtualized CI Macs                     |

The browser also has stream controls for transport, resolution, FPS, and refresh.

## Pick an Android GPU mode

SimDeck-owned Android emulator boots use host GPU acceleration by default:

```sh
simdeck service restart --android-gpu host
```

Use `auto` to let the Android emulator choose the renderer. Use
`swiftshader_indirect`, `swiftshader`, `software`, `lavapipe`, or `swangle` only
when host rendering is unstable on a specific machine.

## Pick a codec

```sh
simdeck service restart --video-codec auto
simdeck service restart --video-codec hardware
simdeck service restart --video-codec software
```

| Codec      | Use it for                                                                          |
| ---------- | ----------------------------------------------------------------------------------- |
| `auto`     | Normal use. SimDeck can move between hardware and software as needed.               |
| `hardware` | Dedicated local machines where VideoToolbox hardware H.264 is reliable.             |
| `software` | x264 software H.264 for CI, screen recording conflicts, or hardware encoder stalls. |

The codec setting controls simulator host encoding. Android emulator streams use
the same service codec by default for shared display frames; set
`SIMDECK_ANDROID_VIDEO_CODEC=auto`, `hardware`, or `software` before starting the
service only when you need an Android-specific encoder override. Stream quality
controls the encoded Android frame size.

When multiple simulator streams run at the same time, `auto` keeps one active
stream on the hardware encoder path and routes additional active auto streams to
software encoding. This avoids saturating the shared VideoToolbox hardware
encoder while preserving explicit `--video-codec hardware` behavior.

For very constrained software sessions:

```sh
simdeck service restart --video-codec software --low-latency
```

## WebRTC

The browser uses WebRTC for live video. SimDeck no longer exposes a separate H.264 WebSocket video transport.

Force a mode while debugging:

```text
http://127.0.0.1:4310?stream=webrtc
```

## Remote browsers

For another browser on the same network, see [LAN access](/guide/lan-access).

For routed remote access, use a tunnel or relay you trust. If your network requires TURN for WebRTC, set these before starting SimDeck:

```sh
SIMDECK_WEBRTC_ICE_SERVERS=turns:turn.example.com:5349?transport=tcp \
SIMDECK_WEBRTC_ICE_USERNAME=simdeck \
SIMDECK_WEBRTC_ICE_CREDENTIAL=secret \
SIMDECK_WEBRTC_ICE_TRANSPORT_POLICY=relay \
simdeck service start --video-codec software --stream-quality low
```

## Stream diagnostics

Check health:

```sh
curl http://127.0.0.1:4310/api/health
```

Check counters:

```sh
curl http://127.0.0.1:4310/api/metrics
```

Signals worth watching:

| Signal                             | Meaning                                                    |
| ---------------------------------- | ---------------------------------------------------------- |
| `latest_first_frame_ms`            | How long the most recent viewer waited for the first frame |
| `frames_dropped_server`            | The server skipped frames to keep the stream fresh         |
| `keyframe_requests`                | The client or server requested stream recovery             |
| `stream_pipeline_resets`           | Encoder resets after the last viewer disconnects           |
| `encoders[].encoder.overloadState` | Encoder pressure: `nominal`, `strained`, or `overloaded`   |

## Stuck stream checklist

1. Click refresh in the browser toolbar.
2. Restart with software encoding:

   ```sh
   simdeck service restart --video-codec software
   ```

3. Lower stream quality:

   ```sh
   simdeck service restart --stream-quality low
   ```

4. Restart Apple's simulator service:

   ```sh
   simdeck core-simulator restart
   ```

5. See [Troubleshooting](/guide/troubleshooting#stream-is-black-or-stuck).
