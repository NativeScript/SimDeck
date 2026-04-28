# React Native Inspector

`react-native-simdeck` is a debug-only runtime inspector for React Native iOS apps. It connects to the SimDeck server over the same WebSocket inspector hub as the NativeScript runtime.

## Install

```sh
npm install react-native-simdeck
cd ios && pod install
```

## Start

Install the inspector before `AppRegistry.registerComponent(...)` so it can capture React commits and retain component source locations.

```ts
import { AppRegistry } from "react-native";
import { startSimDeckReactNativeInspector } from "react-native-simdeck";
import App from "./App";

if (__DEV__) {
  startSimDeckReactNativeInspector({ port: 4310 });
}

AppRegistry.registerComponent("Example", () => App);
```

## What It Exposes

- React component hierarchy from the React Fiber tree.
- `sourceLocation` from React dev-mode `_debugSource` metadata when Metro includes it.
- Host component tags and measured screen frames when React Native can resolve them.
- Best-effort `View.getProperties`, `View.setProperty`, and `View.evaluateScript` for debug sessions.

`View.setProperty` can update native host props through `setNativeProps` when a selected node resolves to a native host instance. It cannot rewrite immutable React component props; re-rendering from app state still wins.
