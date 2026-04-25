# NativeScript Runtime Inspector

`@nativescript/xcode-canvas-inspector` is the runtime that connects a NativeScript app's view hierarchy to the SimDeck server without linking the Swift inspector framework. It implements the same [Inspector Protocol](/api/inspector-protocol) as the Swift agent but ships as a TypeScript package and runs entirely inside the NativeScript runtime.

The package source lives at `packages/nativescript-inspector/` in this repo.

## Install

```sh
npm install @nativescript/xcode-canvas-inspector
```

The package is `darwin`-friendly because the underlying NativeScript runtime is iOS-only; it works fine in NativeScript-iOS builds.

## Start the inspector

Call `startXcodeCanvasInspector(...)` as early as possible in app startup, ideally before any view is bootstrapped.

### NativeScript Core

```ts
import { startXcodeCanvasInspector } from "@nativescript/xcode-canvas-inspector";

if (__DEV__) {
  startXcodeCanvasInspector({ port: 4310 });
}
```

::: tip Port semantics
`port` here is the **SimDeck server port**, not a per-app inspector port. NativeScript apps do not need to choose a unique local inspector port — they connect outbound to the server's WebSocket hub.
:::

### NativeScript + Angular

For Angular NativeScript apps, call `startXcodeCanvasInspector()` **before** `runNativeScriptAngularApp()`:

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

The package installs a small compatibility shim for Angular 20 dev-mode template source locations on NativeScript views.

## What it exposes

`View.getHierarchy` returns the NativeScript logical tree by default — the actual `View` nodes you wrote, not just their UIKit backings. Pass `"source": "uikit"` to force the raw UIKit hierarchy.

Each NativeScript node carries:

- `type` — the view's class name (`Label`, `StackLayout`, `GridLayout`, …).
- `title` — derived text content or accessibility label when available.
- `bounds` and `frameInScreen` — UIKit screen points.
- `nativeScript` — NativeScript-specific metadata (CSS class names, IDs, bindings).
- `sourceLocation` — file/line/column when source maps are available.

When the in-app NativeScript inspector is the source, the SimDeck server returns `"source": "nativescript"` on the accessibility tree response and surfaces the matching `bundleIdentifier`, `processIdentifier`, and `displayScale` in the `inspector` block.

## Connection model

The runtime opens a WebSocket from the simulator app to the SimDeck server:

```text
ws://127.0.0.1:4310/api/inspector/connect
```

After the WebSocket is up, the server sends `Inspector.getInfo` and the runtime responds with the protocol version, `processIdentifier`, bundle metadata, and the available hierarchy sources. The server registers the runtime under that PID and prefers it over any TCP-based inspector for the same process.

If the WebSocket transport is not available, the runtime falls back to long-polling:

- `GET /api/inspector/poll?processIdentifier=<pid>` — waits for the next request from the server.
- `POST /api/inspector/response` — sends the response back.

Both transports speak the same envelope, so the same JS code handles them.

## Source locations from Angular templates

The NativeScript runtime can attach `sourceLocation` metadata to individual nodes when Angular dev-mode template source locations are enabled. Wire up your NativeScript webpack config:

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

With that in place, hierarchy nodes carry entries like:

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

The SimDeck browser client renders the file path inline so you can jump straight to source.

## Stopping the inspector

```ts
import { stopXcodeCanvasInspector } from "@nativescript/xcode-canvas-inspector";

stopXcodeCanvasInspector();
```

This closes the WebSocket and stops responding to inspector requests. Subsequent calls to `startXcodeCanvasInspector(...)` will spin a fresh runtime.

## Coexistence with the Swift agent

If both the Swift in-app inspector agent and the NativeScript runtime are present in the same app, the SimDeck server prefers the NativeScript runtime because it can publish the framework-level hierarchy. Direct TCP clients of the Swift agent are unaffected.
