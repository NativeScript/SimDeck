# Health & Metrics

Two endpoints expose every observable signal SimDeck collects: `GET /api/health` for the bootstrap surface and `GET /api/metrics` for the running counters.

## `GET /api/health`

Returns the static bootstrap information the browser client needs to open a WebTransport session, plus a freshness timestamp.

```json
{
  "ok": true,
  "httpPort": 4310,
  "wtPort": 4311,
  "timestamp": 1714094761.234,
  "videoCodec": "hevc",
  "webTransport": {
    "urlTemplate": "https://127.0.0.1:4311/wt/simulators/{udid}",
    "certificateHash": {
      "algorithm": "sha-256",
      "value": "3f...e9"
    },
    "packetVersion": 1
  }
}
```

| Field                                | Notes                                                                                         |
| ------------------------------------ | --------------------------------------------------------------------------------------------- |
| `ok`                                 | Always `true` if the route is reachable. Network failures are signalled by HTTP errors.       |
| `httpPort` / `wtPort`                | Numeric ports for the HTTP and WebTransport servers. WebTransport is always `httpPort + 1`.   |
| `timestamp`                          | Server-side `time.now()` as a fractional Unix epoch in seconds.                               |
| `videoCodec`                         | Active encoder. One of `hevc`, `h264`, `h264-software`. See [Video Pipeline](/guide/video).   |
| `webTransport.urlTemplate`           | URL with a `{udid}` placeholder for the simulator stream. Substitute and dial.                |
| `webTransport.certificateHash.value` | SHA-256 of the server's self-signed cert. Pin via `serverCertificateHashes` in the WT client. |
| `webTransport.packetVersion`         | The current binary packet protocol version. Clients should refuse to parse unknown versions.  |

The certificate is regenerated every time the server restarts. A client that caches the hash should refetch `/api/health` after any disconnection.

## `GET /api/metrics`

Returns a snapshot of every server-side counter and the rolling buffer of client-reported stats:

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
  "avg_send_queue_depth": 0.91,
  "max_send_queue_depth": 2,
  "latest_first_frame_ms": 412,
  "client_streams": [
    {
      "clientId": "browser-ABC",
      "kind": "viewport",
      "udid": "9D7E5BB7-...",
      "timestampMs": 1714094761234.0,
      "codec": "hevc",
      "width": 1170,
      "height": 2532,
      "decodedFps": 59.7,
      "renderedFps": 59.7,
      "droppedFps": 0.0,
      "latestRenderMs": 6.2
    }
  ]
}
```

### Counter glossary

| Counter                    | Increments when…                                                                        |
| -------------------------- | --------------------------------------------------------------------------------------- |
| `frames_encoded`           | The native bridge produces a frame.                                                     |
| `keyframes_encoded`        | The encoder emits a keyframe (always a subset of `frames_encoded`).                     |
| `frames_sent`              | A frame is written to a WebTransport client.                                            |
| `frames_dropped_server`    | A client is too slow and the broadcast channel skips frames for them.                   |
| `keyframe_requests`        | The transport hub asks the encoder for a fresh keyframe (e.g. on reconnect or refresh). |
| `active_streams`           | Currently open WebTransport sessions.                                                   |
| `subscribers_connected`    | Lifetime count of WebTransport sessions opened.                                         |
| `subscribers_disconnected` | Lifetime count of WebTransport sessions closed.                                         |
| `avg_send_queue_depth`     | Running average of broadcast channel pressure.                                          |
| `max_send_queue_depth`     | Peak broadcast channel pressure.                                                        |
| `latest_first_frame_ms`    | First-frame latency for the most recent connect, in milliseconds.                       |

### Client stream stats

`client_streams` is a rolling buffer of the most recent reports a client posted to `POST /api/client-stream-stats`. The server keeps the last 48 entries per `(clientId, kind)` pair.

The browser client uses these to render its in-app diagnostics overlay and to size its decoder workers. Every field is optional except `clientId` and `kind`; see [`ClientStreamStats`](https://github.com/DjDeveloperr/SimDeck/blob/main/server/src/metrics/counters.rs) for the full schema.

## Submitting client stats

```http
POST /api/client-stream-stats
Content-Type: application/json

{
  "clientId": "browser-ABC",
  "kind": "viewport",
  "udid": "9D7E5BB7-...",
  "codec": "hevc",
  "width": 1170,
  "height": 2532,
  "decodedFps": 59.7,
  "droppedFps": 0.0,
  "latestRenderMs": 6.2
}
```

Required fields:

- `clientId` — any stable identifier you pick.
- `kind` — what slice of the client is reporting (`viewport`, `decoder`, `renderer`, …).

Anything else is optional but typed; unknown fields are rejected.

A successful submission returns:

```json
{ "ok": true }
```

## `GET /api/client-stream-stats`

Returns the same `clientStreams` array that `GET /api/metrics` includes, in case you only want the client-side view:

```json
{ "clientStreams": [ ... ] }
```
