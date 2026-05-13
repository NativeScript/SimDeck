# Swift Inspector

`SimDeckInspectorAgent` is a debug-only Swift package for UIKit and SwiftUI apps. It exposes richer hierarchy data and debug actions than accessibility alone.

## Add The Package

Add this local Swift package to your app:

```text
packages/inspector-agent
```

Link the `SimDeckInspectorAgent` product only in debug builds.

## Start It

SwiftUI:

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

UIKit:

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

## SwiftUI Trees

Publish a SwiftUI root when you want the declared SwiftUI tree instead of only the backing UIKit views:

```swift
WindowGroup {
    ContentView()
        .simDeckPublishSwiftUIViewTree("ContentView", id: "app.root")
}
```

Tag important SwiftUI views so they are easy to find:

```swift
Text("Continue")
    .simDeckInspectorTag("continue-label", id: "onboarding.continue.label")
```

Then inspect:

```sh
simdeck describe <udid> --source swiftui --format agent
simdeck describe <udid> --source uikit --format agent
```

## Debug Actions

When the selected view supports it, the browser can:

- Read runtime properties.
- Set simple UIKit properties for debugging.
- Perform actions such as tap, focus, set text, or scroll.

These edits are temporary and meant for debugging, not persistent app state.

## Direct Protocol Check

The Swift agent listens on `127.0.0.1:47370` and tries nearby ports if needed.

```sh
printf '{"id":1,"method":"Inspector.getInfo"}\n' | nc 127.0.0.1 47370
```

For protocol details, see [Inspector Protocol](/api/inspector-protocol).

## Shared Hosts

For shared or remote hosts, bind deliberately and set an auth token:

```swift
try? SimDeckInspectorAgent.shared.start(
    configuration: .init(
        port: 47370,
        bindToLocalhostOnly: false,
        authToken: "debug-secret"
    )
)
```

Keep the default localhost binding for normal local development.

## SwiftUI Preview Runner

This repo also includes an experimental preview runner for local development:

```sh
npm run preview:swiftui -- \
  --udid <booted-simulator-udid> \
  --file Sources/MyFeature/MyView.swift \
  --preview "Default" \
  --watch
```

It is intended for simulator-debuggable app targets and may need extra flags for complex projects.
