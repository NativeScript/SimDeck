# SimDeck Inspector Agent

`SimDeckInspectorAgent` is a debug-only iOS framework that an app can link to expose its UIKit view hierarchy over a small network protocol.

It is intended to complement the generic accessibility inspector. Accessibility works for any simulator app; this agent works best for apps you control and can link in DEBUG builds.

## Install

Add this folder as a local Swift Package dependency:

```text
packages/inspector-agent
```

Then link the `SimDeckInspectorAgent` product into your app target for Debug only.

## Start The Agent

Call the initializer early in app startup, guarded by `#if DEBUG`.

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

UIKit apps can do the same from `application(_:didFinishLaunchingWithOptions:)`.

```swift
#if DEBUG
try? SimDeckInspectorAgent.shared.start()
#endif
```

The default server starts at TCP `127.0.0.1:47370`. If that port is already used by another simulator app, it automatically tries the next 32 ports and listens on the first free one. It also advertises Bonjour service type `_simdeckinspector._tcp`.

## Query It

The protocol is newline-delimited JSON over TCP.

```sh
printf '{"id":1,"method":"Inspector.getInfo"}\n' | nc 127.0.0.1 47370
printf '{"id":2,"method":"View.getHierarchy","params":{"maxDepth":4}}\n' | nc 127.0.0.1 47370
```

When multiple apps with the inspector are running, probe `47370-47402` and use
`Inspector.getInfo.processIdentifier` to link the response to the selected
simulator process.

See `PROTOCOL.md` for the full method list.

## SwiftUI

For SwiftUI apps you control, attach the root publisher to the top of your scene:

```swift
WindowGroup {
    ContentView()
        .simDeckPublishSwiftUIViewTree("ContentView", id: "app.root")
}
```

The agent reflects the current SwiftUI value/body tree and publishes it as the `swiftui` hierarchy source. `View.getHierarchy` returns that tree by default; pass `"source": "uikit"` to inspect the backing hosting views instead.

This is a debug aid built on Swift reflection. It can show the declared view/body structure, including custom subviews, containers, labels, modifier names, active conditional branches, and `ForEach` rows whose data and content builder are available through SwiftUI's public API. Private/custom containers may still be opaque when they do not expose a child view value or content builder.

The agent also reports SwiftUI hosting/bridge UIViews in the UIKit tree. To make specific SwiftUI elements addressable in that raw UIKit hierarchy, tag them in source:

```swift
Text("Continue")
    .simDeckInspectorTag("continue-label", id: "onboarding.continue.label")
```

The tag is represented by a lightweight, non-interactive probe view in the UIKit hierarchy.

## App Framework Hierarchies

Frameworks with their own logical tree can publish that tree into the agent. When a published snapshot exists, `View.getHierarchy` returns it by default; pass `"source": "uikit"` to force the raw UIKit tree.

```swift
try? SimDeckInspectorAgent.shared.publishHierarchySnapshot(
    source: "nativescript",
    snapshotJSON: #"{"source":"nativescript","roots":[]}"#
)
```

This is how the NativeScript integration exposes NativeScript `View` nodes instead of only the backing UIKit views.

Framework snapshots can attach source locations to individual nodes:

```json
{
  "type": "Label",
  "sourceLocation": {
    "file": "src/app/home.component.html",
    "line": 12,
    "column": 5,
    "offset": 238
  }
}
```

The web inspector shows this in the selected node properties.

## NativeScript

For NativeScript apps, use the companion runtime package instead of linking this
Swift package directly into your app code:

```sh
npm install @nativescript/simdeck-inspector
```

Start it as early as possible in app startup, before bootstrapping the app:

```ts
import { startSimDeckInspector } from "@nativescript/simdeck-inspector";

if (__DEV__) {
  startSimDeckInspector({ port: 4310 });
}
```

Important:

- `port` here is the `simdeck` server port, not a per-app inspector port.
- NativeScript apps do not need to choose or manage a unique local inspector port.
- The package connects to `/api/inspector/connect` and falls back to `/api/inspector/poll`.
- The separate auto-discovered local TCP ports described above are for the Swift in-app inspector transport, not the NativeScript package.

When the NativeScript inspector is running, `View.getHierarchy` returns the
NativeScript logical tree by default and `"source": "uikit"` still forces the
raw UIKit hierarchy.

### Angular

For Angular NativeScript apps, call `startSimDeckInspector()` before
`runNativeScriptAngularApp()`.

```ts
import { startSimDeckInspector } from "@nativescript/simdeck-inspector";

startSimDeckInspector({ port: 4310 });

runNativeScriptAngularApp({
  appModuleBootstrap: () =>
    bootstrapApplication(AppComponent, {
      providers: [
        provideNativeScriptHttpClient(withInterceptorsFromDi()),
        provideNativeScriptRouter(routes),
      ],
    }),
});
```

If you want template file and line locations in the hierarchy, enable Angular
template source locations in your NativeScript webpack config:

```js
const webpack = require("@nativescript/webpack");

class AngularTemplateSourceLocationsPlugin {
  apply(compiler) {
    const enable = async () => {
      const angularCompiler = await import("@angular/compiler");
      angularCompiler.setEnableTemplateSourceLocations?.(true);
    };

    compiler.hooks.beforeRun.tapPromise(
      "AngularTemplateSourceLocationsPlugin",
      enable,
    );
    compiler.hooks.watchRun.tapPromise(
      "AngularTemplateSourceLocationsPlugin",
      enable,
    );
  }
}

module.exports = (env) => {
  webpack.init(env);
  webpack.chainWebpack((config) => {
    config
      .plugin("angular-template-source-locations")
      .use(AngularTemplateSourceLocationsPlugin);
  });

  return webpack.resolveConfig();
};
```

That lets the NativeScript inspector attach `sourceLocation` metadata such as
`src/app/home.component.html:12:5` to hierarchy nodes.

## Auth Token

For shared-network simulator sessions, start with a token and require every request to include top-level `token`.

```swift
try? SimDeckInspectorAgent.shared.start(
    configuration: .init(
        port: 47370,
        bindToLocalhostOnly: false,
        authToken: "debug-secret"
    )
)
```
