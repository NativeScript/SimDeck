# Xcode Canvas Web

`xcode-canvas-web` is a local simulator control plane with a Rust server, native Objective-C simulator bridge, and a React client.

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
npm install -g xcode-canvas-web
```

The npm package builds the native Rust/Objective-C CLI during `postinstall`; it
is not a prebuilt cross-platform binary.

Install the current local checkout globally from source:

```sh
npm install -g .
```

After a global install, use the `xcode-canvas-web` command directly. From a local checkout, you can also run `./build/xcode-canvas-web`.

## Documentation

Full documentation lives at [djdeveloperr.github.io/xcode-canvas-web](https://djdeveloperr.github.io/xcode-canvas-web/), with guides, the CLI reference, the REST API, the WebTransport video pipeline, and the inspector protocols. The source for the site lives in [`docs/`](docs/) — preview it locally with `npm run docs:dev`.

## Run

```sh
xcode-canvas-web serve --port 4310
```

Then open [http://127.0.0.1:4310](http://127.0.0.1:4310).
To focus a specific simulator, open
[http://127.0.0.1:4310?device=UDID](http://127.0.0.1:4310?device=UDID).

The Rust server exposes HTTP on the requested port and WebTransport on `port + 1`.
The browser bootstrap comes from `GET /api/health`, which returns the WebTransport URL template,
certificate hash, and packet version needed by the client.

## Service

Enable the per-user background service with `launchd`:

```sh
xcode-canvas-web service on --port 4310
```

Restart it:

```sh
xcode-canvas-web service restart
```

Disable it:

```sh
xcode-canvas-web service off
```

Restart the CoreSimulator service layer when `simctl` reports a stale service
version or the live display gets stuck before the first frame:

```sh
xcode-canvas-web core-simulator restart
```

You can also start or stop the CoreSimulator service layer explicitly:

```sh
xcode-canvas-web core-simulator start
xcode-canvas-web core-simulator shutdown
```

## CLI

```sh
xcode-canvas-web list
xcode-canvas-web boot <udid>
xcode-canvas-web shutdown <udid>
xcode-canvas-web erase <udid>
xcode-canvas-web install <udid> /path/to/App.app
xcode-canvas-web uninstall <udid> com.example.App
xcode-canvas-web open-url <udid> https://example.com
xcode-canvas-web launch <udid> com.apple.Preferences
xcode-canvas-web toggle-appearance <udid>
xcode-canvas-web pasteboard set <udid> "hello"
xcode-canvas-web pasteboard get <udid>
xcode-canvas-web screenshot <udid> --output screen.png
xcode-canvas-web describe-ui <udid>
xcode-canvas-web describe-ui <udid> --point 120,240
xcode-canvas-web tap <udid> 120 240
xcode-canvas-web tap <udid> --label "Continue" --wait-timeout-ms 5000
xcode-canvas-web swipe <udid> 200 700 200 200
xcode-canvas-web gesture <udid> scroll-down
xcode-canvas-web pinch <udid> --start-distance 160 --end-distance 80
xcode-canvas-web rotate-gesture <udid> --radius 100 --degrees 90
xcode-canvas-web touch <udid> 0.5 0.5 --phase began --normalized
xcode-canvas-web touch <udid> 120 240 --down --up --delay-ms 800
xcode-canvas-web key <udid> enter
xcode-canvas-web key-sequence <udid> --keycodes h,e,l,l,o
xcode-canvas-web key-combo <udid> --modifiers cmd --key a
xcode-canvas-web type <udid> "hello"
xcode-canvas-web type <udid> --file message.txt
xcode-canvas-web button <udid> lock --duration-ms 1000
xcode-canvas-web batch <udid> --step "tap --label Continue" --step "type 'hello'"
xcode-canvas-web dismiss-keyboard <udid>
xcode-canvas-web home <udid>
xcode-canvas-web app-switcher <udid>
xcode-canvas-web rotate-left <udid>
xcode-canvas-web rotate-right <udid>
xcode-canvas-web chrome-profile <udid>
xcode-canvas-web logs <udid> --seconds 30 --limit 200
```

`describe-ui` uses the built-in private CoreSimulator accessibility bridge and
does not shell out to AXe. Coordinate commands accept screen coordinates from
the accessibility tree by default; pass `--normalized` to send `0.0..1.0`
coordinates directly. The CLI intentionally does not implement screenshot-based
video streaming, MJPEG output, or screen recording; the live visual path remains
the web UI's WebTransport stream.

## NativeScript Inspector

NativeScript apps can connect directly to the running server from JS and expose
their NativeScript logical hierarchy plus raw UIKit backing views without
linking the Swift inspector framework:

```ts
import { startXcodeCanvasInspector } from "@nativescript/xcode-canvas-inspector";

if (__DEV__) {
  startXcodeCanvasInspector({ port: 4310 });
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

This writes `build/vscode/xcode-canvas-web-vscode.vsix`.

Install that local package into VS Code:

```sh
npm run install:vscode-extension
```

The install script packages the extension first if the `.vsix` does not exist,
then runs the VS Code CLI with `--install-extension build/vscode/xcode-canvas-web-vscode.vsix --force`.
If the `code` command is not available, install it from VS Code with
`Shell Command: Install 'code' command in PATH`.

Then run `Xcode Canvas Web: Open Simulator View` from the Command Palette. The extension will open the simulator
inside a VS Code panel and auto-start the local server when it is not already reachable.

## License

Copyright 2026 Dj

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
