# Inspector Overview

Simdeck blends three different ways to inspect what an iOS app is rendering:

| Source                   | Coverage                                                              | When to use it                                                                                 |
| ------------------------ | --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| **AXe**                  | Any simulator app via the system accessibility stack.                 | Default fallback. Always available if [AXe](https://github.com/cameroncooke/AXe) is on `PATH`. |
| **Swift in-app agent**   | Apps that link `XcodeCanvasInspectorAgent` in DEBUG.                  | Best for native iOS apps you control.                                                          |
| **NativeScript runtime** | NativeScript apps that import `@nativescript/xcode-canvas-inspector`. | Best for NativeScript apps — exposes the logical view tree, not just UIKit.                    |

The HTTP API picks the most specific source available, falls back to the next one when something goes wrong, and tells the client which sources were available so the UI can offer a switch.

## How the server picks a source

`GET /api/simulators/{udid}/accessibility-tree` accepts a `source` query parameter:

| `source`                     | Behaviour                                                                                            |
| ---------------------------- | ---------------------------------------------------------------------------------------------------- |
| `auto` _(default)_ / unset   | Use the most accurate source available, falling back to AXe.                                         |
| `nativescript` / `ns`        | Force the NativeScript logical tree if a NativeScript inspector is connected for the foreground app. |
| `uikit` / `in-app-inspector` | Force the raw UIKit hierarchy from any in-app inspector (NativeScript or Swift agent).               |
| `axe`                        | Always use the AXe accessibility snapshot.                                                           |

The server resolves which inspector belongs to the simulator by:

1. Checking every connected NativeScript inspector for a process running on the target UDID.
2. Probing TCP `47370–47402` on `127.0.0.1` for a Swift inspector agent and matching its `processIdentifier` against the UDID's processes via `ps`.
3. Falling back to AXe when neither matches.

Every accessibility tree response includes:

```json
{
  "source": "nativescript|in-app-inspector|axe",
  "availableSources": ["nativescript", "in-app-inspector", "axe"],
  "fallbackReason": "...",
  "inspector": { "bundleIdentifier": "...", "processIdentifier": 73214 }
}
```

`fallbackReason` only appears when the requested source could not be honoured. The browser client uses it to render a banner explaining what happened.

## Choosing the right inspector

- **You own the iOS app and write Swift / Objective-C.** Link the [Swift in-app agent](/inspector/swift). It exposes the most semantic data — UIView properties, SwiftUI probes, custom actions — and lets the browser client edit values in place.
- **You ship a NativeScript app.** Use the [NativeScript runtime inspector](/inspector/nativescript). It connects outbound to the Simdeck server and publishes both the NativeScript logical tree and the underlying UIKit hierarchy.
- **You can't link anything into the app.** Stick with [AXe](/inspector/accessibility). It only sees what the iOS accessibility stack exposes, but it works for every app.

## Editing properties

When the in-app inspector is reachable, the browser client can:

- List actions a view supports (`tap`, `setText`, `scrollBy`, …) and trigger them.
- Read editable runtime properties (`alpha`, `backgroundColor`, `clipsToBounds`, `text`, …).
- Set those properties live, with structured value coercion for `UIColor`, `CGRect`, `CGPoint`, `CGSize`, and `UIEdgeInsets`.

These calls go through the HTTP proxy at `POST /api/simulators/{udid}/inspector/request`, which only accepts a small allow-list of methods. Direct TCP and WebSocket clients can use the full method list documented in the [Inspector Protocol](/api/inspector-protocol).
