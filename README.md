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

Install the published CLI globally:

```sh
npm install -g xcode-canvas-web
```

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

The Rust server exposes HTTP on the requested port and WebTransport on `port + 1`.
The browser bootstrap comes from `GET /api/health`, which returns the WebTransport URL template,
certificate hash, and packet version needed by the client.

## Service

Enable the per-user background service with `launchd`:

```sh
xcode-canvas-web service on --port 4310
```

Disable it:

```sh
xcode-canvas-web service off
```

## CLI

```sh
xcode-canvas-web list
xcode-canvas-web boot <udid>
xcode-canvas-web shutdown <udid>
xcode-canvas-web open-url <udid> https://example.com
xcode-canvas-web launch <udid> com.apple.Preferences
```

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
falls back to the Swift TCP inspector or AXe when no matching app inspector is
available.

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
