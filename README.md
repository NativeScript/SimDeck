# SimDeck

SimDeck is a developer tool built for streamlining mobile app development for coding agents.
Drive iOS Simulator apps from the CLI, browser, and automated tests on macOS.

```sh
npm i -g simdeck@latest
```

## Features

- WebTransport streaming server in Rust, plus experimental WebRTC for runner previews, using hardware encoded HEVC/H.264 video
- Simulator control & inspection using private accessibility APIs
- CoreSimulator chrome asset rendering for device bezels
- NativeScript and React Native runtime inspector plugins, plus a native UIKit inspector framework for other apps
- Project daemon reuse: normal CLI commands automatically start and reuse one warm native host per project.
- `simdeck/test` for fast JS/TS app tests that can query accessibility state and drive simulator controls.
- Agent [`SKILL.md`](./skills/simdeck/SKILL.md) reference

## Build

```sh
./scripts/build-client.sh
./scripts/build-cli.sh
```

Requirements:

- macOS
- Xcode or Command Line Tools
- Node.js 18+
- Rust toolchain (`cargo`) when building from source

The npm package includes the native SimDeck binary and does not compile during
install. Rust is only required when building from a source checkout.

Install the current local checkout globally from source:

```sh
npm run build
npm install -g .
```

After a global install, use the `simdeck` command directly. From a local checkout, you can also run `./build/simdeck`.

## Documentation

Full documentation lives at [simdeck.nativescript.org](https://simdeck.nativescript.org/), with guides, the CLI reference, the REST API, the WebTransport video pipeline, and the inspector protocols. The source for the site lives in [`docs/`](docs/) — preview it locally with `npm run docs:dev`.

## Run

```sh
simdeck ui --open
```

This starts or reuses the project daemon, enables the browser UI, and opens the authenticated local URL.
To focus a specific simulator, add `?device=UDID` to the opened URL.
SimDeck Cloud uses the same server binary as its GitHub Actions provider. The
provider workflow starts `simdeck serve` on the runner, exposes it through a
tunnel, and lets the hosted control plane connect to the simulator with a
one-time access token.

The daemon exposes HTTP on the requested port and WebTransport on `port + 1`.
The browser bootstrap comes from `GET /api/health`, which returns the WebTransport URL template,
certificate hash, and packet version needed by the client.
The served browser UI receives the generated API access token automatically.

CLI commands automatically use the same warm daemon:

```sh
simdeck list
simdeck tap <udid> 0.5 0.5 --normalized
simdeck describe <udid> --format agent --max-depth 2
```

## Daemon

Manage the project daemon explicitly when needed:

```sh
simdeck daemon start
simdeck daemon status
simdeck daemon stop
```

Use software H.264 when macOS screen recording starves the hardware encoder:

```sh
simdeck daemon start --video-codec h264-software
```

For LAN browser access:

```sh
simdeck ui --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
```

Restart the CoreSimulator service layer when `simctl` reports a stale service
version or the live display gets stuck before the first frame:

```sh
simdeck core-simulator restart
```

You can also start or stop the CoreSimulator service layer explicitly:

```sh
simdeck core-simulator start
simdeck core-simulator shutdown
```

## CLI

```sh
simdeck list
simdeck boot <udid>
simdeck shutdown <udid>
simdeck erase <udid>
simdeck install <udid> /path/to/App.app
simdeck uninstall <udid> com.example.App
simdeck open-url <udid> https://example.com
simdeck launch <udid> com.apple.Preferences
simdeck toggle-appearance <udid>
simdeck pasteboard set <udid> "hello"
simdeck pasteboard get <udid>
simdeck screenshot <udid> --output screen.png
simdeck describe <udid>
simdeck describe <udid> --format agent --max-depth 4
simdeck describe <udid> --point 120,240
simdeck tap <udid> 120 240
simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
simdeck swipe <udid> 200 700 200 200
simdeck gesture <udid> scroll-down
simdeck pinch <udid> --start-distance 160 --end-distance 80
simdeck rotate-gesture <udid> --radius 100 --degrees 90
simdeck touch <udid> 0.5 0.5 --phase began --normalized
simdeck touch <udid> 120 240 --down --up --delay-ms 800
simdeck key <udid> enter
simdeck key-sequence <udid> --keycodes h,e,l,l,o
simdeck key-combo <udid> --modifiers cmd --key a
simdeck type <udid> "hello"
simdeck type <udid> --file message.txt
simdeck button <udid> lock --duration-ms 1000
simdeck batch <udid> --step "tap --label Continue" --step "type 'hello'"
simdeck dismiss-keyboard <udid>
simdeck home <udid>
simdeck app-switcher <udid>
simdeck rotate-left <udid>
simdeck rotate-right <udid>
simdeck chrome-profile <udid>
simdeck logs <udid> --seconds 30 --limit 200
```

`describe` uses the project daemon to prefer React Native, NativeScript, or
UIKit in-app inspectors, then falls back to the built-in private CoreSimulator
accessibility bridge. Use `--format agent` or `--format compact-json` for
lower-token hierarchy dumps. Coordinate commands accept screen coordinates from
the accessibility tree by default; pass `--normalized` to send `0.0..1.0`
coordinates directly.

## JS/TS Tests

```ts
import { connect } from "simdeck/test";

const sim = await connect();
try {
  await sim.tap("<udid>", 0.5, 0.5);
  await sim.waitFor("<udid>", { label: "Continue" });
  await sim.screenshot("<udid>");
} finally {
  sim.close();
}
```

`connect()` starts the project daemon when needed, reuses it when it is already
healthy, and only stops daemons it started itself.

## NativeScript Inspector

NativeScript apps can connect directly to the running server from JS and expose
their NativeScript logical hierarchy plus raw UIKit backing views without
linking the Swift inspector framework:

```ts
import { startSimDeckInspector } from "@nativescript/simdeck-inspector";

if (__DEV__) {
  startSimDeckInspector({ port: 4310 });
}
```

The runtime connects to `GET /api/inspector/connect` as a WebSocket. The Rust
server prefers connected NativeScript inspectors for hierarchy requests and
falls back to the Swift TCP inspector or the built-in native accessibility
bridge when no matching app inspector is available.

## React Native Inspector

React Native apps can expose their component tree and Metro dev-mode source
locations with the React Native inspector package:

```ts
import { AppRegistry } from "react-native";
import { startSimDeckReactNativeInspector } from "@simdeck/react-native-inspector";
import App from "./App";

if (__DEV__) {
  startSimDeckReactNativeInspector({ port: 4310 });
}

AppRegistry.registerComponent("Example", () => App);
```

Call it before `AppRegistry.registerComponent(...)` so the package can capture
React Fiber commits.

## VS Code

Package the local VS Code extension from this checkout:

```sh
npm run package:vscode-extension
```

This writes `build/vscode/simdeck-vscode.vsix`.

Install that local package into VS Code:

```sh
npm run install:vscode-extension
```

The install script packages the extension first if the `.vsix` does not exist,
then runs the VS Code CLI with `--install-extension build/vscode/simdeck-vscode.vsix --force`.
If the `code` command is not available, install it from VS Code with
`Shell Command: Install 'code' command in PATH`.

Then run `SimDeck: Open Simulator View` from the Command Palette. The extension will open the simulator
inside a VS Code panel and auto-start the local server when it is not already reachable.

## License

Copyright 2026 Dj

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
