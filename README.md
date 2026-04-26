# SimDeck

`simdeck` is a local simulator control plane with a Rust server, native Objective-C simulator bridge, and a React client.

- Rust product server in `server/`
- native Objective-C simulator/private-framework bridge in `cli/`
- `simctl`-backed simulator discovery and lifecycle commands
- private CoreSimulator boot fallback
- vendored private display bridge for continuous frames plus touch and keyboard injection
- CoreSimulator chrome asset rendering for device bezels
- NativeScript runtime inspector in `packages/nativescript-inspector/` for JS-driven UIKit querying and property edits
- local HTTP API plus static client hosting in Rust
- WebTransport video delivery over a self-signed local or LAN endpoint
- React client in `client/`

## Build

```sh
./scripts/build-client.sh
./scripts/build-cli.sh
```

## Install

Requirements:

- macOS
- Xcode or Command Line Tools
- Rust toolchain (`cargo`)
- Node.js 18+

Install the published CLI globally:

```sh
npm install -g simdeck
```

The npm package builds the native Rust/Objective-C CLI during `postinstall`; it
is not a prebuilt cross-platform binary.

Install the current local checkout globally from source:

```sh
npm install -g .
```

After a global install, use the `simdeck` command directly. From a local checkout, you can also run `./build/simdeck`.

## Documentation

Full documentation lives at [djdeveloperr.github.io/SimDeck](https://djdeveloperr.github.io/SimDeck/), with guides, the CLI reference, the REST API, the WebTransport video pipeline, and the inspector protocols. The source for the site lives in [`docs/`](docs/) — preview it locally with `npm run docs:dev`.

## Run

```sh
simdeck serve --port 4310
```

Then open [http://127.0.0.1:4310](http://127.0.0.1:4310).
To focus a specific simulator, open
[http://127.0.0.1:4310?device=UDID](http://127.0.0.1:4310?device=UDID).

The Rust server exposes HTTP on the requested port and WebTransport on `port + 1`.
The browser bootstrap comes from `GET /api/health`, which returns the WebTransport URL template,
certificate hash, and packet version needed by the client.
The served browser UI receives the generated API access token automatically; direct HTTP callers can use the startup token with `X-SimDeck-Token` or `Authorization: Bearer`.

## Service

Enable the per-user background service with `launchd`:

```sh
simdeck service on --port 4310
```

Restart it:

```sh
simdeck service restart
```

Disable it:

```sh
simdeck service off
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
simdeck describe-ui <udid>
simdeck describe-ui <udid> --format agent --max-depth 4
simdeck describe-ui <udid> --point 120,240
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

`describe-ui` can use the running local SimDeck service to prefer NativeScript or
UIKit in-app inspectors, then falls back to the built-in private CoreSimulator
accessibility bridge. Use `--format agent` or `--format compact-json` for
lower-token hierarchy dumps. Coordinate commands accept screen coordinates from
the accessibility tree by default; pass `--normalized` to send `0.0..1.0`
coordinates directly. The CLI intentionally does not implement screenshot-based
video streaming, MJPEG output, or screen recording; the live visual path remains
the web UI's WebTransport stream.

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
