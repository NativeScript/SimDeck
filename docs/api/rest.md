# REST Endpoints

The SimDeck server exposes one REST API over plain HTTP. Every route lives under `/api/`. Responses are JSON unless explicitly noted otherwise. Errors return a JSON body with `{"error": {"message": "..."}}` and an appropriate HTTP status.

CORS is wide open (`Access-Control-Allow-Origin: *`) so you can call the API from any browser tab on the same network.

## Conventions

- Method casing follows REST conventions. `GET` for queries, `POST` for state changes.
- Path parameters use `{name}` notation in this reference. UDIDs come from `GET /api/simulators` (or `simdeck list`).
- All control endpoints that mutate a simulator return the refreshed simulator metadata in `{ "simulator": ... }`.
- Times are reported as ISO-8601 strings unless explicitly numeric.

## Health and metrics

### `GET /api/health`

Returns server health, the WebTransport URL template, and the certificate hash the client must pin.

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

The browser client polls this endpoint at startup and again after a long disconnect to detect server restarts (which rotate the certificate).

### `GET /api/metrics`

Returns server-side video stats and a rolling buffer of client-side stats. See [Video Pipeline](/guide/video#tuning-with-metrics) for an annotated example.

### `GET /api/client-stream-stats`

Returns just the client-side stats:

```json
{ "clientStreams": [{ "clientId": "...", "kind": "viewport", ... }] }
```

### `POST /api/client-stream-stats`

Submit a stats sample from a client. The server keeps the last 48 entries per `(clientId, kind)`:

```http
POST /api/client-stream-stats
Content-Type: application/json

{
  "clientId": "browser-ABC",
  "kind": "viewport",
  "codec": "hevc",
  "width": 1170,
  "height": 2532,
  "decodedFps": 59.7,
  "droppedFps": 0.0,
  "latestRenderMs": 6.2
}
```

Required fields: `clientId` and `kind`. Every other field is optional but typed in `ClientStreamStats`.

## Simulator inventory

### `GET /api/simulators`

Returns every simulator known to the native bridge, enriched with any session state SimDeck has attached:

```json
{
  "simulators": [
    {
      "udid": "9D7E5BB7-...",
      "name": "iPhone 15 Pro",
      "runtimeName": "iOS 18.0",
      "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-15-Pro",
      "isBooted": true,
      "privateDisplay": {
        "displayReady": true,
        "displayStatus": "running",
        "displayWidth": 1170,
        "displayHeight": 2532,
        "frameSequence": 8124
      }
    }
  ]
}
```

`privateDisplay` is `null` until a stream attaches.

## Simulator lifecycle

### `POST /api/simulators/{udid}/boot`

Boots the simulator and returns the refreshed simulator metadata:

```json
{ "simulator": { ... } }
```

### `POST /api/simulators/{udid}/shutdown`

Tears down the live session (if any) and shuts the simulator down.

### `POST /api/simulators/{udid}/toggle-appearance`

Toggles between light and dark appearance via `simctl ui appearance`.

### `POST /api/simulators/{udid}/refresh`

Forces the encoder to emit a fresh keyframe. Useful after a discontinuity or when the client decoder drifts.

```json
{ "ok": true }
```

### `POST /api/simulators/{udid}/open-url`

Opens a URL inside the simulator:

```http
POST /api/simulators/{udid}/open-url
Content-Type: application/json

{ "url": "https://example.com" }
```

### `POST /api/simulators/{udid}/launch`

Launches an installed app:

```http
POST /api/simulators/{udid}/launch
Content-Type: application/json

{ "bundleId": "com.apple.Preferences" }
```

## Input

### `POST /api/simulators/{udid}/touch`

Replays a single touch event. For drags, send `began`, one or more `moved`, then `ended` (or `cancelled`).

```http
POST /api/simulators/{udid}/touch
Content-Type: application/json

{ "x": 240.0, "y": 480.0, "phase": "began" }
```

Allowed `phase` values: `began`, `moved`, `ended`, `cancelled`.

### `POST /api/simulators/{udid}/key`

Replays a single keyboard event by HID key code:

```http
POST /api/simulators/{udid}/key
Content-Type: application/json

{ "keyCode": 4, "modifiers": 0 }
```

`keyCode` is the HID usage value. `modifiers` is a bitmask defined by the HID input subsystem (defaults to `0`).

### `POST /api/simulators/{udid}/home`

Presses the home button:

```json
{ "ok": true }
```

### `POST /api/simulators/{udid}/app-switcher`

Double-presses the home button to invoke the app switcher.

### `POST /api/simulators/{udid}/rotate-left`

Rotates the simulator 90° counter-clockwise.

### `POST /api/simulators/{udid}/rotate-right`

Rotates the simulator 90° clockwise.

## Chrome rendering

### `GET /api/simulators/{udid}/chrome-profile`

Returns the bezel layout for the simulator:

```json
{
  "totalWidth": 1240,
  "totalHeight": 2602,
  "screenX": 35,
  "screenY": 35,
  "screenWidth": 1170,
  "screenHeight": 2532,
  "cornerRadius": 220
}
```

The browser client uses this to compose chrome around the live frame.

### `GET /api/simulators/{udid}/chrome.png`

Returns the rendered bezel as a PNG. Cache headers are set to `no-cache, no-store, must-revalidate` so changes (e.g. after a device rotation) are picked up immediately.

## Accessibility

### `GET /api/simulators/{udid}/accessibility-tree`

Returns the current accessibility tree. The server merges three sources: NativeScript, Swift in-app agent, and AXe. Use the `source` query parameter to ask for a specific one:

| `source`                     | Behaviour                                                                                            |
| ---------------------------- | ---------------------------------------------------------------------------------------------------- |
| `auto` _(default)_ / unset   | Use the most accurate source available, falling back to AXe.                                         |
| `nativescript` / `ns`        | Force the NativeScript logical tree if a NativeScript inspector is connected for the foreground app. |
| `uikit` / `in-app-inspector` | Force the raw UIKit hierarchy from the in-app inspector agent (NativeScript or Swift).               |
| `axe`                        | Always use the AXe accessibility snapshot.                                                           |

The response always includes:

```json
{
  "roots": [...],
  "source": "nativescript|in-app-inspector|axe",
  "availableSources": ["nativescript", "in-app-inspector", "axe"],
  "fallbackReason": "...",
  "inspector": { ... }
}
```

`fallbackReason` is only present when the server could not honour the requested source.

### `GET /api/simulators/{udid}/accessibility-point?x=...&y=...`

Returns the AXe-style accessibility description of the topmost element at a screen point. `x` and `y` are in UIKit screen points and must be finite, non-negative numbers.

## Inspector proxy

### `POST /api/simulators/{udid}/inspector/request`

Proxies a single inspector method to the active in-app inspector (NativeScript or Swift) for the simulator. This is used by the browser client to fetch view properties, list available actions, and run debug-only edits.

```http
POST /api/simulators/{udid}/inspector/request
Content-Type: application/json

{
  "method": "View.getProperties",
  "params": { "id": "view:0x1234" }
}
```

Allowed methods (the server enforces this allow-list):

- `Runtime.ping`
- `View.get`
- `View.evaluateScript`
- `View.getProperties`
- `View.setProperty`
- `View.listActions`
- `View.perform`

The response includes both the inspector's `result` and metadata about the inspector that handled the request:

```json
{
  "result": { "id": "view:0x1234", "properties": [...] },
  "inspector": {
    "bundleIdentifier": "com.example.MyApp",
    "bundleName": "MyApp",
    "transport": "websocket",
    "processIdentifier": 73214,
    "host": "127.0.0.1",
    "port": null,
    "displayScale": 3,
    "protocolVersion": "0.1"
  }
}
```

For the full method semantics, see the [Inspector Protocol](/api/inspector-protocol).

## NativeScript inspector hub

### `GET /api/inspector/connect`

Upgrades to a WebSocket. Used by the [`@nativescript/simdeck-inspector`](/inspector/nativescript) runtime to register itself as an in-app inspector.

After connection the server sends `Inspector.getInfo` and waits for a response that includes a `processIdentifier`. Once registered, the server uses this socket as the preferred transport for `accessibility-tree` and `inspector/request` calls that target the same process.

### `GET /api/inspector/poll?processIdentifier=...`

Long-poll fallback for environments where the WebSocket transport is not viable. Returns the next pending request as JSON, or `204 No Content` after 25 seconds with no work.

### `POST /api/inspector/response`

Posts a response to a previous polled request:

```http
POST /api/inspector/response
Content-Type: application/json

{
  "processIdentifier": 73214,
  "id": 12,
  "result": { "ok": true }
}
```

Pass `error` instead of `result` to deliver an error.

## Logs

### `GET /api/simulators/{udid}/logs`

Returns recent simulator logs. Without `backfill=true`, the server tails the live `os_log` stream it has already started for the simulator. With `backfill=true`, the server runs a fresh `simctl spawn ... log show` over the requested window.

| Query parameter | Default | Notes                                                                         |
| --------------- | ------- | ----------------------------------------------------------------------------- |
| `backfill`      | `false` | When `true`, fetch a one-shot history instead of streaming.                   |
| `seconds`       | `30`    | Backfill window in seconds. Clamped to `[1, 1800]`.                           |
| `limit`         | `250`   | Max entries to return. Clamped to `[1, 1000]`.                                |
| `levels`        | _none_  | Comma-separated list of log levels to keep (`debug,info,notice,error,fault`). |
| `processes`     | _none_  | Comma-separated list of process names (case-insensitive substring matches).   |
| `q`             | _none_  | Free-text filter applied to the rendered log message.                         |

```json
{
  "entries": [
    {
      "timestamp": "2026-04-23T19:14:12.123Z",
      "level": "info",
      "process": "MyApp",
      "subsystem": "com.example.MyApp",
      "category": "ui",
      "pid": 73214,
      "message": "Loaded 12 items"
    }
  ]
}
```

## Errors

Error bodies look like:

```json
{
  "error": {
    "message": "Unknown simulator 9D7E5BB7-..."
  }
}
```

| Status | Cause                                                                                       |
| ------ | ------------------------------------------------------------------------------------------- |
| `400`  | Bad request body or query parameter (e.g. missing `url`, invalid `x`/`y`).                  |
| `404`  | Unknown simulator.                                                                          |
| `408`  | Timed out waiting for a downstream component (encoder keyframe, AXe, inspector).            |
| `500`  | Unhandled native bridge error. Always reported as JSON with the original message preserved. |
