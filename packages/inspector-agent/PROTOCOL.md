# Xcode Canvas Inspector Protocol

The inspector agent speaks `XCWI/0.1`: newline-delimited JSON messages over TCP. Every request and response is one UTF-8 JSON object followed by `\n`.

The protocol is intentionally JSON-RPC-like, but small enough to use from `nc` while the server-side proxy is still evolving.

## Transport

The original debug agent listens for newline-delimited JSON over TCP.

Default starting port:

```text
47370
```

If that port is already in use, the debug configuration tries the next 32
ports and listens on the first free one. Clients should discover a running
agent by probing `47370-47402` or Bonjour, then call `Inspector.getInfo` and
match `processIdentifier` to the target simulator app.

Default bind mode:

```text
127.0.0.1 only
```

The app may also advertise Bonjour service type:

```text
_xcwinspector._tcp
```

NativeScript apps can also connect out to the Rust server over WebSocket:

```text
GET /api/inspector/connect
```

For WebSocket sessions, the server sends the same JSON request envelopes to the
app and the app responds with the same response envelopes. The server registers
the app after `Inspector.getInfo` returns a `processIdentifier`.

## Envelope

Request:

```json
{ "id": 1, "method": "View.getHierarchy", "params": { "includeHidden": false } }
```

Response:

```json
{ "id": 1, "result": { "protocolVersion": "0.1", "roots": [] } }
```

Error:

```json
{
  "id": 1,
  "error": {
    "code": -32004,
    "message": "No view was found for id view:0x1234."
  }
}
```

Event:

```json
{
  "event": "Inspector.connected",
  "params": { "protocolVersion": "0.1", "framing": "ndjson" }
}
```

If the agent is started with an `authToken`, each request must include a matching top-level `token`.

## Coordinate Space

All point input and `frameInScreen` values use UIKit screen points, not pixels.

## Methods

### Runtime.ping

Checks connectivity.

```json
{ "id": 1, "method": "Runtime.ping" }
```

### Inspector.getInfo

Returns protocol version, app process metadata, display scale, coordinate space, and available methods.

```json
{ "id": 2, "method": "Inspector.getInfo" }
```

### View.getHierarchy

Returns the current hierarchy rooted at all visible windows. If the app published a framework hierarchy, such as a NativeScript logical view tree, that hierarchy is returned by default.

Params:

```json
{ "includeHidden": false, "maxDepth": 20, "source": "uikit" }
```

`source: "uikit"` forces the raw UIKit hierarchy. Without it, published app hierarchy snapshots take precedence.

Published framework nodes may include source metadata so clients can jump from
the logical hierarchy back to app source:

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

`line` and `column` are one-based when produced by the NativeScript publisher.

### View.get

Returns one view subtree by `id`. IDs are process-local and valid until the object is destroyed.

```json
{
  "id": 4,
  "method": "View.get",
  "params": { "id": "view:0x1234", "maxDepth": 2 }
}
```

### View.hitTest

Returns the topmost hit-tested view for a screen point.

```json
{
  "id": 5,
  "method": "View.hitTest",
  "params": { "x": 120, "y": 240, "maxDepth": 1 }
}
```

### View.describeAtPoint

Returns the hit view plus its ancestor chain.

```json
{ "id": 6, "method": "View.describeAtPoint", "params": { "x": 120, "y": 240 } }
```

### View.listActions

Lists safe interactions supported by a view.

```json
{ "id": 7, "method": "View.listActions", "params": { "id": "view:0x1234" } }
```

### View.perform

Performs a high-level action on a view.

Supported actions:

- `tap`
- `focus`
- `resignFirstResponder`
- `accessibilityActivate`
- `setText`
- `setValue`
- `toggle`
- `scrollBy`
- `scrollTo`

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

### View.getProperties

Returns editable runtime properties for a view.

```json
{
  "id": 11,
  "method": "View.getProperties",
  "params": { "id": "view:0x1234" }
}
```

### View.setProperty

Sets a UIKit property dynamically. This is a debug-only escape hatch. Agents
should reject unsafe property names and coerce structured UIKit values such as
`UIColor`, `CGRect`, `CGPoint`, `CGSize`, and `UIEdgeInsets`.

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

## SwiftUI

SwiftUI's value tree is not publicly enumerable at runtime. The agent therefore exposes SwiftUI in two ways:

- Automatic detection of UIKit bridge/hosting views whose runtime classes include `SwiftUI` or `UIHosting`.
- Optional source-level tags using `View.xcwInspectorTag(_:id:metadata:)`.

Tagged SwiftUI example:

```swift
Text("Checkout")
    .xcwInspectorTag("checkout-title", id: "checkout.title")
```

Tagged views appear as lightweight probe UIViews in the hierarchy with `swiftUI.isProbe = true`.
