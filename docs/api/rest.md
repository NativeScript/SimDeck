# REST API

SimDeck serves the browser UI and API from the same HTTP server. Routes under `/api/*` return JSON unless noted.

Use the CLI for most automation. Use the API when you are building a custom client, test harness, or editor integration.

## Authentication

Browser sessions loaded from the SimDeck server receive auth automatically. Direct callers should send the daemon token from `simdeck daemon status`:

```text
X-SimDeck-Token: <token>
Authorization: Bearer <token>
```

LAN browsers can pair with the printed six-digit code through:

```http
POST /api/pair
```

## Quick Examples

```sh
curl -H "X-SimDeck-Token: $SIMDECK_TOKEN" \
  http://127.0.0.1:4310/api/simulators
```

```sh
curl -X POST \
  -H "Content-Type: application/json" \
  -H "X-SimDeck-Token: $SIMDECK_TOKEN" \
  -d '{"url":"https://example.com"}' \
  http://127.0.0.1:4310/api/simulators/<udid>/open-url
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

See [Health & Metrics](/api/health) for details.

## Devices

| Method | Path                                       | Purpose                                   |
| ------ | ------------------------------------------ | ----------------------------------------- |
| `GET`  | `/api/simulators`                          | List iOS Simulators and Android emulators |
| `GET`  | `/api/simulators/{udid}/state`             | Get one device state                      |
| `POST` | `/api/simulators/{udid}/boot`              | Boot a simulator or emulator              |
| `POST` | `/api/simulators/{udid}/shutdown`          | Shut it down                              |
| `POST` | `/api/simulators/{udid}/erase`             | Erase data and settings                   |
| `POST` | `/api/simulators/{udid}/toggle-appearance` | Toggle light/dark appearance              |

Device IDs come from `/api/simulators`. Android IDs use the `android:` prefix.

## Apps

| Method | Path                               | Body                                |
| ------ | ---------------------------------- | ----------------------------------- |
| `POST` | `/api/simulators/{udid}/install`   | `{ "appPath": "/path/to/App.app" }` |
| `POST` | `/api/simulators/{udid}/uninstall` | `{ "bundleId": "com.example.App" }` |
| `POST` | `/api/simulators/{udid}/launch`    | `{ "bundleId": "com.example.App" }` |
| `POST` | `/api/simulators/{udid}/open-url`  | `{ "url": "https://example.com" }`  |

## Live Video

| Method | Path                                  | Purpose                                |
| ------ | ------------------------------------- | -------------------------------------- |
| `POST` | `/api/simulators/{udid}/webrtc/offer` | WebRTC offer/answer stream setup       |
| `GET`  | `/api/simulators/{udid}/h264`         | H.264 WebSocket fallback               |
| `GET`  | `/api/simulators/{udid}/input`        | Input WebSocket for fallback transport |
| `GET`  | `/api/simulators/{udid}/control`      | Alias for input control WebSocket      |
| `POST` | `/api/simulators/{udid}/refresh`      | Request a fresh frame or keyframe      |

For normal clients, copy the browser behavior instead of hand-coding a raw decoder. The UI supports WebRTC first and H.264 WebSocket fallback.

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

## Input

| Method | Path                                      | Body                                       |
| ------ | ----------------------------------------- | ------------------------------------------ |
| `POST` | `/api/simulators/{udid}/tap`              | Selector or coordinate tap                 |
| `POST` | `/api/simulators/{udid}/touch`            | `{ "x": 120, "y": 240, "phase": "began" }` |
| `POST` | `/api/simulators/{udid}/touch-sequence`   | Multiple touch phases                      |
| `POST` | `/api/simulators/{udid}/key`              | `{ "keyCode": 4, "modifiers": 0 }`         |
| `POST` | `/api/simulators/{udid}/key-sequence`     | `{ "keyCodes": [11,8,15], "delayMs": 5 }`  |
| `POST` | `/api/simulators/{udid}/button`           | `{ "button": "lock", "durationMs": 50 }`   |
| `POST` | `/api/simulators/{udid}/crown`            | `{ "delta": 50 }`                          |
| `POST` | `/api/simulators/{udid}/dismiss-keyboard` | Dismiss the software keyboard              |
| `POST` | `/api/simulators/{udid}/home`             | Press Home                                 |
| `POST` | `/api/simulators/{udid}/app-switcher`     | Open app switcher                          |
| `POST` | `/api/simulators/{udid}/rotate-left`      | Rotate left                                |
| `POST` | `/api/simulators/{udid}/rotate-right`     | Rotate right                               |

Touch coordinates are screen points unless the endpoint body explicitly uses normalized values.

## UI State And Inspection

| Method | Path                                                     | Purpose                         |
| ------ | -------------------------------------------------------- | ------------------------------- |
| `GET`  | `/api/simulators/{udid}/accessibility-tree`              | Current UI tree                 |
| `GET`  | `/api/simulators/{udid}/accessibility-point?x=120&y=240` | Element at a point              |
| `POST` | `/api/simulators/{udid}/query`                           | Query tree by selector          |
| `POST` | `/api/simulators/{udid}/wait-for`                        | Wait until selector appears     |
| `POST` | `/api/simulators/{udid}/assert`                          | Assert selector exists          |
| `POST` | `/api/simulators/{udid}/batch`                           | Run multiple control steps      |
| `POST` | `/api/simulators/{udid}/inspector/request`               | Call an in-app inspector method |

Tree query parameters:

| Parameter       | Values                                                                                                    |
| --------------- | --------------------------------------------------------------------------------------------------------- |
| `source`        | `auto`, `nativescript`, `react-native`, `flutter`, `swiftui`, `uikit`, `native-ax`, `android-uiautomator` |
| `maxDepth`      | Integer depth limit                                                                                       |
| `includeHidden` | `true` or `false`                                                                                         |

Every tree response reports the `source` used and may include a `fallbackReason`.

## DevTools And WebKit

| Method | Path                                                        | Purpose                                             |
| ------ | ----------------------------------------------------------- | --------------------------------------------------- |
| `GET`  | `/api/simulators/{udid}/webkit/targets`                     | Inspectable Safari or WKWebView targets             |
| `GET`  | `/api/simulators/{udid}/webkit/targets/{targetId}/socket`   | WebKit inspector WebSocket                          |
| `GET`  | `/webkit-inspector-ui/Main.html`                            | WebInspectorUI frontend                             |
| `GET`  | `/api/simulators/{udid}/devtools/targets`                   | React Native, app runtime, Metro, or Chrome targets |
| `GET`  | `/api/simulators/{udid}/devtools/targets/{targetId}/socket` | DevTools WebSocket                                  |
| `GET`  | `/chrome-devtools-ui/inspector.html`                        | Chrome DevTools frontend                            |

For app-owned `WKWebView` on iOS 16.4 or newer, the app must set `isInspectable = true`.

## Evidence And Chrome

| Method | Path                                            | Purpose                                        |
| ------ | ----------------------------------------------- | ---------------------------------------------- |
| `GET`  | `/api/simulators/{udid}/screenshot.png`         | PNG screenshot                                 |
| `GET`  | `/api/simulators/{udid}/pasteboard`             | Get pasteboard text                            |
| `POST` | `/api/simulators/{udid}/pasteboard`             | Set pasteboard text with `{ "text": "hello" }` |
| `GET`  | `/api/simulators/{udid}/logs`                   | Recent logs                                    |
| `GET`  | `/api/simulators/{udid}/chrome-profile`         | Screen and chrome geometry                     |
| `GET`  | `/api/simulators/{udid}/chrome.png`             | Rendered device chrome PNG                     |
| `GET`  | `/api/simulators/{udid}/chrome-button/{button}` | Rendered button sprite                         |
| `GET`  | `/api/simulators/{udid}/screen-mask.png`        | Rendered screen mask PNG                       |

Log query parameters:

| Parameter            | Notes                                        |
| -------------------- | -------------------------------------------- |
| `backfill=true`      | Fetch recent history instead of current tail |
| `seconds=30`         | Time window                                  |
| `limit=250`          | Max entries                                  |
| `levels=error,fault` | Filter by level                              |
| `processes=MyApp`    | Filter by process substring                  |
| `q=Loaded`           | Message text filter                          |

## Inspector Runtime Hub

| Method | Path                                        | Purpose                                 |
| ------ | ------------------------------------------- | --------------------------------------- |
| `GET`  | `/api/inspector/connect`                    | WebSocket for in-app runtime inspectors |
| `GET`  | `/api/inspector/poll?processIdentifier=...` | Long-poll fallback                      |
| `POST` | `/api/inspector/request`                    | Protected daemon-to-daemon relay        |
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
