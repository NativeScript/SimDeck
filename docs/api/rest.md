# REST API

SimDeck serves the browser UI and API from the same HTTP server. Routes under `/api/*` return JSON unless noted.

Use the CLI for most automation. Use the API when you are building a custom client, test harness, or editor integration.

## Authentication

Browser sessions loaded from the SimDeck server receive auth automatically. Direct callers should send the service token from `simdeck service status`:

```text
X-SimDeck-Token: <token>
Authorization: Bearer <token>
```

LAN browsers can pair with the printed six-digit code through:

```http
POST /api/pair
```

Successful pairing sets the browser auth cookie and also returns the access token
for native clients:

```json
{ "ok": true, "accessToken": "<token>" }
```

## Quick examples

```sh
curl -H "X-SimDeck-Token: $SIMDECK_TOKEN" \
  http://127.0.0.1:4310/api/simulators
```

```sh
curl -X POST \
  -H "Content-Type: application/json" \
  -H "X-SimDeck-Token: $SIMDECK_TOKEN" \
  -d '{"action":"openUrl","url":"https://example.com"}' \
  http://127.0.0.1:4310/api/simulators/<udid>/action
```

## Server

| Method | Path                       | Purpose                                                    |
| ------ | -------------------------- | ---------------------------------------------------------- |
| `GET`  | `/api/health`              | Server health, version-ish runtime settings, stream config |
| `GET`  | `/api/metrics`             | Video, encoder, and client stream counters                 |
| `GET`  | `/api/client-stream-stats` | Recent client stream reports                               |
| `POST` | `/api/client-stream-stats` | Submit client stream stats                                 |
| `GET`  | `/api/stream-quality`      | Current stream quality settings                            |
| `POST` | `/api/stream-quality`      | Update stream quality settings                             |

See [Health and metrics](/api/health) for details.

## Devices

| Method | Path                              | Purpose                                   |
| ------ | --------------------------------- | ----------------------------------------- |
| `GET`  | `/api/simulators`                 | List iOS Simulators and Android emulators |
| `POST` | `/api/simulators`                 | Create and boot a simulator or emulator   |
| `GET`  | `/api/simulators/create-options`  | List device types and runtimes for create |
| `GET`  | `/api/simulators/{udid}/state`    | Get one device state                      |
| `POST` | `/api/simulators/{udid}/boot`     | Boot a simulator or emulator              |
| `POST` | `/api/simulators/{udid}/shutdown` | Shut it down                              |
| `POST` | `/api/simulators/{udid}/erase`    | Erase data and settings                   |

Device IDs come from `/api/simulators`. Android IDs use the `android:` prefix.
Booted devices are listed first. Paired iPhone and Apple Watch entries include
`pairedWatchUDID` or `pairedPhoneUDID` when CoreSimulator reports a pairing.

Create requests use identifiers from `/api/simulators/create-options`. New
devices are booted before the response is returned. If an iOS simulator is
created with `pairedWatch`, the watch is created, paired, and booted too.

iOS:

```json
{
  "platform": "ios",
  "name": "iPhone Air",
  "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-Air",
  "runtimeIdentifier": "com.apple.CoreSimulator.SimRuntime.iOS-26-4",
  "pairedWatch": {
    "name": "Apple Watch Series 11 (46mm)",
    "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-11-46mm",
    "runtimeIdentifier": "com.apple.CoreSimulator.SimRuntime.watchOS-26-4"
  }
}
```

Android:

```json
{
  "platform": "android",
  "name": "Pixel_8_API_36",
  "deviceTypeIdentifier": "pixel_8",
  "runtimeIdentifier": "system-images;android-36;google_apis;arm64-v8a"
}
```

## Apps

| Method | Path                                    | Body                                                 |
| ------ | --------------------------------------- | ---------------------------------------------------- |
| `POST` | `/api/simulators/{udid}/install`        | `{ "appPath": "/path/to/App.app" }`                  |
| `POST` | `/api/simulators/{udid}/install-upload` | Raw `.ipa` or `.apk` bytes with `x-simdeck-filename` |
| `POST` | `/api/simulators/{udid}/uninstall`      | `{ "bundleId": "com.example.App" }`                  |

`install-upload` is intended for browser clients. iOS simulator uploads must be
`.ipa` archives; Android emulator uploads must be `.apk` files.
Launch apps and open URLs through `/api/simulators/{udid}/action` with
`{ "action": "launch", "bundleId": "com.example.App" }` or
`{ "action": "openUrl", "url": "https://example.com" }`.

## Camera Simulation

| Method   | Path                                   | Purpose                                   |
| -------- | -------------------------------------- | ----------------------------------------- |
| `GET`    | `/api/camera/webcams`                  | List available Mac camera sources         |
| `GET`    | `/api/simulators/{udid}/camera`        | Get daemon camera feed status             |
| `POST`   | `/api/simulators/{udid}/camera`        | Start feed and optionally relaunch an app |
| `POST`   | `/api/simulators/{udid}/camera/source` | Switch the running daemon source          |
| `DELETE` | `/api/simulators/{udid}/camera`        | Stop the daemon camera feed               |

Start request:

```json
{
  "bundleId": "com.example.App",
  "mirror": "off",
  "source": {
    "kind": "video",
    "arg": "/absolute/path/to/feed.mov"
  }
}
```

Source `kind` is `placeholder`, `image`, `video`, or `webcam`. Image and video
sources require `arg`; local files must be absolute paths. Video sources also
accept `http://`, `https://`, and `file://` URLs. `webcam` can omit `arg` to use
the first available Mac camera, or pass a camera ID/name from `/api/camera/webcams`.
`mirror` is `auto`, `on`, or `off`.

## Performance

iOS simulator app processes run as host macOS processes. These endpoints expose host-process telemetry for matching simulator app PIDs.

| Method | Path                                                 | Purpose                                                     |
| ------ | ---------------------------------------------------- | ----------------------------------------------------------- |
| `GET`  | `/api/simulators/{udid}/processes`                   | List app, extension, helper, and web-content PIDs           |
| `GET`  | `/api/simulators/{udid}/performance`                 | Current sample plus rolling CPU/memory/disk/network history |
| `GET`  | `/api/simulators/{udid}/processes/{pid}/performance` | Performance data for one simulator app process              |
| `POST` | `/api/simulators/{udid}/processes/{pid}/sample`      | Capture a short CPU stack sample with `sample`              |

Performance query parameters:

| Parameter      | Notes                                                     |
| -------------- | --------------------------------------------------------- |
| `pid=123`      | Select a process; defaults to the foreground app          |
| `windowMs=...` | History window, clamped between 10 seconds and 10 minutes |
| `seconds=3`    | Stack sample duration for `POST .../sample`               |

## Live video

| Method | Path                                  | Purpose                               |
| ------ | ------------------------------------- | ------------------------------------- |
| `POST` | `/api/simulators/{udid}/webrtc/offer` | WebRTC offer/answer stream setup      |
| `GET`  | `/api/simulators/{udid}/input`        | Input WebSocket fallback for controls |
| `GET`  | `/api/simulators/{udid}/control`      | Alias for input control WebSocket     |
| `POST` | `/api/simulators/{udid}/refresh`      | Request a fresh frame or keyframe     |

For normal clients, copy the browser behavior instead of hand-coding a raw decoder. The UI uses the WebRTC offer endpoint for live video. Android emulator IDs use the same WebRTC endpoint; their H.264 frames are produced from the emulator `-share-vid` display surface, not screenshot polling.

Minimal WebRTC request:

```json
{
  "type": "offer",
  "sdp": "v=0...",
  "streamConfig": {
    "profile": "balanced",
    "fps": 60,
    "videoCodec": "auto"
  }
}
```

Response:

```json
{
  "type": "answer",
  "sdp": "v=0..."
}
```

## Actions and input

| Method | Path                            | Body                               |
| ------ | ------------------------------- | ---------------------------------- |
| `POST` | `/api/simulators/{udid}/action` | One tagged action or batch payload |

Common action bodies:

```json
{ "action": "tap", "selector": { "text": "Continue" }, "waitTimeoutMs": 5000 }
```

```json
{
  "action": "tap",
  "selector": { "id": "com.apple.settings.screenTime" },
  "expect": { "selector": { "id": "BackButton" }, "timeoutMs": 5000 }
}
```

```json
{ "action": "back", "timeoutMs": 5000 }
```

```json
{ "action": "touch", "x": 0.5, "y": 0.5, "phase": "began" }
```

```json
{
  "action": "edgeTouch",
  "x": 0.5,
  "y": 0.98,
  "phase": "began",
  "edge": "bottom"
}
```

```json
{
  "action": "multiTouch",
  "x1": 0.35,
  "y1": 0.5,
  "x2": 0.65,
  "y2": 0.5,
  "phase": "began"
}
```

```json
{ "action": "keySequence", "keyCodes": [11, 8, 15], "delayMs": 5 }
```

```json
{ "action": "button", "button": "lock", "durationMs": 50 }
```

```json
{
  "action": "batch",
  "steps": [
    { "action": "tap", "selector": { "text": "Continue" } },
    { "action": "waitFor", "selector": { "text": "Done" }, "timeoutMs": 5000 }
  ]
}
```

Supported action tags include `tap`, `query`, `waitFor`, `assert`,
`assertNot`, `scrollUntilVisible`, `touch`, `touchSequence`, `edgeTouch`,
`multiTouch`, `swipe`, `gesture`, `type`, `key`, `keySequence`, `button`,
`crown`, `home`, `back`, `dismissKeyboard`, `appSwitcher`, `rotateLeft`,
`rotateRight`, `toggleAppearance`, `launch`, `openUrl`, `describe`, and
`batch`. Touch, edge-touch, swipe, gesture, and multi-touch coordinates are
normalized from `0.0` to `1.0`.

## UI state and inspection

| Method | Path                                                     | Purpose                         |
| ------ | -------------------------------------------------------- | ------------------------------- |
| `GET`  | `/api/simulators/{udid}/accessibility-tree`              | Current UI tree                 |
| `GET`  | `/api/simulators/{udid}/accessibility-point?x=120&y=240` | Element at a point              |
| `POST` | `/api/simulators/{udid}/action`                          | Query, wait, assert, or batch   |
| `POST` | `/api/simulators/{udid}/inspector/request`               | Call an in-app inspector method |

Tree query parameters:

| Parameter         | Values                                                                                                    |
| ----------------- | --------------------------------------------------------------------------------------------------------- |
| `source`          | `auto`, `nativescript`, `react-native`, `flutter`, `swiftui`, `uikit`, `native-ax`, `android-uiautomator` |
| `maxDepth`        | Integer depth limit                                                                                       |
| `includeHidden`   | `true` or `false`                                                                                         |
| `interactiveOnly` | `true` keeps actionable elements plus their ancestors                                                     |

Point query parameters:

| Parameter  | Values                                            |
| ---------- | ------------------------------------------------- |
| `x`, `y`   | Required screen-point coordinates                 |
| `maxDepth` | Optional integer depth limit for native AX output |

Every tree response reports the `source` used and may include a `fallbackReason`.

Selector actions accept compact accessibility selectors:

```json
{
  "selector": {
    "text": "Continue",
    "id": "continue-button",
    "elementType": "Button",
    "enabled": true,
    "regex": false
  },
  "source": "auto",
  "maxDepth": 8,
  "limit": 20
}
```

Selectors can match `text`, `id`, `label`, `value`, `elementType`, `index`, `enabled`, `checked`, `focused`, and `selected`. Set `regex: true` to use regular expression matching for string fields.
`index` uses the same zero-based traversal order behind agent refs, so CLI
`@e3` maps to API selector `{ "index": 2 }`.

`POST /api/simulators/{udid}/action` with `{ "action": "query", ... }`
returns compact matches. `waitFor` and `assert` use the same body shape for
positive checks. `assertNot` performs negative checks.

`scrollUntilVisible` scrolls and polls until a selector appears:

```json
{
  "action": "scrollUntilVisible",
  "selector": { "text": "Settings" },
  "direction": "down",
  "timeoutMs": 10000
}
```

`direction` accepts `up`, `down`, `left`, and `right`.

## DevTools and WebKit

| Method        | Path                                                        | Purpose                                             |
| ------------- | ----------------------------------------------------------- | --------------------------------------------------- |
| `GET`         | `/api/simulators/{udid}/webkit/targets`                     | Inspectable Safari or WKWebView targets             |
| `GET`         | `/api/simulators/{udid}/webkit/targets/{targetId}/socket`   | WebKit inspector WebSocket                          |
| `GET`         | `/webkit-inspector-ui/Main.html`                            | WebInspectorUI frontend                             |
| `GET`         | `/api/simulators/{udid}/devtools/targets`                   | React Native, app runtime, Metro, or Chrome targets |
| `GET`         | `/api/simulators/{udid}/devtools/targets/{targetId}/socket` | DevTools WebSocket                                  |
| `GET`         | `/chrome-devtools-ui/inspector.html`                        | Chrome DevTools frontend                            |
| `GET`, `POST` | `/api/metro/{port}/{path}`                                  | Proxied Metro HTTP resources and DevTools frontend  |

For app-owned `WKWebView` on iOS 16.4 or newer, the app must set `isInspectable = true`.

## Evidence and chrome

| Method | Path                                                         | Purpose                                        |
| ------ | ------------------------------------------------------------ | ---------------------------------------------- |
| `GET`  | `/api/simulators/{udid}/screenshot.png`                      | PNG screenshot, with `?bezel=true` for chrome  |
| `POST` | `/api/simulators/{udid}/screen-recording`                    | MP4 recording with `{ "seconds": 5 }`          |
| `POST` | `/api/simulators/{udid}/screen-recording/start`              | Start MP4 recording and return `recordingId`   |
| `POST` | `/api/simulators/{udid}/screen-recording/{recordingId}/stop` | Stop recording and return MP4                  |
| `GET`  | `/api/simulators/{udid}/pasteboard`                          | Get pasteboard text                            |
| `POST` | `/api/simulators/{udid}/pasteboard`                          | Set pasteboard text with `{ "text": "hello" }` |
| `GET`  | `/api/simulators/{udid}/logs`                                | Recent logs                                    |
| `GET`  | `/api/simulators/{udid}/chrome-profile`                      | Screen and chrome geometry                     |
| `GET`  | `/api/simulators/{udid}/chrome.png`                          | Rendered device chrome PNG                     |
| `GET`  | `/api/simulators/{udid}/chrome-button/{button}`              | Rendered button sprite                         |
| `GET`  | `/api/simulators/{udid}/screen-mask.png`                     | Rendered screen mask PNG                       |

Log query parameters:

| Parameter            | Notes                                        |
| -------------------- | -------------------------------------------- |
| `backfill=true`      | Fetch recent history instead of current tail |
| `seconds=30`         | Time window                                  |
| `limit=250`          | Max entries                                  |
| `levels=error,fault` | Filter by level                              |
| `processes=MyApp`    | Filter by process substring                  |
| `q=Loaded`           | Message text filter                          |

## Inspector runtime hub

| Method | Path                                        | Purpose                                 |
| ------ | ------------------------------------------- | --------------------------------------- |
| `GET`  | `/api/inspector/connect`                    | WebSocket for in-app runtime inspectors |
| `GET`  | `/api/inspector/poll?processIdentifier=...` | Long-poll fallback                      |
| `POST` | `/api/inspector/request`                    | Protected service-to-service relay      |
| `POST` | `/api/inspector/response`                   | Response for polled requests            |

Most clients should call `/api/simulators/{udid}/inspector/request` instead of these hub routes.

## Errors

Errors are JSON:

```json
{
  "error": {
    "message": "Unknown simulator 9D7E5BB7-..."
  }
}
```

| Status | Common cause                                         |
| ------ | ---------------------------------------------------- |
| `400`  | Bad body or query parameter                          |
| `401`  | Missing or invalid token                             |
| `404`  | Unknown simulator, target, or asset                  |
| `408`  | Timed out waiting for a device, stream, or inspector |
| `500`  | Native bridge or server error                        |
