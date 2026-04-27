# Inspector Protocol

In-app inspectors talk to SimDeck using a small newline-delimited JSON protocol called `SDI/0.1`. Both transports (TCP and WebSocket) speak the same envelope and method set, so app-side code is interchangeable between them.

## Transports

There are two equivalent transports:

### Newline-delimited TCP

The original Swift in-app agent listens on TCP. The default port is `47370`; if it is busy the agent tries the next 32 ports and listens on the first free one. Clients should probe `47370–47402` and call `Inspector.getInfo` to disambiguate.

```sh
printf '{"id":1,"method":"Inspector.getInfo"}\n' | nc 127.0.0.1 47370
```

The agent also advertises Bonjour service type `_simdeckinspector._tcp`.

### WebSocket via the server

NativeScript apps connect outbound to the SimDeck server:

```text
GET /api/inspector/connect
```

After connection the server sends `Inspector.getInfo` and waits for a response that includes a `processIdentifier`. Once that arrives, the server treats the WebSocket as the preferred transport for that PID and routes inspector requests there.

A polling fallback is available for environments without WebSocket support:

- `GET /api/inspector/poll?processIdentifier=<pid>` — long-polls for the next request, returning `204 No Content` if nothing arrives within 25 seconds.
- `POST /api/inspector/response` — submits the response body.

## Envelopes

### Request

```json
{
  "id": 1,
  "method": "View.getHierarchy",
  "params": { "includeHidden": false }
}
```

### Response

```json
{
  "id": 1,
  "result": { "protocolVersion": "0.1", "roots": [] }
}
```

### Error

```json
{
  "id": 1,
  "error": {
    "code": -32004,
    "message": "No view was found for id view:0x1234."
  }
}
```

### Event

```json
{
  "event": "Inspector.connected",
  "params": { "protocolVersion": "0.1", "framing": "ndjson" }
}
```

If the agent is started with an `authToken`, every request must include a matching top-level `token` field.

All point input and `frameInScreen` values use UIKit screen points, **not pixels**. Multiply by `displayScale` from `Inspector.getInfo` to convert to native pixels.

## Methods

The full method list. The SimDeck HTTP proxy at `POST /api/simulators/{udid}/inspector/request` only allows a curated subset (see [REST endpoints](/api/rest#post-api-simulators-udid-inspector-request)); direct TCP/WebSocket clients can call any of them.

### `Runtime.ping`

Quick connectivity check. Returns:

```json
{ "result": { "ok": true } }
```

### `Inspector.getInfo`

Returns protocol version, app process metadata, display scale, coordinate space, and the available method list. Required first call after connect.

```json
{
  "result": {
    "protocolVersion": "0.1",
    "processIdentifier": 73214,
    "bundleIdentifier": "com.example.MyApp",
    "bundleName": "MyApp",
    "displayScale": 3,
    "coordinateSpace": "uikit-screen-points",
    "appHierarchy": {
      "available": true,
      "source": "nativescript"
    }
  }
}
```

### `View.getHierarchy`

Returns the current hierarchy rooted at every visible window.

Params:

```json
{ "includeHidden": false, "maxDepth": 20, "source": "uikit" }
```

By default the agent returns the published framework hierarchy (e.g. NativeScript) when one exists. Pass `"source": "uikit"` to force the raw UIKit tree.

Published framework nodes may include `sourceLocation`:

```json
{
  "type": "Label",
  "title": "Continue",
  "sourceLocation": {
    "file": "src/app/home.component.html",
    "line": 12,
    "column": 5,
    "offset": 238
  }
}
```

`line` and `column` are one-based when produced by NativeScript or React Native development metadata.

### `View.get`

Returns one view subtree by id:

```json
{
  "id": 4,
  "method": "View.get",
  "params": { "id": "view:0x1234", "maxDepth": 2 }
}
```

IDs are process-local and valid until the underlying object is destroyed.

### `View.hitTest`

Returns the topmost hit-tested view for a screen point:

```json
{
  "id": 5,
  "method": "View.hitTest",
  "params": { "x": 120, "y": 240, "maxDepth": 1 }
}
```

### `View.describeAtPoint`

Returns the hit view plus its ancestor chain.

```json
{ "id": 6, "method": "View.describeAtPoint", "params": { "x": 120, "y": 240 } }
```

### `View.listActions`

Lists the safe interactions a view supports.

```json
{ "id": 7, "method": "View.listActions", "params": { "id": "view:0x1234" } }
```

### `View.perform`

Performs a high-level action on a view.

Supported actions: `tap`, `focus`, `resignFirstResponder`, `accessibilityActivate`, `setText`, `setValue`, `toggle`, `scrollBy`, `scrollTo`.

Examples:

```json
{
  "id": 8,
  "method": "View.perform",
  "params": { "id": "view:0x1234", "action": "tap" }
}
```

```json
{
  "id": 9,
  "method": "View.perform",
  "params": { "id": "view:0x1234", "action": "setText", "value": "hello" }
}
```

```json
{
  "id": 10,
  "method": "View.perform",
  "params": {
    "id": "view:0x1234",
    "action": "scrollBy",
    "y": 400,
    "animated": true
  }
}
```

### `View.getProperties`

Returns editable runtime properties for a view:

```json
{ "id": 11, "method": "View.getProperties", "params": { "id": "view:0x1234" } }
```

### `View.setProperty`

Sets a UIKit property dynamically. This is a debug-only escape hatch; agents reject unsafe property names and coerce structured UIKit values such as `UIColor`, `CGRect`, `CGPoint`, `CGSize`, and `UIEdgeInsets`.

```json
{
  "id": 12,
  "method": "View.setProperty",
  "params": {
    "id": "view:0x1234",
    "property": "backgroundColor",
    "value": { "$type": "UIColor", "hex": "#FF6600FF" }
  }
}
```

### `View.evaluateScript`

Evaluates a small UIKit script against a view. Used by the browser inspector to run pre-canned diagnostics.

## SwiftUI

SwiftUI's value tree is not publicly enumerable at runtime. The agent therefore exposes SwiftUI in two ways:

1. **Automatic detection.** UIKit bridge or hosting views whose runtime classes contain `SwiftUI` or `UIHosting` are reported with `swiftUI.isHost` or `swiftUI.isProbe` markers.
2. **Source-level tags.** Apps can tag SwiftUI views with `View.simDeckInspectorTag(_:id:metadata:)` from the Swift agent. Tagged views appear as lightweight probe `UIView`s with `swiftUI.isProbe = true`.

```swift
Text("Continue")
    .simDeckInspectorTag("continue-label", id: "onboarding.continue.label")
```

## Allowed proxy methods

When you call the inspector via `POST /api/simulators/{udid}/inspector/request`, the SimDeck server enforces an allow-list to keep the HTTP surface small:

- `Runtime.ping`
- `View.get`
- `View.evaluateScript`
- `View.getProperties`
- `View.setProperty`
- `View.listActions`
- `View.perform`

For anything not in this list, talk directly to the inspector over TCP or WebSocket.
