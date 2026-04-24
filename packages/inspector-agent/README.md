# Xcode Canvas Inspector Agent

`XcodeCanvasInspectorAgent` is a debug-only iOS framework that an app can link to expose its UIKit view hierarchy over a small network protocol.

It is intended to complement the generic accessibility inspector. Accessibility works for any simulator app; this agent works best for apps you control and can link in DEBUG builds.

## Install

Add this folder as a local Swift Package dependency:

```text
packages/inspector-agent
```

Then link the `XcodeCanvasInspectorAgent` product into your app target for Debug only.

## Start The Agent

Call the initializer early in app startup, guarded by `#if DEBUG`.

```swift
#if DEBUG
import XcodeCanvasInspectorAgent
#endif

@main
struct DemoApp: App {
    init() {
        #if DEBUG
        try? XcodeCanvasInspectorAgent.shared.start()
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
try? XcodeCanvasInspectorAgent.shared.start()
#endif
```

The default server starts at TCP `127.0.0.1:47370`. If that port is already used by another simulator app, it automatically tries the next 32 ports and listens on the first free one. It also advertises Bonjour service type `_xcwinspector._tcp`.

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

The agent automatically reports SwiftUI hosting/bridge UIViews. SwiftUI's value tree is not publicly enumerable, so meaningful SwiftUI nodes should be tagged in source:

```swift
Text("Continue")
    .xcwInspectorTag("continue-label", id: "onboarding.continue.label")
```

The tag is represented by a lightweight, non-interactive probe view in the UIKit hierarchy.

## App Framework Hierarchies

Frameworks with their own logical tree can publish that tree into the agent. When a published snapshot exists, `View.getHierarchy` returns it by default; pass `"source": "uikit"` to force the raw UIKit tree.

```swift
try? XcodeCanvasInspectorAgent.shared.publishHierarchySnapshot(
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
npm install @nativescript/xcode-canvas-inspector
```

Start it as early as possible in app startup, before bootstrapping the app:

```ts
import { startXcodeCanvasInspector } from "@nativescript/xcode-canvas-inspector";

if (__DEV__) {
  startXcodeCanvasInspector({ port: 4310 });
}
```

Important:

- `port` here is the `xcode-canvas-web` server port, not a per-app inspector port.
- NativeScript apps do not need to choose or manage a unique local inspector port.
- The package connects to `/api/inspector/connect` and falls back to `/api/inspector/poll`.
- The separate auto-discovered local TCP ports described above are for the Swift in-app inspector transport, not the NativeScript package.

When the NativeScript inspector is running, `View.getHierarchy` returns the
NativeScript logical tree by default and `"source": "uikit"` still forces the
raw UIKit hierarchy.

### Angular

For Angular NativeScript apps, call `startXcodeCanvasInspector()` before
`runNativeScriptAngularApp()`.

```ts
import { startXcodeCanvasInspector } from "@nativescript/xcode-canvas-inspector";

startXcodeCanvasInspector({ port: 4310 });

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
try? XcodeCanvasInspectorAgent.shared.start(
    configuration: .init(
        port: 47370,
        bindToLocalhostOnly: false,
        authToken: "debug-secret"
    )
)
```
