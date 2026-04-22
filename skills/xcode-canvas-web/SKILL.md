# Xcode Canvas Web

Use this skill when you need to operate the local Xcode Canvas Web project: build the CLI, manage simulators from the command line, or launch the local server and browser client.

## What This Project Does

`xcode-canvas-web` is a local simulator control plane.

- The product server lives in `server/` and is written in Rust.
- The native simulator bridge lives in `cli/` and is written in Objective-C.
- The browser client lives in `client/` and is built with React.
- The Rust CLI serves the HTTP API and the built web app, and exposes WebTransport for video.

## Build Commands

Build the client bundle:

```sh
./scripts/build-client.sh
```

Build the native CLI:

```sh
./scripts/build-cli.sh
```

The compiled binary lands at:

```sh
./build/xcode-canvas-web
```

Install the CLI globally from this checkout:

```sh
npm install -g .
```

## Launch The Web Server

```sh
xcode-canvas-web serve --port 4310
```

Open:

```sh
http://127.0.0.1:4310
```

The server also exposes WebTransport on `4311` when the HTTP port is `4310`.
The client should discover the exact URL template and certificate hash from `GET /api/health`.

From a local checkout without a global install, the equivalent command is:

```sh
./build/xcode-canvas-web serve --port 4310
```

You can point the server at a different built client directory if needed:

```sh
xcode-canvas-web serve --port 4310 --client-root /absolute/path/to/client/dist
```

Enable the per-user `launchd` service:

```sh
xcode-canvas-web service on --port 4310
```

Disable it:

```sh
xcode-canvas-web service off
```

## Simulator CLI Commands

List simulators:

```sh
xcode-canvas-web list
```

Boot a simulator:

```sh
xcode-canvas-web boot <udid>
```

Shut a simulator down:

```sh
xcode-canvas-web shutdown <udid>
```

Open a URL inside a simulator:

```sh
xcode-canvas-web open-url <udid> https://example.com
```

Launch an installed app by bundle identifier:

```sh
xcode-canvas-web launch <udid> com.apple.Preferences
```

## Current API Shape

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

## Important Notes

- The live frame pane comes from the vendored private display bridge.
- The live video path is WebTransport-only after the Rust server cutover. Do not add `/stream.h264` back as a fallback.
- Device chrome comes from CoreSimulator device-type chrome PDFs, with the private bridge code kept nearby as an experimental reference.
- If you change CLI flags or API routes, update `README.md` and `AGENTS.md` in the same pass.
