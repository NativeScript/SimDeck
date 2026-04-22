# Xcode Canvas Web

`xcode-canvas-web` is a local simulator control plane with a Rust server, native Objective-C simulator bridge, and a React client.

- Rust product server in `server/`
- native Objective-C simulator/private-framework bridge in `cli/`
- `simctl`-backed simulator discovery and lifecycle commands
- private CoreSimulator boot fallback
- vendored private display bridge for continuous frames plus touch and keyboard injection
- CoreSimulator chrome asset rendering for device bezels
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

## License

Copyright 2026 Dj

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
