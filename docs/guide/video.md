# Video & Streaming

SimDeck streams live device video to the browser. Local sessions default to high quality. Remote or constrained sessions can trade detail for lower CPU and latency.

## Pick A Stream Quality

Start with the default:

```sh
simdeck
```

Lower quality when the stream stutters, the machine is under load, or you are using a remote browser:

```sh
simdeck daemon restart --stream-quality low
simdeck daemon restart --stream-quality tiny
simdeck daemon restart --stream-quality ci-software
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

## Pick A Codec

```sh
simdeck daemon restart --video-codec auto
simdeck daemon restart --video-codec hardware
simdeck daemon restart --video-codec software
```

| Codec      | Use it for                                                            |
| ---------- | --------------------------------------------------------------------- |
| `auto`     | Normal use. SimDeck can move between hardware and software as needed. |
| `hardware` | Dedicated local machines where hardware H.264 is reliable.            |
| `software` | CI, screen recording conflicts, or hardware encoder stalls.           |

For very constrained software sessions:

```sh
simdeck daemon restart --video-codec software --low-latency
```

## WebRTC And Fallback

The browser tries WebRTC first. If WebRTC cannot render a frame, the UI can fall back to H.264 over WebSocket when the browser supports WebCodecs.

Force a mode while debugging:

```text
http://127.0.0.1:4310?stream=webrtc
http://127.0.0.1:4310?stream=h264
```

## Remote Browsers

For another browser on the same network, see [LAN Access](/guide/lan-access).

For routed remote access, use a tunnel or relay you trust. If your network requires TURN for WebRTC, set these before starting SimDeck:

```sh
SIMDECK_WEBRTC_ICE_SERVERS=turns:turn.example.com:5349?transport=tcp \
SIMDECK_WEBRTC_ICE_USERNAME=simdeck \
SIMDECK_WEBRTC_ICE_CREDENTIAL=secret \
SIMDECK_WEBRTC_ICE_TRANSPORT_POLICY=relay \
simdeck daemon start --video-codec software --stream-quality low
```

## Stream Diagnostics

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
| `encoders[].encoder.overloadState` | Encoder pressure: `nominal`, `strained`, or `overloaded`   |

## Stuck Stream Checklist

1. Click refresh in the browser toolbar.
2. Restart with software encoding:

   ```sh
   simdeck daemon restart --video-codec software
   ```

3. Lower stream quality:

   ```sh
   simdeck daemon restart --stream-quality low
   ```

4. Restart Apple's simulator service:

   ```sh
   simdeck core-simulator restart
   ```

5. See [Troubleshooting](/guide/troubleshooting#stream-is-black-or-stuck).
