# NativeScript Inspector

`@nativescript/simdeck-inspector` publishes a NativeScript app's logical view tree to SimDeck.

Use it when accessibility does not show enough framework context or when you want source locations.

## Install

```sh
npm install @nativescript/simdeck-inspector
```

## Start It

Call it before app bootstrap in debug builds:

```ts
import { startSimDeckInspector } from "@nativescript/simdeck-inspector";

if (__DEV__) {
  startSimDeckInspector({ port: 4310 });
}
```

`port` is the SimDeck server port.

For Angular NativeScript apps, start the inspector before `runNativeScriptAngularApp(...)`:

```ts
import { startSimDeckInspector } from "@nativescript/simdeck-inspector";

startSimDeckInspector({ port: 4310 });

runNativeScriptAngularApp({
  appModuleBootstrap: () => bootstrapApplication(AppComponent),
});
```

## What You Get

NativeScript hierarchy nodes can include:

- NativeScript view type.
- Text, labels, IDs, CSS classes, and bindings.
- Screen-point frames.
- Underlying UIKit view information.
- Source locations when available.

Inspect it:

```sh
simdeck describe <udid> --source nativescript --format agent
```

Force UIKit instead:

```sh
simdeck describe <udid> --source uikit --format agent
```

## Angular Source Locations

Enable Angular template source locations in your NativeScript webpack config when you want nodes to point back to templates:

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

## Stop It

```ts
import { stopSimDeckInspector } from "@nativescript/simdeck-inspector";

stopSimDeckInspector();
```

## Troubleshooting

- Start the inspector before bootstrap.
- Confirm the app can reach `http://127.0.0.1:4310/api/health`.
- Bring the app to the foreground before calling `describe`.
- Force `--source nativescript` to read the fallback reason.
