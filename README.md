<p align="center">
  <img width="180" src="./client/public/simdeck.png">

  <h1 align="center">SimDeck</h1>

  <p align="center">
    SimDeck is a developer tool built for streamlining mobile app development using agents.
    Drive iOS Simulators and Android emulators from browser & CLI.
  </p>
</p>

<hr/>

![Codex Screenshot](./assets/codex-screenshot.png)

## Try it out

```sh
npx simdeck
```

Open the URL in your IDE of choice, for example in-app browser in Codex.

Install the CLI globally for agentic-use:

```sh
npm i -g simdeck@latest
```

After installing the CLI, install the Codex skill so agents know the stable
SimDeck workflow:

```sh
npx skills add NativeScript/SimDeck --skill simdeck -g
```

For VS Code, install the [`nativescript.simdeck-vscode`](https://marketplace.visualstudio.com/items?itemName=NativeScript.simdeck-vscode) extension to open the simulator
view inside the editor.

## Features

- Supports streaming both iOS simulators and Android emulators
- Full simulator control & inspection using private iOS accessibility APIs and Android UIAutomator - available using `simdeck` CLI
- Real-time screen `describe` command using accessibility view tree - available in token-efficient format for agents
- Profiling built-in: CPU, memory, disk writes, network throughput, hang signals, and stack sampling
- CoreSimulator chrome asset rendering for device bezels
- NativeScript, React Native, Flutter, UIKit and SwiftUI runtime inspector plugins to debug app's view hierarchy live
- `simdeck/test` for fast JS-based app tests that can query accessibility state and drive simulator controls

## Documentation

Full documentation lives at [simdeck.nativescript.org](https://simdeck.nativescript.org/), with guides, the CLI reference, the REST API, the video pipeline, and the inspector protocols.

For hosted pull request simulator sessions, use the GitHub Actions integration
documented in the [GitHub Actions guide](https://simdeck.nativescript.org/guide/github-actions).

## Quick start

```sh
simdeck
```

To focus a specific simulator by name or UDID, pass it as the only argument:

```sh
simdeck "iPhone 17 Pro Max"
```

`simdeck -d` for detached start, `simdeck -k` to kill the background daemon, and `simdeck -r` to restart it.

The served loopback browser UI receives the generated API access token automatically.
LAN clients should pair with the printed code before receiving the API cookie.

For pairing with SimDeck iOS app:

```sh
simdeck pair
```

This starts or refreshes the global LaunchAgent-backed SimDeck service, prints
local, LAN, and Tailscale URLs when available, and shows a QR code with a
`simdeck://pair` link. The QR contains the pairing code plus all detected
non-loopback addresses, so pairing once can save both the LAN and Tailscale
routes with the same service token.
Normal service restarts preserve that token so paired clients stay connected.
Use `simdeck service reset` only when you want to rotate the service token and
restart the LaunchAgent.
The LaunchAgent service uses port 4310. Project daemons start at port 4311 and
probe upward when that port is busy. When the service is active, `simdeck` and
`simdeck ui` print the existing service endpoints instead of starting a project
daemon; use the `daemon` subcommand when you explicitly want a workspace daemon.

CLI commands automatically use the same warm daemon:

```sh
simdeck list
simdeck use <udid>
simdeck tap 0.5 0.5 --normalized
simdeck tap "Continue"
simdeck describe --format agent --max-depth 2 --interactive
simdeck press @e3
simdeck snapshot --format agent --max-depth 2 -i
simdeck --device <other-udid> describe --format agent --max-depth 2
```

## CLI

```sh
simdeck list
simdeck use <udid>
simdeck boot <udid>
simdeck shutdown
simdeck erase
simdeck install /path/to/App.app
simdeck install /path/to/App.ipa
simdeck install android:<avd-name> /path/to/app.apk
simdeck uninstall com.example.App
simdeck open-url https://example.com
simdeck launch com.apple.Preferences
simdeck toggle-appearance
simdeck pasteboard set "hello"
simdeck pasteboard get
simdeck screenshot --output screen.png
simdeck screenshot --with-bezel --output screen-bezel.png
simdeck record --seconds 5 --output screen-recording.mp4
simdeck stream --frames 120 > stream.h264
simdeck describe
simdeck describe --format agent --max-depth 4
simdeck describe --format agent --max-depth 4 --interactive
simdeck snapshot --format agent --max-depth 4 -i
simdeck describe --point 120,240
simdeck wait-for --label "Welcome" --timeout-ms 5000
simdeck wait --label "Welcome" --timeout-ms 5000
simdeck assert --id login.button --source auto --max-depth 8
simdeck tap 120 240
simdeck tap --label "Continue" --wait-timeout-ms 5000
simdeck tap --id com.apple.settings.screenTime --expect-id BackButton
simdeck tap "Continue"
simdeck press @e3
simdeck back
simdeck swipe 200 700 200 200
simdeck gesture scroll-down
simdeck pinch --start-distance 160 --end-distance 80
simdeck rotate-gesture --radius 100 --degrees 90
simdeck touch 0.5 0.5 --phase began --normalized
simdeck touch 120 240 --down --up --delay-ms 800
simdeck key enter
simdeck key-sequence --keycodes h,e,l,l,o
simdeck key-combo --modifiers cmd --key a
simdeck type "hello"
simdeck type --file message.txt
simdeck button lock --duration-ms 1000
simdeck button volume-up
simdeck button action --duration-ms 1000
simdeck button digital-crown
simdeck crown --delta 50
simdeck button left-side-button
simdeck batch --step "tap --label Continue --expect-label Done" --step "type 'hello'" --step "back"
simdeck dismiss-keyboard
simdeck button software-keyboard
simdeck home
simdeck app-switcher
simdeck rotate-left
simdeck rotate-right
simdeck chrome-profile
simdeck logs --seconds 30 --limit 200
simdeck processes
simdeck stats --watch
simdeck sample --seconds 3
```

`simdeck list` defaults to compact JSON for agent-friendly device selection.
Use `simdeck list --format json` for the full inventory with paths and display
metadata.

`simdeck use <udid>` stores a default simulator for the current project
directory. Most device commands accept `[<udid>]`; when it is omitted, SimDeck
uses `--device`, `SIMDECK_DEVICE`, `SIMDECK_UDID`, the saved project default,
or the only booted simulator, in that order.

## JS/TS Tests

```ts
import { connect } from "simdeck/test";

const sim = await connect({ udid: "<udid>" });
try {
  await sim.tap(0.5, 0.5);
  await sim.waitFor({ label: "Continue" });
  await sim.screenshot();
  await sim.screenshot({ withBezel: true });
  await sim.record({ seconds: 5 });
} finally {
  sim.close();
}
```

`connect()` starts the project daemon when needed, reuses it when it is already
healthy, and only stops daemons it started itself. Pass `udid` to `connect()`
to make it the default for session methods; each method still accepts an
explicit UDID as the first argument when needed. Query helpers such as
`tree()`, `query()`, `waitFor()`, `assert()`, and selector `tapElement()`
default to `source: "native-ax"` for fast agent control; pass
`source: "auto"` when a test intentionally wants richer framework inspector
trees first.

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
import "react-native-simdeck/auto";
import "expo-router/entry";
```

Import it before `expo-router/entry` or `AppRegistry.registerComponent(...)`
so the package can capture React Fiber commits. The auto entrypoint no-ops
outside development, reads `EXPO_PUBLIC_SIMDECK_PORT` when present, and
otherwise scans common SimDeck daemon ports.

## Flutter Inspector

Flutter apps can expose their widget tree, render frames, semantics metadata,
and debug widget creation locations with the Flutter inspector package:

```dart
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:simdeck_flutter_inspector/simdeck_flutter_inspector.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();

  if (kDebugMode) {
    startSimDeckFlutterInspector(port: 4310);
  }

  runApp(const App());
}
```

## Contributing

Contributors should read [CONTRIBUTING.md](CONTRIBUTING.md) for local build
instructions, the dev workflow, and architecture notes.

## Copyright notice

Copyright [OpenJS Foundation](https://openjsf.org) and `NativeScript` contributors. All rights reserved. The [OpenJS Foundation](https://openjsf.org) has registered trademarks and uses trademarks. For a list of trademarks of the [OpenJS Foundation](https://openjsf.org), please see our [Trademark Policy](https://trademark-policy.openjsf.org/) and [Trademark List](https://trademark-list.openjsf.org/). Trademarks and logos not indicated on the [list of OpenJS Foundation trademarks](https://trademark-list.openjsf.org) are trademarks™ or registered® trademarks of their respective holders. Use of them does not imply any affiliation with or endorsement by them.

[The OpenJS Foundation](https://openjsf.org/) | [Terms of Use](https://terms-of-use.openjsf.org/) | [Privacy Policy](https://privacy-policy.openjsf.org/) | [OpenJS Foundation Bylaws](https://bylaws.openjsf.org/) | [Trademark Policy](https://trademark-policy.openjsf.org/) | [Trademark List](https://trademark-list.openjsf.org/) | [Cookie Policy](https://www.linuxfoundation.org/cookies/)

<h3 align="center">Made with ❤️</h3>
