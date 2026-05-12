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

Returns server-side video stats, active encoder overload states, and a rolling
buffer of client-side stats. See [Video Pipeline](/guide/video#tuning-with-metrics)
for an annotated example.

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

### `GET /api/stream-quality`

Returns the active stream encoder settings and available quality profiles.

### `POST /api/stream-quality`

Updates the active stream encoder settings for newly encoded frames. Browser
clients normally send these updates on the active WebRTC data channel or H.264
WebSocket; this endpoint remains for scripts and fallback clients.

```json
{
  "videoCodec": "hardware",
  "fps": 60,
  "profile": "full"
}
```

`videoCodec` accepts `hardware` or `software` from the UI, and the API also
accepts `auto`. `fps` is clamped to the local stream range. Browser viewers show
five H.264 resolution profiles: `full` (4096 px at 60 fps), `balanced`
(1280 px at 60 fps), `economy` (1080 px at 30 fps), `low` (720 px at 30 fps),
and `tiny` (540 px at 30 fps). The API still accepts the legacy `quality`,
`fast`, `smooth`, and `ci-software` profiles for CLI/provider compatibility.
When
`profile` is provided, its resolution preset is applied; send `maxEdge` without
`profile` for a custom resolution cap.

## Simulator inventory

### `GET /api/simulators`

Returns every iOS Simulator known to the native bridge plus every Android AVD
found in the Android SDK, enriched with any session state SimDeck has attached:

```json
{
  "simulators": [
    {
      "udid": "9D7E5BB7-...",
      "name": "iPhone 15 Pro",
      "runtimeName": "iOS 18.0",
      "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-15-Pro",
      "isBooted": true,
      "platform": "ios-simulator",
      "privateDisplay": {
        "displayReady": true,
        "displayStatus": "running",
        "displayWidth": 1170,
        "displayHeight": 2532,
        "frameSequence": 8124,
        "rotationQuarterTurns": 0
      }
    }
  ]
}
```

Android emulators use IDs prefixed with `android:` and include Android metadata:

```json
{
  "udid": "android:SimDeck_Pixel_8_API_36",
  "name": "SimDeck_Pixel_8_API_36",
  "platform": "android-emulator",
  "runtimeName": "Android",
  "deviceTypeName": "Android Emulator",
  "isBooted": true,
  "android": {
    "avdName": "SimDeck_Pixel_8_API_36",
    "serial": "emulator-5554",
    "grpcPort": 8554
  }
}
```

For iOS, `privateDisplay` is `null` until a stream attaches. For Android,
SimDeck fills display size from `adb shell wm size` when the emulator is booted.

## Simulator lifecycle

### `POST /api/simulators/{udid}/boot`

Boots the simulator or Android emulator and returns the refreshed device metadata:

```json
{ "simulator": { ... } }
```

### `POST /api/simulators/{udid}/shutdown`

Tears down the live session (if any) and shuts the simulator or emulator down.

### `POST /api/simulators/{udid}/toggle-appearance`

Toggles between light and dark appearance via `simctl ui appearance` on iOS or
`cmd uimode night` on Android.

```json
{ "ok": true }
```

### `POST /api/simulators/{udid}/refresh`

Forces the iOS encoder to emit a fresh frame. For Android IDs, this route is a
no-op that returns `{ "ok": true, "stream": "screenshot" }`; Android WebRTC
keyframe requests are handled through the WebRTC control channel and RTCP
feedback.

```json
{ "ok": true }
```

### `POST /api/simulators/{udid}/webrtc/offer`

WebRTC transport for browser-native live video. The browser sends an SDP offer
and the server responds with an SDP answer for a receive-only H.264 video track:

```json
{
  "sdp": "v=0\r\n...",
  "streamConfig": {
    "fps": 60,
    "profile": "full",
    "videoCodec": "auto"
  },
  "type": "offer"
}
```

```json
{
  "sdp": "v=0\r\n...",
  "type": "answer"
}
```

For iOS, samples come from the native simulator display session and are sent as
an H.264 media track. Android loopback clients use the same endpoint and control
channels, but receive raw RGBA frames over the `simdeck-rgba` data channel.
Non-loopback Android clients receive VideoToolbox-encoded H.264.

The browser also opens `simdeck-control` and `simdeck-telemetry` data channels.
In addition to input messages, clients can request a keyframe or tune the
stream attached to that peer:

```json
{ "type": "streamControl", "forceKeyframe": true }
```

```json
{ "type": "streamQuality", "config": { "profile": "low", "fps": 30 } }
```

The telemetry channel accepts:

```json
{ "type": "clientStats", "stats": { "clientId": "browser", "kind": "webrtc" } }
```

### `GET /api/simulators/{udid}/h264`

Direct H.264 video over WebSocket for browsers that support WebCodecs but
cannot establish WebRTC media. The server sends binary messages with this
layout:

| Offset | Size | Field                                               |
| ------ | ---- | --------------------------------------------------- |
| 0      | 4    | Magic bytes `SDH1`                                  |
| 4      | 1    | Version, currently `1`                              |
| 5      | 1    | Flags: bit 0 keyframe, bit 1 decoder config present |
| 6      | 2    | Header length, big-endian                           |
| 8      | 8    | Frame sequence, big-endian                          |
| 16     | 8    | Timestamp in microseconds, big-endian               |
| 24     | 4    | Encoded width, big-endian                           |
| 28     | 4    | Encoded height, big-endian                          |
| 32     | 4    | Decoder config byte length, big-endian              |
| 36     | 4    | H.264 sample byte length, big-endian                |

The optional decoder config bytes follow the header, then the encoded H.264
sample bytes. Clients can pass initial stream settings as query parameters
(`profile`, `fps`, `videoCodec`) and can send text control messages on the same
socket:

```json
{ "type": "streamControl", "forceKeyframe": true }
```

```json
{ "type": "streamQuality", "config": { "profile": "low", "fps": 30 } }
```

```json
{ "type": "clientStats", "stats": { "clientId": "browser", "kind": "page" } }
```

Touch and keyboard input should use the separate `/api/simulators/{udid}/input`
WebSocket. The video socket is latest-frame oriented: clients should paint the
latest decoded frame locally and request a keyframe if the decoder loses sync,
rather than ACKing every rendered frame.

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

### `GET /api/simulators/{udid}/screenshot.png`

Returns a PNG screenshot for the selected device. iOS screenshots come from the
native simulator bridge; Android screenshots come from `adb exec-out screencap
-p`. The browser client uses this endpoint for still-image diagnostics and
fallbacks.

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

Supported button names match the CLI and chrome controls: `home`, `lock`,
`power`, `side-button`, `volume-up`, `volume-down`, `action`, `mute`,
`app-switcher`, `siri`, and `apple-pay`. `durationMs` defaults to `0` and is
used for press-and-hold interactions.

For live chrome interactions, send explicit button edges instead of a completed
press:

```json
{ "button": "power", "phase": "down" }
```

`phase` accepts `down`, `up`, `began`, `ended`, and `cancelled`. Chrome controls
may also pass `usagePage` and `usage` from the device profile when an exact HID
usage is available.

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
  "cornerRadius": 220,
  "buttons": [
    {
      "name": "power",
      "label": "Sleep/Wake",
      "x": 1210,
      "y": 420,
      "width": 18,
      "height": 112,
      "anchor": "right",
      "onTop": false,
      "normalOffset": { "x": -2, "y": 420 },
      "rolloverOffset": { "x": -4, "y": 420 },
      "imageName": "SideButton",
      "imageDownName": "SideButtonPressed"
    }
  ]
}
```

The browser client uses this to compose chrome around the live frame and to
render physical button sprites over or under the device body.

### `GET /api/simulators/{udid}/chrome.png`

Returns the rendered bezel as a PNG. Pass `?buttons=false` to omit physical
button sprites when the client renders them interactively. Cache headers are set
to `no-cache, no-store, must-revalidate` so changes (e.g. after a device
rotation) are picked up immediately.

### `GET /api/simulators/{udid}/chrome-button/{button}.png`

Returns a rendered physical button sprite. Pass `?pressed=true` for the
pressed-state sprite when the device profile exposes one.

## Accessibility

### `GET /api/simulators/{udid}/accessibility-tree`

Returns the current accessibility tree. The server merges framework inspectors, the Swift in-app agent, and the native accessibility tree. Query parameters:

| `source`                     | Behaviour                                                                                              |
| ---------------------------- | ------------------------------------------------------------------------------------------------------ |
| `auto` _(default)_ / unset   | Use the most accurate source available, falling back to AX.                                            |
| `nativescript` / `ns`        | Force the NativeScript logical tree if a NativeScript inspector is connected for the foreground app.   |
| `react-native` / `rn`        | Force the React Native component tree if a React Native inspector is connected for the foreground app. |
| `flutter` / `fl`             | Force the Flutter widget tree if a Flutter inspector is connected for the foreground app.              |
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
  "source": "nativescript|react-native|flutter|swiftui|in-app-inspector|native-ax",
  "availableSources": ["nativescript", "react-native", "flutter", "swiftui", "in-app-inspector", "native-ax"],
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

- `Inspector.getInfo`
- `Runtime.ping`
- `View.get`
- `View.evaluateScript`
- `View.getHierarchy`
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
    "daemonUrl": null,
    "host": "127.0.0.1",
    "port": null,
    "displayScale": 3,
    "protocolVersion": "0.1"
  }
}
```

For the full method semantics, see the [Inspector Protocol](/api/inspector-protocol).

## DevTools inspectors

SimDeck serves WebKit Remote Inspector targets and Chrome DevTools Protocol
targets separately at the API layer. The browser UI combines both sources into
one resizeable DevTools panel with a single target list.

## WebKit inspector

### `GET /api/simulators/{udid}/webkit/targets`

Discovers inspectable WebKit targets exposed by the simulator's `webinspectord`
Remote Inspector socket. These are Safari pages and app web content that WebKit
has made inspectable. On iOS 16.4 and newer, app-owned `WKWebView` instances must
set `isInspectable = true` before they appear here.

```json
{
  "udid": "4889B81C-FD88-49A9-BC1D-2087E7C451A2",
  "socketPath": "/private/var/tmp/.../com.apple.webinspectord_sim.socket",
  "targets": [
    {
      "id": "5049443a3731353731-1",
      "appId": "PID:71571",
      "appName": "Example",
      "pageId": 1,
      "title": "Example",
      "url": "https://example.com/",
      "kind": "app-web-content",
      "inspectorUrl": "/webkit-inspector-ui/Main.html?ws=127.0.0.1:4310/api/simulators/{udid}/webkit/targets/5049443a3731353731-1/socket",
      "webSocketUrl": "/api/simulators/{udid}/webkit/targets/5049443a3731353731-1/socket"
    }
  ],
  "warnings": []
}
```

`kind` is best-effort metadata:

- `safari-page` for Mobile Safari targets.
- `app-web-content` for app-owned inspectable web content.
- `web-content-proxy` for WebKit proxy targets.

This endpoint lists **inspectable WebKit targets**, not every `WKWebView` object
in UIKit. AX can still reveal visible web areas that are not inspectable.

### `GET /api/simulators/{udid}/webkit/targets/{targetId}/socket`

Upgrades to a WebSocket. The server bridges JSON Web Inspector frontend messages
to the simulator's binary-plist WebKit Remote Inspector protocol for the selected
target. The `inspectorUrl` returned by `webkit/targets` points WebInspectorUI at
this socket.

### `GET /webkit-inspector-ui/Main.html`

Serves the local macOS WebInspectorUI resources with a SimDeck browser host shim.
This is authenticated like the rest of the server routes.

## Chrome DevTools inspector

### `GET /api/simulators/{udid}/devtools/targets`

Discovers app runtime inspector sessions that can be opened with the embedded
Chrome DevTools frontend. This includes SimDeck's in-app inspector protocol for
React Native and NativeScript runtimes, UIKit/SwiftUI fallback metadata when
those are the only connected logical sources, Metro React Native DevTools
targets, and generic local Chrome Inspector targets.

```json
{
  "udid": "4889B81C-FD88-49A9-BC1D-2087E7C451A2",
  "targets": [
    {
      "id": "sdi-73214",
      "title": "React Native: Example",
      "type": "page",
      "url": "simdeck://com.example.Example",
      "description": "SimDeck React Native inspector target",
      "devtoolsFrontendUrl": "/chrome-devtools-ui/inspector.html?ws=127.0.0.1:4310/api/simulators/{udid}/devtools/targets/sdi-73214/socket",
      "webSocketDebuggerUrl": "ws://127.0.0.1:4310/api/simulators/{udid}/devtools/targets/sdi-73214/socket",
      "source": "react-native",
      "processIdentifier": 73214,
      "bundleIdentifier": "com.example.Example",
      "appName": "Example"
    }
  ],
  "warnings": []
}
```

Metro-discovered targets use `source: "react-native-metro"` and point at
`/chrome-devtools-ui/rn_fusebox.html` when the target supports the React Native
Fusebox frontend. SimDeck probes common Metro ports and matching local
Node/React Native listener ports instead of assuming one fixed port. Generic
Chrome Inspector targets use `source: "chrome-inspector"` and are discovered from
common Inspector ports such as `9222-9230` plus matching local Chrome/Node
listener ports. Proxied targets return a SimDeck `webSocketDebuggerUrl`; SimDeck
then connects to the upstream `/inspector/debug` or Chrome DevTools WebSocket.

### `GET /api/simulators/{udid}/devtools/targets/{targetId}/socket`

Upgrades to a WebSocket speaking enough of the Chrome DevTools Protocol for the
frontend to open a target, evaluate console expressions through
`View.evaluateScript`, and render the published logical hierarchy as DOM nodes.

### `GET /chrome-devtools-ui/inspector.html`

Serves the bundled Chrome DevTools frontend. SimDeck uses the React Native
debugger frontend package for these static assets and copies them into
`client/dist/chrome-devtools-ui` during the client build.

## NativeScript inspector hub

### `GET /api/inspector/connect`

Upgrades to a WebSocket. Used by the [`@nativescript/simdeck-inspector`](/inspector/nativescript) runtime to register itself as an in-app inspector.

After connection the server sends `Inspector.getInfo` and waits for a response that includes a `processIdentifier`. Once registered, the server uses this socket as the preferred transport for `accessibility-tree` and `inspector/request` calls that target the same process.

Registered WebSocket and polled inspectors are advertised in `~/.simdeck/inspectors.json` with the owning daemon URL, access token, process id, and advertised hierarchy sources. Other SimDeck daemons read this registry, validate that the process belongs to the requested simulator, and relay inspector requests through the owning daemon. Entries are refreshed while the inspector is alive and expire automatically if the daemon or app exits.

### `GET /api/inspector/poll?processIdentifier=...`

Long-poll fallback for environments where the WebSocket transport is not viable. Returns the next pending request as JSON, or `204 No Content` after 25 seconds with no work.

### `POST /api/inspector/request`

Protected daemon-to-daemon relay endpoint. A daemon uses this after discovering a matching inspector in `~/.simdeck/inspectors.json`; app runtimes and browsers should use the WebSocket/poll endpoints or `POST /api/simulators/{udid}/inspector/request` instead.

```http
POST /api/inspector/request
Content-Type: application/json
X-SimDeck-Token: <owning-daemon-token>

{
  "processIdentifier": 73214,
  "method": "Runtime.ping",
  "params": null
}
```

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
