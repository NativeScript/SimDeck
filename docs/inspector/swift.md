# Swift In-App Inspector Agent

`SimDeckInspectorAgent` is a debug-only iOS Swift package that exposes a UIKit hierarchy and live property edits through the [`SDI/0.1`](/api/inspector-protocol) protocol. Apps that link it for `Debug` builds can be inspected from the SimDeck browser client without going through the system accessibility stack.

The package source lives at `packages/inspector-agent/` in this repo.

## Install

Add the local Swift package to your app:

```text
packages/inspector-agent
```

Then link the `SimDeckInspectorAgent` product into your app target for `Debug` configurations only.

## Start the agent

Call the initializer early in app startup, behind a `#if DEBUG` guard.

### SwiftUI

```swift
#if DEBUG
import SimDeckInspectorAgent
#endif

@main
struct DemoApp: App {
    init() {
        #if DEBUG
        try? SimDeckInspectorAgent.shared.start()
        #endif
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}
```

### UIKit

```swift
func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
) -> Bool {
    #if DEBUG
    try? SimDeckInspectorAgent.shared.start()
    #endif
    return true
}
```

## How discovery works

The agent listens on TCP `127.0.0.1:47370` by default. If that port is already in use it tries the next 32 ports and listens on the first free one. It also advertises Bonjour service type `_simdeckinspector._tcp` so other tools can find it without probing.

The SimDeck server discovers the agent for a given simulator by:

1. Probing TCP `47370–47402` on `127.0.0.1`.
2. Calling `Inspector.getInfo` to read the agent's `processIdentifier`.
3. Running `ps -p <pid> -o command=` and matching the simulator's UDID against the process command line.

This lets multiple inspector-enabled apps run side by side without colliding.

## Talking to the agent directly

The protocol is newline-delimited JSON over TCP — convenient enough for `nc`:

```sh
printf '{"id":1,"method":"Inspector.getInfo"}\n' | nc 127.0.0.1 47370
printf '{"id":2,"method":"View.getHierarchy","params":{"maxDepth":4}}\n' | nc 127.0.0.1 47370
```

For the full envelope shape and method list, see the [Inspector Protocol](/api/inspector-protocol).

## SwiftUI view tree

For SwiftUI apps you control, attach the root publisher to the top of your scene. The agent reflects the current SwiftUI value/body tree and publishes it as the `swiftui` hierarchy source while keeping the raw UIKit host tree available as `uikit`.

```swift
WindowGroup {
    ContentView()
        .simDeckPublishSwiftUIViewTree("ContentView", id: "app.root")
}
```

`View.getHierarchy` returns the published SwiftUI tree by default. Pass `"source": "uikit"` to inspect the backing hosting views instead.

This is a debug aid built on Swift reflection. It can show the declared view/body structure, including custom subviews, containers, labels, modifier names, active conditional branches, and `ForEach` rows whose data and content builder are available through SwiftUI's public API. Private/custom containers may still be opaque when they do not expose a child view value or content builder.

## SwiftUI tagging

The agent also reports SwiftUI hosting and bridge `UIView`s in the UIKit tree. To make specific SwiftUI elements addressable in the raw UIKit hierarchy, tag them in source:

```swift
Text("Continue")
    .simDeckInspectorTag("continue-label", id: "onboarding.continue.label")
```

Tagged SwiftUI views appear as lightweight, non-interactive probe `UIView`s in the hierarchy with `swiftUI.isProbe = true`. The browser client surfaces them in the inspector pane.

## Publishing a framework hierarchy

Frameworks with their own logical tree can publish that tree into the agent. When a published snapshot exists, `View.getHierarchy` returns it by default; pass `"source": "uikit"` to force the raw UIKit tree.

```swift
try? SimDeckInspectorAgent.shared.publishHierarchySnapshot(
    source: "nativescript",
    snapshotJSON: #"{"source":"nativescript","roots":[]}"#
)
```

This is exactly how the [NativeScript runtime inspector](/inspector/nativescript) exposes its logical tree.

Framework snapshots can attach source locations to individual nodes:

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

The browser client uses these to jump from the hierarchy back to source.

## Auth token

For shared-network simulator sessions, start the agent with a token and require every request to include a top-level `token`:

```swift
try? SimDeckInspectorAgent.shared.start(
    configuration: .init(
        port: 47370,
        bindToLocalhostOnly: false,
        authToken: "debug-secret"
    )
)
```

When a token is set, requests without a matching `token` field are rejected with `error.code = -32401`.

## Recommended configuration

For most apps the defaults are correct:

- Bind to `127.0.0.1` only.
- Listen on `47370` (with auto-fallback if busy).
- No auth token.

For multi-user development setups (shared simulator hosts on a LAN, CI rigs), set `bindToLocalhostOnly: false` plus an auth token and forward the inspector port through your VPN or SSH tunnel of choice.

## Compatibility with the NativeScript inspector

The Swift agent and the NativeScript runtime inspector implement the same protocol on different transports. Anything that can talk `SDI/0.1` over TCP can also talk it over the SimDeck WebSocket hub, and vice versa.

The SimDeck server prefers connected NativeScript inspectors over Swift TCP agents when both are present for the same process. Direct TCP clients can pick whichever transport they prefer.
