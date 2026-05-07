# Inspector Overview

SimDeck blends three different ways to inspect what an iOS app is rendering:

| Source                   | Coverage                                                                                                                 | When to use it                                                                   |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| **Native AX**            | Any simulator app via the Simulator accessibility stack.                                                                 | Default fallback.                                                                |
| **Swift in-app agent**   | Apps that link `SimDeckInspectorAgent` in DEBUG.                                                                         | Best for native iOS apps you control.                                            |
| **NativeScript runtime** | NativeScript apps that import `@nativescript/simdeck-inspector`.                                                         | Best for NativeScript apps — exposes the logical view tree, not just UIKit.      |
| **React Native runtime** | React Native apps that import `react-native-simdeck`.                                                                    | Best for React Native apps — exposes components and Metro source locations.      |
| **Flutter runtime**      | Flutter apps that import `simdeck_flutter_inspector`.                                                                    | Best for Flutter apps — exposes widgets, render frames, semantics actions, and creation locations. |
| **DevTools panel**       | Safari/WebKit targets, Metro React Native targets, Chrome Inspector targets, and SimDeck app runtime inspector sessions. | Best when you want a familiar browser inspector for app runtimes or web content. |

The HTTP API picks the most specific source available, falls back to the next one when something goes wrong, and tells the client which sources were available so the UI can offer a switch.

## How the server picks a source

`GET /api/simulators/{udid}/accessibility-tree` accepts a `source` query parameter:

| `source`                     | Behaviour                                                                                              |
| ---------------------------- | ------------------------------------------------------------------------------------------------------ |
| `auto` _(default)_ / unset   | Use the most accurate source available, falling back to AX.                                            |
| `nativescript` / `ns`        | Force the NativeScript logical tree if a NativeScript inspector is connected for the foreground app.   |
| `react-native` / `rn`        | Force the React Native component tree if a React Native inspector is connected for the foreground app. |
| `flutter` / `fl`             | Force the Flutter widget tree if a Flutter inspector is connected for the foreground app.              |
| `swiftui` / `swift-ui`       | Force the published SwiftUI logical tree if the Swift agent root publisher is installed in the app.    |
| `uikit` / `in-app-inspector` | Force the raw UIKit hierarchy from any in-app inspector (NativeScript or Swift agent).                 |
| `native-ax` / `ax`           | Always use the native accessibility snapshot.                                                          |

The server resolves which inspector belongs to the simulator by:

1. Checking every connected WebSocket runtime inspector for a process running on the target UDID.
2. Reading `~/.simdeck/inspectors.json` for inspectors published by other SimDeck daemons, validating the process against the target UDID, and relaying requests through the owning daemon.
3. Probing TCP `47370–47402` on `127.0.0.1` for a Swift inspector agent and matching its `processIdentifier` against the UDID's processes via `ps`.
4. Falling back to AX when none match.

Every accessibility tree response includes:

```json
{
  "source": "nativescript|react-native|flutter|swiftui|in-app-inspector|native-ax",
  "availableSources": [
    "nativescript",
    "react-native",
    "flutter",
    "swiftui",
    "in-app-inspector",
    "native-ax"
  ],
  "fallbackReason": "...",
  "inspector": { "bundleIdentifier": "...", "processIdentifier": 73214 }
}
```

`fallbackReason` only appears when the requested source could not be honoured. The browser client uses it to render a banner explaining what happened.

## Choosing the right inspector

- **You own the iOS app and write Swift / Objective-C.** Link the [Swift in-app agent](/inspector/swift). It exposes the most semantic data — UIView properties, SwiftUI view trees/probes, custom actions — and lets the browser client edit values in place.
- **You ship a NativeScript app.** Use the [NativeScript runtime inspector](/inspector/nativescript). It connects outbound to the SimDeck server and publishes both the NativeScript logical tree and the underlying UIKit hierarchy.
- **You ship a React Native app.** Use the [React Native runtime inspector](/inspector/react-native). It connects outbound to the SimDeck server and publishes the React component tree with dev-mode source locations.
- **You want DevTools for React Native, Safari/WebKit, Chrome Inspector, or a connected app runtime.** Use the DevTools toolbar toggle. It shows one target list for WebKit Remote Inspector targets, Metro React Native targets, local Chrome Inspector ports, and connected React Native or NativeScript SimDeck inspector sessions.
- **You ship a Flutter app.** Use the [Flutter runtime inspector](/inspector/flutter). It connects outbound to the SimDeck server and publishes the Flutter widget tree with render frames, semantics metadata, and debug creation locations.
- **You can't link anything into the app.** Stick with [AX snapshot](/inspector/accessibility). It only sees what the iOS accessibility stack exposes, but it works for every app.

## Editing properties

When the in-app inspector is reachable, the browser client can:

- List actions a view supports (`tap`, `setText`, `scrollBy`, …) and trigger them.
- Read editable runtime properties (`alpha`, `backgroundColor`, `clipsToBounds`, `text`, …).
- Set those properties live, with structured value coercion for `UIColor`, `CGRect`, `CGPoint`, `CGSize`, and `UIEdgeInsets`.

These calls go through the HTTP proxy at `POST /api/simulators/{udid}/inspector/request`, which only accepts a small allow-list of methods. Direct TCP and WebSocket clients can use the full method list documented in the [Inspector Protocol](/api/inspector-protocol).

## WebKit targets

WebKit inspection is separate from the accessibility tree. The server discovers
inspectable targets through the simulator's `webinspectord` socket:

```http
GET /api/simulators/{udid}/webkit/targets
```

Each target includes an `inspectorUrl` that opens WebInspectorUI through SimDeck.
The browser client also exposes the same flow through the unified DevTools
toolbar toggle, which opens a resizeable right-side panel and attaches to the
first discovered target by default.
This only finds WebKit content that WebKit exposes to the Remote Inspector. On
iOS 16.4 and newer, app-owned `WKWebView` instances must set
`isInspectable = true`; otherwise AX may show the web area, but WebInspectorUI
cannot attach to it.

## Chrome DevTools targets

Chrome DevTools inspection is also separate from the accessibility tree. The
server exposes Metro React Native DevTools pages, local Chrome Inspector targets,
and connected React Native or NativeScript runtimes as Chrome DevTools targets:

```http
GET /api/simulators/{udid}/devtools/targets
```

Metro targets are discovered from common Metro ports plus matching local
Node/React Native listener ports, then matched to the selected simulator by
device name. Generic Chrome Inspector targets are discovered from common
Inspector ports such as `9222-9230` plus matching local Chrome/Node listener
ports. SimDeck proxies target WebSockets so the embedded frontend connects back
to the SimDeck server instead of opening cross-origin browser sockets. Each
target includes a `devtoolsFrontendUrl` that opens the bundled Chrome DevTools UI
through SimDeck. The browser client exposes these targets in the same DevTools
panel as WebKit inspection.
