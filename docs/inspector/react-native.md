# React Native Runtime Inspector

`@simdeck/react-native-inspector` is a debug-only React Native iOS package that publishes the React component tree to SimDeck. It uses the same [Inspector Protocol](/api/inspector-protocol) as the Swift and NativeScript inspectors, but connects outbound to the SimDeck server over WebSocket.

The package source lives at `packages/react-native-inspector/` in this repo.

## Install

```sh
npm install @simdeck/react-native-inspector
cd ios && pod install
```

## Start the inspector

Call `startSimDeckReactNativeInspector(...)` before `AppRegistry.registerComponent(...)` so the package can install the React Fiber hook before the app's first render.

```ts
import { AppRegistry } from "react-native";
import { startSimDeckReactNativeInspector } from "@simdeck/react-native-inspector";
import App from "./App";

if (__DEV__) {
  startSimDeckReactNativeInspector({ port: 4310 });
}

AppRegistry.registerComponent("Example", () => App);
```

`port` is the SimDeck server port. React Native apps do not need to choose a local inspector port.

## What it exposes

`View.getHierarchy` returns the React component hierarchy by default. Each node may include:

- `type` — the component or host component name.
- `title` — derived from accessibility label, `testID`, `nativeID`, or text children.
- `frame` and `frameInScreen` — measured screen-point bounds when the node resolves to a native host tag.
- `reactNative` — React Native metadata such as host tag, `testID`, and `nativeID`.
- `sourceLocation` — Metro dev-mode file/line/column data from React's `_debugSource` metadata.

Source locations depend on development builds. Production bundles normally strip this metadata.

## Debug edits and JS execution

The package supports the standard `View.getProperties`, `View.setProperty`, and `View.evaluateScript` methods.

`View.setProperty` is best-effort: when the selected node resolves to a native host instance, it calls `setNativeProps(...)`. It cannot rewrite immutable React component props, and the app's next render can override debug edits.

`View.evaluateScript` runs with `fiber`, `props`, `instance`, and `ReactNative` in scope.
