<p align="center">
  <img width="180" src="./client/public/simdeck.png">

  <h1 align="center">SimDeck</h1>

  <p align="center">
    SimDeck is a developer tool built for streamlining mobile app development for coding agents.
    Drive Simulator from the CLI using agents, browser, and automated tests on macOS.
  </p>
</p>

<hr/>

## Try it out

```sh
npx simdeck
```

Install the CLI globally for agentic-use:

```sh
npm i -g simdeck@latest
```

After installing the CLI, install the Codex skill so agents know the stable
SimDeck workflow:

```sh
npx skills add NativeScript/SimDeck --skill simdeck -g
```

For VS Code, install the `nativescript.simdeck-vscode` extension to open the simulator
view inside the editor.

## Features

- Local simulator video stream over WebTransport or WebRTC, hardware-encoded in full resolution
- Full simulator control & inspection using private accessibility APIs
- CoreSimulator chrome asset rendering for device bezels
- NativeScript, React Native, UIKit and SwiftUI runtime inspector plugins to view app's view hierarchy live
- `simdeck/test` for fast JS/TS app tests that can query accessibility state and drive simulator controls.
- SimDeck Studio for automatic PR deployments to on-demand simulators

## Documentation

Full documentation lives at [simdeck.nativescript.org](https://simdeck.nativescript.org/), with guides, the CLI reference, the REST API, the video pipeline, and the inspector protocols.

## Quick start

```sh
simdeck
```

This starts a workspace-local foreground daemon, prints local and LAN HTTP URLs plus a pairing code for LAN browsers, and stops when you press `q` or Ctrl-C.
To focus a specific simulator by name or UDID, pass it as the only argument:

```sh
simdeck "iPhone 17 Pro Max"
```

Use `simdeck ui --open` or `simdeck daemon start` when you want a reusable background daemon instead.
The no-subcommand lifecycle shortcuts are `simdeck -d` for detached start, `simdeck -k` to kill the background daemon, and `simdeck -r` to restart it.
The served loopback browser UI receives the generated API access token automatically. LAN browsers pair with the printed code before receiving the API cookie.

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

`simdeck daemon` manages the normal per-project warm process.

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
import { startSimDeckReactNativeInspector } from "react-native-simdeck";
import App from "./App";

if (__DEV__) {
  startSimDeckReactNativeInspector({ port: 4310 });
}

AppRegistry.registerComponent("Example", () => App);
```

Call it before `AppRegistry.registerComponent(...)` so the package can capture
React Fiber commits.

## VS Code

Install the `nativescript.simdeck` extension from the VS Code Marketplace, then
run `SimDeck: Open Simulator View` from the Command Palette. The extension
opens the simulator inside a VS Code panel and auto-starts the local daemon
when it is not already reachable.

## Contributing

Contributors should read [CONTRIBUTING.md](CONTRIBUTING.md) for local build
instructions, the dev workflow, and architecture notes.

## Copyright notice

Copyright [OpenJS Foundation](https://openjsf.org) and `NativeScript` contributors. All rights reserved. The [OpenJS Foundation](https://openjsf.org) has registered trademarks and uses trademarks. For a list of trademarks of the [OpenJS Foundation](https://openjsf.org), please see our [Trademark Policy](https://trademark-policy.openjsf.org/) and [Trademark List](https://trademark-list.openjsf.org/). Trademarks and logos not indicated on the [list of OpenJS Foundation trademarks](https://trademark-list.openjsf.org) are trademarks™ or registered® trademarks of their respective holders. Use of them does not imply any affiliation with or endorsement by them.

[The OpenJS Foundation](https://openjsf.org/) | [Terms of Use](https://terms-of-use.openjsf.org/) | [Privacy Policy](https://privacy-policy.openjsf.org/) | [OpenJS Foundation Bylaws](https://bylaws.openjsf.org/) | [Trademark Policy](https://trademark-policy.openjsf.org/) | [Trademark List](https://trademark-list.openjsf.org/) | [Cookie Policy](https://www.linuxfoundation.org/cookies/)

<h3 align="center">Made with ❤️</h3>
