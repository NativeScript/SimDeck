# Inspector Protocol

In-app inspectors use SimDeck's small JSON protocol to publish richer UI trees and handle debug actions.

Most users do not need this page. Use it when you are building or debugging an inspector runtime.

## Transports

| Transport                                                    | Used by                                                          |
| ------------------------------------------------------------ | ---------------------------------------------------------------- |
| TCP on `127.0.0.1:47370-47402`                               | Swift in-app agent                                               |
| WebSocket `/api/inspector/connect`                           | NativeScript, React Native, Flutter, and other outbound runtimes |
| Polling `/api/inspector/poll` plus `/api/inspector/response` | Fallback when WebSocket is unavailable                           |

TCP is newline-delimited JSON:

```sh
printf '{"id":1,"method":"Inspector.getInfo"}\n' | nc 127.0.0.1 47370
```

## Envelope

Request:

```json
{
  "id": 1,
  "method": "View.getHierarchy",
  "params": { "maxDepth": 4 }
}
```

Success:

```json
{
  "id": 1,
  "result": { "roots": [] }
}
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
  "params": { "protocolVersion": "0.1" }
}
```

If an inspector is started with an auth token, requests must include a matching top-level `token`.

## Core Methods

| Method                 | Purpose                                                |
| ---------------------- | ------------------------------------------------------ |
| `Runtime.ping`         | Connectivity check                                     |
| `Inspector.getInfo`    | Protocol version, app metadata, display scale, sources |
| `View.getHierarchy`    | Current UI hierarchy                                   |
| `View.get`             | One subtree by ID                                      |
| `View.hitTest`         | Topmost view at a point                                |
| `View.describeAtPoint` | Hit view plus ancestors                                |
| `View.listActions`     | Supported actions for a view                           |
| `View.perform`         | Run an action such as `tap`, `setText`, or `scrollBy`  |
| `View.getProperties`   | Editable debug properties                              |
| `View.setProperty`     | Best-effort runtime property edit                      |
| `View.evaluateScript`  | Debug script evaluation                                |

Coordinates and frames use UIKit screen points, not pixels. Multiply by `displayScale` when you need pixels.

## Hierarchy Request

```json
{
  "id": 2,
  "method": "View.getHierarchy",
  "params": {
    "includeHidden": false,
    "maxDepth": 20,
    "source": "uikit"
  }
}
```

Framework runtimes may return logical nodes with source locations:

```json
{
  "type": "Label",
  "title": "Continue",
  "sourceLocation": {
    "file": "src/app/home.component.html",
    "line": 12,
    "column": 5
  }
}
```

## Actions

```json
{
  "id": 3,
  "method": "View.perform",
  "params": {
    "id": "view:0x1234",
    "action": "tap"
  }
}
```

Common actions include `tap`, `focus`, `resignFirstResponder`, `setText`, `setValue`, `toggle`, `scrollBy`, and `scrollTo`. Support depends on the inspector runtime and selected view.

## SwiftUI Publishing

Swift apps can publish a SwiftUI root tree:

```swift
WindowGroup {
    ContentView()
        .simDeckPublishSwiftUIViewTree("ContentView", id: "app.root")
}
```

They can also tag specific SwiftUI views so the inspector can find them:

```swift
Text("Continue")
    .simDeckInspectorTag("continue-label", id: "onboarding.continue.label")
```

## HTTP Proxy Allow List

The public proxy at `POST /api/simulators/{udid}/inspector/request` allows:

- `Runtime.ping`
- `Inspector.getInfo`
- `View.get`
- `View.evaluateScript`
- `View.getHierarchy`
- `View.getProperties`
- `View.setProperty`
- `View.listActions`
- `View.perform`

For other methods, talk directly to the inspector transport.
