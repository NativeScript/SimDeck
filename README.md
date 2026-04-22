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

## Run

```sh
./build/xcode-canvas-web serve --port 4310
```

Then open [http://127.0.0.1:4310](http://127.0.0.1:4310).

The Rust server exposes HTTP on the requested port and WebTransport on `port + 1`.
The browser bootstrap comes from `GET /api/health`, which returns the WebTransport URL template,
certificate hash, and packet version needed by the client.

To expose the server on your LAN, bind on all interfaces and advertise a host
or IP that the browser can actually reach and validate in the self-signed
certificate:

```sh
./build/xcode-canvas-web serve --port 4310 --bind 0.0.0.0 --advertise-host 192.168.1.50
```

Then open `http://192.168.1.50:4310` from another device on the same network.

## CLI

```sh
./build/xcode-canvas-web list
./build/xcode-canvas-web boot <udid>
./build/xcode-canvas-web shutdown <udid>
./build/xcode-canvas-web open-url <udid> https://example.com
./build/xcode-canvas-web launch <udid> com.apple.Preferences
```

## HTTP API

- `GET /api/health`
- `GET /api/metrics`
- `GET /api/simulators`
- `POST /api/simulators/:udid/boot`
- `POST /api/simulators/:udid/shutdown`
- `POST /api/simulators/:udid/open-url`
- `POST /api/simulators/:udid/launch`
- `POST /api/simulators/:udid/touch`
- `POST /api/simulators/:udid/key`
- `POST /api/simulators/:udid/home`
- `POST /api/simulators/:udid/rotate-right`
- `GET /api/simulators/:udid/chrome-profile`
- `GET /api/simulators/:udid/chrome.png`

## Status

The live browser preview now comes from the vendored private display bridge rather than `simctl io screenshot`.
The Rust server owns REST routing, session orchestration, metrics, static file serving, and the WebTransport video path.
The native layer owns simulator lookup/boot, chrome rendering, HID input injection, and hardware H.264 encode.
