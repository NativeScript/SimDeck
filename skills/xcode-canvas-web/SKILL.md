# Xcode Canvas Web

Use this skill when you need to operate the local Xcode Canvas Web project: build the CLI, manage simulators from the command line, or launch the local server and browser client.

## What This Project Does

`xcode-canvas-web` is a local simulator control plane.

- The product server lives in `server/` and is written in Rust.
- The native simulator bridge lives in `cli/` and is written in Objective-C.
- The browser client lives in `client/` and is built with React.
- The NativeScript in-app inspector runtime lives in `packages/nativescript-inspector/`
  and is written in TypeScript.
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

Use software H.264 when macOS screen recording starves the hardware encoder:

```sh
xcode-canvas-web serve --port 4310 --video-codec h264-software
```

Open:

```sh
http://127.0.0.1:4310
```

If the IDE has an in-app browser capability, open that URL so the user can see
the simulator while agents build and test. To focus a specific simulator in the
web UI, include the simulator UDID:

```sh
http://127.0.0.1:4310?device=<udid>
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

Restart the per-user `launchd` service:

```sh
xcode-canvas-web service restart
```

Disable it:

```sh
xcode-canvas-web service off
```

Restart the CoreSimulator service layer when `simctl` reports a stale service
version or the display stream gets stuck waiting for the first frame:

```sh
xcode-canvas-web core-simulator restart
```

Start or shut down the CoreSimulator service layer explicitly:

```sh
xcode-canvas-web core-simulator start
xcode-canvas-web core-simulator shutdown
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

Dismiss the software keyboard when it is visible:

```sh
xcode-canvas-web dismiss-keyboard <udid>
```

Inspect the native accessibility tree as JSON, without AXe:

```sh
xcode-canvas-web describe-ui <udid>
xcode-canvas-web describe-ui <udid> --point 120,240
```

Capture a PNG screenshot:

```sh
xcode-canvas-web screenshot <udid> --output screen.png
xcode-canvas-web screenshot <udid> --stdout > screen.png
```

Interact with the simulator from agent scripts. Coordinates are screen
coordinates from `describe-ui` unless `--normalized` is present:

```sh
xcode-canvas-web tap <udid> 120 240
xcode-canvas-web tap <udid> --id LoginButton --wait-timeout-ms 5000
xcode-canvas-web tap <udid> --label "Continue" --element-type Button
xcode-canvas-web swipe <udid> 200 700 200 200
xcode-canvas-web swipe <udid> 200 700 200 200 --duration-ms 500 --pre-delay-ms 100 --post-delay-ms 250
xcode-canvas-web gesture <udid> scroll-up
xcode-canvas-web gesture <udid> swipe-from-left-edge
xcode-canvas-web pinch <udid> --start-distance 160 --end-distance 80
xcode-canvas-web pinch <udid> --start-distance 0.20 --end-distance 0.35 --normalized --duration-ms 250 --steps 8
xcode-canvas-web rotate-gesture <udid> --radius 100 --degrees 90
xcode-canvas-web rotate-gesture <udid> --radius 0.12 --degrees 45 --normalized --duration-ms 250 --steps 8
xcode-canvas-web touch <udid> 0.5 0.5 --phase began --normalized
xcode-canvas-web touch <udid> 0.5 0.5 --phase ended --normalized
xcode-canvas-web touch <udid> 120 240 --down --up --delay-ms 800
xcode-canvas-web key <udid> enter
xcode-canvas-web key <udid> 42 --duration-ms 500
xcode-canvas-web key-sequence <udid> --keycodes h,e,l,l,o --delay-ms 75
xcode-canvas-web key-combo <udid> --modifiers cmd,shift --key z
xcode-canvas-web type <udid> "hello"
xcode-canvas-web type <udid> --stdin
xcode-canvas-web type <udid> --file message.txt
```

Use batch for multi-step agent flows. Batch accepts one source: repeated
`--step`, `--file`, or `--stdin`. Step lines support `tap`, `swipe`, `gesture`,
`pinch`, `rotate-gesture`, `touch`, `type`, `button`, `key`, `key-sequence`,
`key-combo`, and `sleep`.

```sh
xcode-canvas-web batch <udid> \
  --step "tap --label Continue --wait-timeout-ms 5000" \
  --step "type 'hello world'" \
  --step "gesture scroll-down" \
  --step "pinch --start-distance 0.20 --end-distance 0.35 --normalized"
```

Use hardware/system controls:

```sh
xcode-canvas-web button <udid> home
xcode-canvas-web button <udid> lock --duration-ms 1000
xcode-canvas-web button <udid> side-button
xcode-canvas-web button <udid> siri
xcode-canvas-web button <udid> apple-pay
xcode-canvas-web home <udid>
xcode-canvas-web app-switcher <udid>
xcode-canvas-web rotate-left <udid>
xcode-canvas-web rotate-right <udid>
xcode-canvas-web toggle-appearance <udid>
```

Manage simulator app state and pasteboard:

```sh
xcode-canvas-web install <udid> /path/to/App.app
xcode-canvas-web uninstall <udid> com.example.App
xcode-canvas-web erase <udid>
xcode-canvas-web pasteboard set <udid> "text"
xcode-canvas-web pasteboard get <udid>
```

Read diagnostics and chrome metadata:

```sh
xcode-canvas-web logs <udid> --seconds 30 --limit 200
xcode-canvas-web chrome-profile <udid>
```

## Current API Shape

The route inventory changes faster than this operator guide. Treat
`server/src/api/routes.rs` as the canonical source for the live API surface.

The most commonly used routes are:

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
- `POST /api/simulators/:udid/app-switcher`
- `POST /api/simulators/:udid/rotate-left`
- `POST /api/simulators/:udid/rotate-right`
- `POST /api/simulators/:udid/toggle-appearance`
- `POST /api/simulators/:udid/refresh`
- `GET /api/simulators/:udid/chrome-profile`
- `GET /api/simulators/:udid/chrome.png`
- `GET /api/simulators/:udid/accessibility-tree`
- `GET /api/simulators/:udid/accessibility-point`
- `POST /api/simulators/:udid/inspector/request`
- `GET /api/simulators/:udid/logs`
- `GET /api/inspector/connect`
- `GET /api/inspector/poll`
- `POST /api/inspector/response`
- `GET /api/client-stream-stats`
- `POST /api/client-stream-stats`

## Important Notes

- The live frame pane comes from the vendored private display bridge.
- The native accessibility tree comes from the local
  `AccessibilityPlatformTranslation` bridge. Do not add an AXe CLI dependency
  for simulator inspection.
- The live video path is WebTransport-only after the Rust server cutover. Do not add `/stream.h264` back as a fallback.
- The CLI intentionally omits AXe-style record-video, MJPEG streaming, raw JPEG streaming, and BGRA piping. Use `screenshot` for still PNG capture and the web UI for live viewing.
- Device chrome comes from CoreSimulator device-type chrome PDFs rendered by `cli/XCWChromeRenderer.*`.
- If you change CLI flags or API routes, update `README.md` and `AGENTS.md` in the same pass.
