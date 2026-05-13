# Health & Metrics

Use these endpoints to check whether a daemon is reachable and to diagnose stream performance.

## Health

```http
GET /api/health
```

Example:

```json
{
  "ok": true,
  "httpPort": 4310,
  "timestamp": 1714094761.234,
  "videoCodec": "auto",
  "lowLatency": false,
  "realtimeStream": true,
  "localStreamFps": 60,
  "streamQuality": {
    "profile": "full"
  },
  "webRtc": {
    "iceServers": [{ "urls": ["stun:stun.l.google.com:19302"] }],
    "iceTransportPolicy": "all"
  }
}
```

Important fields:

| Field           | Meaning                                                 |
| --------------- | ------------------------------------------------------- |
| `ok`            | Server is alive                                         |
| `httpPort`      | Port serving UI and API                                 |
| `videoCodec`    | Requested codec mode: `auto`, `hardware`, or `software` |
| `streamQuality` | Active stream profile and limits                        |
| `webRtc`        | ICE settings the browser should use                     |

## Metrics

```http
GET /api/metrics
```

Useful fields:

| Field                              | What to look for                             |
| ---------------------------------- | -------------------------------------------- |
| `latest_first_frame_ms`            | First-frame startup time                     |
| `frames_dropped_server`            | Server dropping stale frames to stay current |
| `keyframe_requests`                | Stream refresh or recovery activity          |
| `active_streams`                   | Open browser streams                         |
| `encoders[].encoder.overloadState` | `nominal`, `strained`, or `overloaded`       |
| `client_streams`                   | Recent browser decoder and render reports    |

If `overloadState` is `overloaded` or dropped frames keep increasing, lower stream quality or restart with software encoding:

```sh
simdeck daemon restart --video-codec software --stream-quality low
```

## Submit Client Stats

Custom clients can report their own stream stats:

```http
POST /api/client-stream-stats
Content-Type: application/json

{
  "clientId": "browser-ABC",
  "kind": "viewport",
  "udid": "9D7E5BB7-...",
  "codec": "h264",
  "decodedFps": 59.7,
  "droppedFps": 0.0,
  "latestRenderMs": 6.2
}
```

Required fields are `clientId` and `kind`.

Read only the client buffer:

```http
GET /api/client-stream-stats
```
