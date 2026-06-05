# Video and streaming

SimDeck streams live device video to the browser. Local sessions default to high quality. Remote or constrained sessions can trade detail for lower CPU and latency.

iOS simulator H.264 uses VideoToolbox for hardware encoding and x264 for software encoding.

WebRTC streams also include simulator audio. The browser menu exposes a Sound
toggle so viewers can keep playback muted until they want to hear the device.
H.264 WebSocket fallback remains video-only.

## When encoding runs

SimDeck starts encoding when a browser stream needs H.264 frames. The server
requests an initial keyframe to answer the WebRTC or H.264 WebSocket viewer,
then keeps a shared refresh pump active while frame subscribers exist.

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
| `full`        | Local browser on a fast Mac             |
| `balanced`    | Good local quality with less bandwidth  |
| `economy`     | Remote browser or busy machine          |
| `low`         | Slower Wi-Fi or shared hosts            |
| `tiny`        | Pull request previews and low bandwidth |
| `ci-software` | Virtualized CI Macs                     |

The browser also has stream controls for transport, resolution, FPS, and refresh.

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

When multiple simulator streams run at the same time, `auto` keeps one active
stream on the hardware encoder path and routes additional active auto streams to
software encoding. This avoids saturating the shared VideoToolbox hardware
encoder while preserving explicit `--video-codec hardware` behavior.

For very constrained software sessions:

```sh
simdeck service restart --video-codec software --low-latency
```

## WebRTC and fallback

The browser tries WebRTC first. If WebRTC cannot render a frame, the UI can fall back to H.264 over WebSocket when the browser supports WebCodecs.

Audio is carried on the WebRTC path using a browser-native Opus track. On
macOS 14.2 and newer, SimDeck uses Core Audio process taps over the selected
simulator or emulator process tree, then routes that tap through a private
aggregate device into the WebRTC audio track. If macOS has not granted system
audio recording access, video still streams and the server logs the
audio-capture failure. While the tap is being read, Core Audio mutes the tapped
simulator process at the hardware output; browser playback is controlled by the
Sound toggle. Android emulators launched by SimDeck are started with host audio
enabled, so restart older no-audio emulator processes before testing Android
sound.

Force a mode while debugging:

```text
http://127.0.0.1:4310?stream=webrtc
http://127.0.0.1:4310?stream=h264
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
