# React Native Inspector

`react-native-simdeck` publishes a React Native component tree to SimDeck in development builds.

Use it when you want component names, host tags, `testID`/`nativeID`, source locations, and best-effort debug actions.

## Install

```sh
npm install react-native-simdeck
cd ios && pod install
```

## Start It

Import the auto entrypoint before the app registers.

Expo Router:

```ts
import "react-native-simdeck/auto";
import "expo-router/entry";
```

Manual entrypoint:

```ts
import "react-native-simdeck/auto";
import { AppRegistry } from "react-native";
import App from "./App";

AppRegistry.registerComponent("Example", () => App);
```

Explicit options:

```ts
import { startSimDeckReactNativeInspector } from "react-native-simdeck";

if (__DEV__) {
  startSimDeckReactNativeInspector({ port: 4310 });
}
```

`port` is the SimDeck server port.

## Inspect

```sh
simdeck describe <udid> --source react-native --format agent
```

Nodes may include:

- Component or host component name.
- Accessibility label, `testID`, `nativeID`, and text.
- Screen-point frames when the component resolves to a native host.
- Metro dev-mode source locations.

## Debug Edits

`View.setProperty` is best-effort. If the node resolves to a native host instance, the runtime calls `setNativeProps(...)`. The app's next React render can overwrite the edit.

`View.evaluateScript` runs with useful React Native objects in scope for diagnostics.

## Troubleshooting

- Import the auto entrypoint before app registration.
- Use a development build for source locations.
- Set `EXPO_PUBLIC_SIMDECK_PORT=4310` if auto port scanning cannot find the daemon.
- Bring the app to the foreground before inspecting.
