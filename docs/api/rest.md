# REST Endpoints

The SimDeck server exposes one REST API over plain HTTP. Every route lives under `/api/`. Responses are JSON unless explicitly noted otherwise. Errors return a JSON body with `{"error": "..."}` and an appropriate HTTP status.

The served browser UI receives the generated access token automatically through a strict same-site cookie. Direct API callers must send `X-SimDeck-Token: <token>` or `Authorization: Bearer <token>`.

## Conventions

- Method casing follows REST conventions. `GET` for queries, `POST` for state changes.
- Path parameters use `{name}` notation in this reference. UDIDs come from `GET /api/simulators` (or `simdeck list`).
- Most mutation endpoints return `{ "ok": true }`; boot and shutdown return refreshed simulator metadata.
- Timestamps are numeric unless a route documents otherwise.

## Health and metrics

### `GET /api/health`

Returns server health and the active video encoder mode.

```json
{
  "ok": true,
  "httpPort": 4310,
  "timestamp": 1714094761.234,
  "videoCodec": "auto",
  "lowLatency": false,
  "webRtc": {
    "iceServers": [{ "urls": ["stun:stun.l.google.com:19302"] }],
    "iceTransportPolicy": "all"
  }
}
```

The browser client polls this endpoint at startup to detect server restarts and
to mirror the daemon's WebRTC ICE configuration.

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
  "codec": "h264",
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

```json
{ "ok": true }
```

### `POST /api/simulators/{udid}/refresh`

Forces the encoder to emit a fresh keyframe. Useful after a discontinuity or when the client decoder drifts.

```json
{ "ok": true }
```

### `POST /api/simulators/{udid}/webrtc/offer`

WebRTC transport for browser-native live video. The browser sends an SDP offer
and the server responds with an SDP answer for a receive-only H.264 video track:

```json
{
  "sdp": "v=0\r\n...",
  "type": "offer"
}
```

```json
{
  "sdp": "v=0\r\n...",
  "type": "answer"
}
```

The endpoint requires the active simulator stream to produce H.264-compatible
samples. The bundled browser client always uses this endpoint.

### `POST /api/simulators/{udid}/open-url`

Opens a URL inside the simulator:

```http
POST /api/simulators/{udid}/open-url
Content-Type: application/json

{ "url": "https://example.com" }
```

```json
{ "ok": true }
```

### `POST /api/simulators/{udid}/launch`

Launches an installed app:

```http
POST /api/simulators/{udid}/launch
Content-Type: application/json

{ "bundleId": "com.apple.Preferences" }
```

```json
{ "ok": true }
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

### `POST /api/simulators/{udid}/touch-sequence`

Replays multiple normalized touch events through one native input session:

```http
POST /api/simulators/{udid}/touch-sequence
Content-Type: application/json

{
  "events": [
    { "x": 0.5, "y": 0.7, "phase": "began", "delayMsAfter": 25 },
    { "x": 0.5, "y": 0.4, "phase": "moved", "delayMsAfter": 25 },
    { "x": 0.5, "y": 0.2, "phase": "ended" }
  ]
}
```

This is the preferred API for agent gestures because it avoids one HTTP request
per touch phase.

### `POST /api/simulators/{udid}/key`

Replays a single keyboard event by HID key code:

```http
POST /api/simulators/{udid}/key
Content-Type: application/json

{ "keyCode": 4, "modifiers": 0 }
```

`keyCode` is the HID usage value. `modifiers` is a bitmask defined by the HID input subsystem (defaults to `0`).

### `POST /api/simulators/{udid}/key-sequence`

Replays multiple HID key codes through one native input session:

```http
POST /api/simulators/{udid}/key-sequence
Content-Type: application/json

{ "keyCodes": [11, 8, 15, 15, 18], "delayMs": 5 }
```

`delayMs` defaults to `0`.

### `POST /api/simulators/{udid}/button`

Presses a hardware button:

```http
POST /api/simulators/{udid}/button
Content-Type: application/json

{ "button": "lock", "durationMs": 50 }
```

Supported button names match the CLI: `home`, `lock`, `side-button`, `siri`,
and `apple-pay`. `durationMs` defaults to `0`.

### `POST /api/simulators/{udid}/home`

Presses the home button:

```json
{ "ok": true }
```

### `POST /api/simulators/{udid}/app-switcher`

Invokes the app switcher as one server-side native action.

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

Returns the current accessibility tree. The server merges framework inspectors, the Swift in-app agent, and the native accessibility tree. Query parameters:

| `source`                     | Behaviour                                                                                              |
| ---------------------------- | ------------------------------------------------------------------------------------------------------ |
| `auto` _(default)_ / unset   | Use the most accurate source available, falling back to AX.                                            |
| `nativescript` / `ns`        | Force the NativeScript logical tree if a NativeScript inspector is connected for the foreground app.   |
| `react-native` / `rn`        | Force the React Native component tree if a React Native inspector is connected for the foreground app. |
| `swiftui` / `swift-ui`       | Force the published SwiftUI logical tree if the Swift agent root publisher is installed in the app.    |
| `uikit` / `in-app-inspector` | Force the raw UIKit hierarchy from the in-app inspector agent (NativeScript or Swift).                 |
| `native-ax` / `ax`           | Always use the native accessibility snapshot.                                                          |

| Parameter       | Default | Description                                                                                     |
| --------------- | ------- | ----------------------------------------------------------------------------------------------- |
| `maxDepth`      | `80`    | Limits returned descendants for in-app inspectors. Native AX responses are trimmed server-side. |
| `includeHidden` | `false` | Includes hidden in-app inspector views when supported by the connected inspector runtime.       |

The response always includes:

```json
{
  "roots": [...],
  "source": "nativescript|react-native|swiftui|in-app-inspector|native-ax",
  "availableSources": ["nativescript", "react-native", "swiftui", "in-app-inspector", "native-ax"],
  "fallbackReason": "...",
  "inspector": { ... }
}
```

`fallbackReason` is only present when the server could not honour the requested source.

### `GET /api/simulators/{udid}/accessibility-point?x=...&y=...`

Returns the AX-style accessibility description of the topmost element at a screen point. `x` and `y` are in UIKit screen points and must be finite, non-negative numbers.

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
| `408`  | Timed out waiting for a downstream component (encoder keyframe, AX, inspector).             |
| `500`  | Unhandled native bridge error. Always reported as JSON with the original message preserved. |
