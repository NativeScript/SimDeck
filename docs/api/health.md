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
  "serverId": "2e640c5a06a9b732",
  "advertiseHost": "192.168.1.50",
  "hostId": "5163434b8c5e3fa4",
  "hostName": "Dj-MacBook-Pro",
  "httpPort": 4310,
  "serverKind": "launchAgent",
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

| Field           | Meaning                                                   |
| --------------- | --------------------------------------------------------- |
| `ok`            | Server is alive                                           |
| `serverId`      | Stable identity for the current daemon token              |
| `advertiseHost` | Host/IP the daemon advertises for non-local clients       |
| `hostId`        | Stable hashed identity for the Mac hardware host          |
| `hostName`      | Local host name for grouping LAN/Tailscale/Bonjour URLs   |
| `httpPort`      | Port serving UI and API                                   |
| `serverKind`    | `launchAgent`, `workspace`, `foreground`, or `standalone` |
| `videoCodec`    | Requested codec mode: `auto`, `hardware`, or `software`   |
| `streamQuality` | Active stream profile and limits                          |
| `webRtc`        | ICE settings the browser should use                       |

When auth is required, the `401` JSON body still includes `serverId`, `advertiseHost`, `hostId`, `hostName`, `httpPort`, and `serverKind` so native clients can group endpoints before pairing.

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
