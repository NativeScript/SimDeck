# NativeScript Xcode Canvas Inspector

Debug-only NativeScript runtime agent for `xcode-canvas-web`.

```sh
npm install @nativescript/xcode-canvas-inspector
```

```ts
import { startXcodeCanvasInspector } from "@nativescript/xcode-canvas-inspector";

if (__DEV__) {
  startXcodeCanvasInspector({ port: 4310 });
}
```

The agent connects from the simulator app to:

```text
ws://127.0.0.1:4310/api/inspector/connect
```

It implements the same inspector methods used by the Swift debug framework:

- `Inspector.getInfo`
- `View.getHierarchy`
- `View.get`
- `View.listActions`
- `View.perform`
- `View.getProperties`
- `View.setProperty`

`View.getHierarchy` returns the NativeScript logical tree by default and falls
back to raw UIKit when called with `{ "source": "uikit" }`.

For Angular NativeScript apps, call `startXcodeCanvasInspector()` before
`runNativeScriptAngularApp()`. The package installs a small compatibility shim
for Angular 20 dev-mode template source locations on NativeScript views.
