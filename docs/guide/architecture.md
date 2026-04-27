# Architecture

SimDeck is intentionally split into a small number of clearly-scoped layers. Every layer has a single concern and a single owner directory in the repo.

## High-level layout

SimDeck has three layers stacked between the browser and the iOS Simulator:

1. **Browser / VS Code** runs the React client from `client/`. It speaks HTTP for control and WebTransport for live video, both served by the Rust server.
2. **The Rust server** (`server/`, built on `axum` + `tokio`) owns the CLI entrypoint, project daemon lifecycle, REST routes (`api/`), the WebTransport hub and packet codec (`transport/`), the inspector WebSocket hub (`inspector.rs`), the per-UDID session registry (`simulators/`), metrics, and log streaming.
3. **The Objective-C bridge** (`cli/`) is reached through a narrow C ABI in `cli/native/XCWNativeBridge.*`. It wraps `xcrun simctl`, the private `CoreSimulator` direct-boot path, the per-session HEVC/H.264 encoder, the headless display bridge that produces frames and accepts HID input, and the device-chrome renderer.

Underneath all of that is the iOS Simulator itself — `CoreSimulator` for lifecycle, `SimulatorKit` for chrome assets.

## Layer responsibilities

### `server/` — Rust HTTP and WebTransport

Owns the public CLI shape (`simdeck ui`, `daemon`, `boot`, `shutdown`, …), daemon metadata, the HTTP API, the WebTransport hub, the inspector hub, log streaming, and metrics.

Key modules:

| Module                                 | Responsibility                                                                               |
| -------------------------------------- | -------------------------------------------------------------------------------------------- |
| `server/src/main.rs`                   | CLI entrypoint, project daemon management, AppKit main-thread shim, tokio runtime bootstrap. |
| `server/src/api/routes.rs`             | Every `/api/*` route, including simulator control, accessibility, and inspector proxy.       |
| `server/src/transport/webtransport.rs` | WebTransport server, per-session frame fanout, keyframe handshake.                           |
| `server/src/transport/packet.rs`       | Binary video packet header (`PACKET_VERSION`, flags, layout).                                |
| `server/src/inspector.rs`              | WebSocket hub for the NativeScript runtime inspector.                                        |
| `server/src/simulators/registry.rs`    | Per-UDID session registry with lazy attachment to the native bridge.                         |
| `server/src/simulators/session.rs`     | Frame broadcast channel, keyframe gating, refresh requests.                                  |
| `server/src/metrics/counters.rs`       | Atomic counters and per-client stream stats accepted via `/api/client-stream-stats`.         |
| `server/src/logs.rs`                   | `os_log` log streaming and filtering.                                                        |

The Rust server runs the tokio runtime on a worker thread while the AppKit main loop spins on the main thread. The native bridge needs the main loop to deliver display callbacks and HID events.

### `cli/` — Objective-C native bridge

Anything that depends on macOS frameworks, `xcrun simctl`, or private `CoreSimulator` / `SimulatorKit` APIs lives here. The Rust side talks to it through a narrow C ABI:

- `cli/native/XCWNativeBridge.{h,m}` — exported C functions for simulator control, chrome rendering, and frame callbacks.
- `cli/native/XCWNativeSession.{h,m}` — wraps one Objective-C private simulator session handle for the Rust registry.

Inside the bridge:

- **`XCWSimctl.{h,m}`** wraps `xcrun simctl` for discovery, lifecycle management, app launching, URL opening, and screenshot capture.
- **`XCWPrivateSimulatorBooter.{h,m}`** uses private `CoreSimulator` APIs for direct simulator boot when available, with `simctl` as the fallback path.
- **`DFPrivateSimulatorDisplayBridge.{h,m}`** owns headless private display frames plus HID-based touch and keyboard injection.
- **`XCWPrivateSimulatorSession.{h,m}`** owns one private display bridge per booted simulator plus a selectable HEVC or H.264 encoder.
- **`XCWPrivateSimulatorChromeBridge.{h,m}`** is an experimental private `SimulatorKit` chrome bridge kept nearby as a reference.
- **`XCWChromeRenderer.{h,m}`** renders Apple's CoreSimulator device-type PDF chrome assets into PNGs for the browser.
- **`XCWH264Encoder.{h,m}`** software / hardware H.264 encode used as a fallback when HEVC is starved.

### `client/` — React browser UI

The React app served at `/` is a thin shell that calls the REST API and consumes binary video over WebTransport.

Layout under `client/src/`:

- `app/AppShell.tsx` — top-level shell.
- `api/` — typed wrappers around `/api/*` (`client.ts`, `controls.ts`, `simulators.ts`, `types.ts`).
- `features/stream/` — WebTransport reader, decoder workers, frame renderer.
- `features/viewport/` — frame canvas, hit testing, chrome compositing.
- `features/input/` — touch/keyboard/hardware button affordances.
- `features/accessibility/` — accessibility tree pane and source switcher.
- `features/simulators/` — simulator list, boot/shutdown affordances.
- `features/toolbar/` — top toolbar (rotate, home, app switcher, dark mode toggle).
- `workers/` — video decode workers.

The client never depends on private APIs and never assumes anything not exposed by the HTTP API.

### `packages/` — companion packages

- **`packages/nativescript-inspector/`** ships `@nativescript/simdeck-inspector`, a TypeScript runtime that connects from a NativeScript app to the server's WebSocket inspector hub. See [NativeScript Runtime](/inspector/nativescript).
- **`packages/react-native-inspector/`** ships `@simdeck/react-native-inspector`, a React Native runtime that connects from an app to the server's WebSocket inspector hub and publishes React Fiber hierarchy data. See [React Native Runtime](/inspector/react-native).
- **`packages/inspector-agent/`** ships `SimDeckInspectorAgent`, a Swift Package you can link from a debug iOS app to expose its UIKit hierarchy. See [Swift In-App Agent](/inspector/swift).
- **`packages/vscode-extension/`** is the VS Code extension that opens the browser client inside a webview panel and auto-starts the server.
- **`packages/simdeck-test/`** ships `simdeck/test`, a small JS/TS wrapper around daemon startup and the REST control API. See [Testing](/guide/testing).

## Data flow

### Simulator control

Most control endpoints follow the same path: a typed Rust handler in `server/src/api/routes.rs` calls `SessionRegistry::bridge()`, which dispatches into `cli/native/XCWNativeBridge.*` over the C ABI. From there the call lands in the matching Objective-C unit — for example, `POST /api/simulators/{udid}/boot` ends up in `XCWPrivateSimulatorBooter`, which uses private `CoreSimulator` APIs for direct boot and falls back to `simctl` if that fails.

### Live video

The browser opens a WebTransport session at `https://host:4311/wt/simulators/{udid}`. The handler in `transport::webtransport::handle_session` ensures the per-UDID `SimulatorSession` is started, waits up to ~3 s for the first keyframe, then opens two unidirectional streams to the client: a control stream that carries a single JSON `ControlHello` describing the codec, and a video stream that carries binary frame packets fanned out from `SimulatorSession.subscribe()`.

Each binary packet has a fixed-size 36-byte header followed by an optional codec configuration (description) blob and the encoded video data. See [WebTransport](/api/webtransport) and [Packet Format](/api/packet-format) for the wire layout.

### Input

Touch and keyboard events POST to `/api/simulators/{udid}/touch` and `/key`. The handler resolves the active session and replays the event through the private display bridge using HID.

### Inspectors

The accessibility tree endpoint blends three sources, in priority order:

1. **NativeScript runtime inspector** — preferred when the foreground app has connected to `/api/inspector/connect` over WebSocket.
2. **Swift in-app inspector agent** — used when the foreground app links the `SimDeckInspectorAgent` Swift Package and listens on a TCP port discovered between `47370` and `47402`.
3. **Accessibility snapshot** — a final fallback that shells out to the accessibility snapshot

The server discovers which inspectors are reachable for a given Simulator and surfaces the available list in the `availableSources` field on every accessibility-tree response.

## Process model

SimDeck stays in one OS process. The Rust binary:

1. Calls `xcw_native_initialize_app()` so AppKit creates an `NSApplication` on the main thread.
2. Spawns a tokio runtime on a worker thread that owns the HTTP server, WebTransport server, inspector hub, and registry.
3. Spins the AppKit main loop in 50 ms slices on the main thread to dispatch display and HID callbacks.

Normal CLI commands may spawn `simdeck daemon run` in the background for the current project. The daemon writes metadata under the system temp directory, and later commands reuse it while `/api/health` stays healthy.

## Working rules

If you contribute, keep the following invariants in mind:

- Simulator-native logic stays in Objective-C under `cli/`.
- Rust server logic stays under `server/`.
- Browser-only presentation logic stays in `client/`.
- NativeScript app runtime inspection logic stays in `packages/nativescript-inspector/`.
- Add a server endpoint before adding client-only assumptions.
- The supported live video path is WebTransport-only — do not bring back legacy `/stream.h264` handling.
