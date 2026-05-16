# SimDeck Agents Guide

This repository is a local-first simulator control plane. The product goal is a native CLI server that can manage iOS simulators, expose an HTTP API, and serve a React web client that shows live simulator output in the browser.

## Product Shape

- `cli/` is the native boundary.
- `client/` is the browser UI.
- `skills/simdeck/SKILL.md` is the operator guide for using the tool from Codex.
- `scripts/` holds repeatable build entrypoints.
- `docs/` is the public VitePress documentation site (deployed to GitHub Pages by `.github/workflows/docs.yml`).

The native side should own anything that depends on macOS frameworks, `xcrun simctl`, or private CoreSimulator/SimulatorKit APIs. The web client should stay thin and consume the CLI API.

## Current Architecture

- `server/src/main.rs`
  Owns the CLI entrypoint, Rust subcommands, HTTP server, and static asset serving.
- `server/src/api/routes.rs`
  Defines REST routes for simulator control, health, metrics, and chrome assets.
- `server/src/transport/webrtc.rs`
  Exposes the H.264 WebRTC offer/answer endpoint for browser live video.
- `server/src/webkit.rs`
  Discovers simulator WebKit Remote Inspector targets and bridges WebInspectorUI
  WebSocket traffic to the simulator `webinspectord` binary-plist socket.
- `server/src/simulators/registry.rs`
  Tracks Rust-side simulator session state and lazy native attachment by UDID.
- `cli/XCWSimctl.*`
  Wraps `xcrun simctl` for discovery, lifecycle management, app launching, URL opening, appearance toggles, and simulator log capture.
- `cli/DFPrivateSimulatorDisplayBridge.*`
  Owns headless private display frames plus HID-based touch and keyboard injection.
- `cli/XCWAccessibilityBridge.*`
  Owns private CoreSimulator accessibility snapshots through `AccessibilityPlatformTranslation`.
- `cli/XCWPrivateSimulatorSession.*`
  Owns one private display bridge per booted simulator plus selectable hardware/software H.264 encode.
- `cli/native/XCWNativeBridge.*`
  Narrow C ABI for simulator control, chrome rendering, and native frame callbacks into Rust.
- `cli/native/XCWNativeSession.*`
  Wraps one Objective-C private simulator session handle for the Rust registry.
- `cli/XCWPrivateSimulatorBooter.*`
  Uses private `CoreSimulator` APIs for direct simulator boot without launching Simulator.app.
- `cli/XCWChromeRenderer.*`
  Renders Apple’s CoreSimulator device-type PDF chrome assets into PNGs for the browser.
- `client/src/app/App.tsx`
  Browser entrypoint for the React control surface.
- `packages/nativescript-inspector/src/index.ts`
  NativeScript in-app inspector runtime that connects to the Rust server over
  WebSocket, publishes NativeScript/UIKit hierarchies, and performs debug UIKit
  property edits from JavaScript.
- `packages/react-native-inspector/src/index.ts`
  React Native in-app inspector runtime that connects to the Rust server over
  WebSocket, publishes React Fiber component hierarchies with Metro source
  locations, and performs best-effort debug JS/native prop edits.
- `packages/flutter-inspector/lib/simdeck_flutter_inspector.dart`
  Flutter in-app inspector runtime that connects to the Rust server over
  WebSocket, publishes widget/render/semantics hierarchies with debug creation
  locations, and performs best-effort semantics, focus, text, and scroll actions.

## Working Rules

- Keep simulator-native logic in Objective-C under `cli/`.
- Keep Rust server logic under `server/`.
- Keep browser-only presentation logic in `client/`.
- Keep NativeScript app runtime inspection logic in `packages/nativescript-inspector/`.
- Keep React Native app runtime inspection logic in `packages/react-native-inspector/`.
- Keep Flutter app runtime inspection logic in `packages/flutter-inspector/`.
- Prefer adding a native API endpoint before adding client-only assumptions.
- Do not add a Node or Swift dependency to solve work that already fits in Foundation/AppKit.
- When touching private API usage, keep the adaptation small and explicit and document any simulator/runtime assumptions here.
- Prefer stable CLI subcommands over hidden environment variables.

## Private API Notes

Private simulator behavior is implemented locally in:

- Boot path: `cli/XCWPrivateSimulatorBooter.*`
- Full live display bridge: `cli/DFPrivateSimulatorDisplayBridge.*`
- Accessibility bridge: `cli/XCWAccessibilityBridge.*`

The current repo uses the private boot path, private display bridge, and private accessibility translation bridge directly. The browser streams frames from that bridge, injects touch and keyboard events through the same native session layer, inspects accessibility through `AccessibilityPlatformTranslation`, and renders device chrome from `cli/XCWChromeRenderer.*`.
CoreSimulator service contexts resolve the active developer directory from `DEVELOPER_DIR`, then `xcode-select -p`, then `/Applications/Xcode.app/Contents/Developer`. The display bridge prefers direct CoreSimulator screen IOSurface callbacks and activates the SimulatorKit offscreen renderable view only if direct callbacks are unavailable.
Accessibility recovery may use simulator launchctl UIKit application state plus hit-tested translations to recover candidate foreground pids; the returned tree must still be rooted at tokenized `AXPTranslator` application objects, because `translationApplicationObjectForPid:` can omit the bridge delegate token after private display lifecycle changes. Full-tree snapshots merge those recovered roots with the private frontmost application translation. When multiple candidate application roots are discovered, serialize all of them in preferred order: non-extension app roots first, then largest translated roots, with `.appex`/PlugIns processes de-prioritized so SpringBoard and Safari app roots stay primary while widgets and WebContent roots remain debuggable. Widget renderer extension roots may report local frames; normalize those roots and children against matching SpringBoard widget placeholder frames before returning the snapshot.
Physical chrome button support uses DeviceKit `chrome.json` input geometry for browser hit targets. Volume, action, mute, Apple Watch digital crown, Watch side button, and Watch left-side button dispatch through `IndigoHIDMessageForHIDArbitrary` with consumer/telephony/vendor HID usage pairs from the device chrome metadata; home, lock, and app-switcher remain on the existing SimulatorKit button paths. Apple Watch Digital Crown rotation dispatches through `IndigoHIDMessageForScrollEvent` with the same digitizer target as touch input.
WebKit inspection uses the simulator `webinspectord` Unix socket named `com.apple.webinspectord_sim.socket` and WebKit's binary-plist Remote Inspector selectors. It lists only WebKit content that the runtime exposes as inspectable. For app-owned `WKWebView` on iOS 16.4 and newer, the app must set `isInspectable = true`.

## Build and Run

Build the native CLI and browser bundle:

```sh
npm run build
```

Build individual pieces when needed:

```sh
npm run build:cli
npm run build:client
npm run build:all
npm run package:vscode
```

This now builds the Rust server in `server/` and copies the resulting binary to `build/simdeck`.

Run the local daemon:

```sh
./build/simdeck
./build/simdeck daemon start --port 4310
```

Running without a subcommand starts a foreground workspace daemon, prints local and LAN HTTP URLs, prints a six-digit pairing code for LAN browsers, and stops when the command exits, when you press `q`, or when you press Ctrl-C. Pass a simulator name or UDID as the only argument to select it by default in the UI. Use `./build/simdeck -d`, `./build/simdeck -k`, and `./build/simdeck -r` as detached start, kill, and restart shortcuts.

Use software H.264 when macOS screen recording starves the hardware encoder:

```sh
./build/simdeck daemon start --port 4310 --video-codec h264-software
```

For LAN access:

```sh
./build/simdeck daemon start --port 4310 --bind 0.0.0.0 --advertise-host 192.168.1.50
```

Useful direct commands:

```sh
./build/simdeck list
./build/simdeck boot <udid>
./build/simdeck shutdown <udid>
./build/simdeck erase <udid>
./build/simdeck install <udid> /path/to/App.app
./build/simdeck uninstall <udid> com.example.App
./build/simdeck open-url <udid> https://example.com
./build/simdeck launch <udid> com.apple.Preferences
./build/simdeck pasteboard set <udid> "hello"
./build/simdeck pasteboard get <udid>
./build/simdeck screenshot <udid> --output screen.png
./build/simdeck screenshot <udid> --with-bezel --output screen-bezel.png
./build/simdeck record <udid> --seconds 5 --output screen-recording.mp4
./build/simdeck describe <udid>
./build/simdeck tap <udid> 120 240
./build/simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
./build/simdeck swipe <udid> 200 700 200 200
./build/simdeck gesture <udid> scroll-down
./build/simdeck pinch <udid> --start-distance 160 --end-distance 80
./build/simdeck rotate-gesture <udid> --radius 100 --degrees 90
./build/simdeck key-sequence <udid> --keycodes h,e,l,l,o
./build/simdeck key-combo <udid> --modifiers cmd --key a
./build/simdeck type <udid> "hello"
./build/simdeck button <udid> lock --duration-ms 1000
./build/simdeck home <udid>
```

## Expectations For Future Changes

- If you add an API route, add the matching client affordance or document why it stays CLI-only.
- If you change the CLI invocation shape, update `README.md` and `skills/simdeck/SKILL.md` in the same pass.
- If you change a CLI flag, REST route, stream contract, or inspector method, update the matching page under `docs/` in the same pass.
- If you expand the private framework bridge, document the Xcode/runtime assumptions here.
- If a feature depends on a booted simulator, fail with a clear JSON error instead of silently returning an empty asset.
- Do not reintroduce legacy `/stream.h264` handling. The supported live path is the Rust-managed WebRTC H.264 offer endpoint.

## Near-Term Roadmap

- Compose the private frame stream and CoreSimulator chrome into a single server-side render path.
- Keep private Indigo multi-touch packet assumptions documented when Xcode runtimes change.
- Add simulator creation and log streaming commands.
