# SimDeck Agents Guide

This repository is a local-first simulator control plane. The product goal is a native CLI server that can manage iOS simulators, expose an HTTP API, and serve a React web client that shows live simulator output in the browser.

## Product Shape

- `packages/server/native/` is the native boundary.
- `packages/client/` is the browser UI.
- `skills/simdeck/SKILL.md` is the operator guide for using the tool from Codex.
- `scripts/` holds repeatable build entrypoints.
- `docs/` is the public VitePress documentation site (deployed to GitHub Pages by `.github/workflows/docs.yml`).

The native side should own anything that depends on macOS frameworks, `xcrun simctl`, or private CoreSimulator/SimulatorKit APIs. The web client should stay thin and consume the CLI API.

## Current Architecture

- `packages/server/src/main.rs`
  Owns the CLI entrypoint, Rust subcommands, HTTP server, and static asset serving.
- `packages/server/src/api/routes.rs`
  Defines REST routes for simulator control, health, metrics, and chrome assets.
- `packages/server/src/transport/webrtc.rs`
  Exposes the H.264 WebRTC offer/answer endpoint for browser live video.
- `packages/server/src/webkit.rs`
  Discovers simulator WebKit Remote Inspector targets and bridges WebInspectorUI
  WebSocket traffic to the simulator `webinspectord` binary-plist socket.
- `packages/server/src/simulators/registry.rs`
  Tracks Rust-side simulator session state and lazy native attachment by UDID.
- `packages/server/native/XCWSimctl.*`
  Wraps `xcrun simctl` for discovery, lifecycle management, app launching, URL opening, appearance toggles, and simulator log capture.
- `packages/server/native/DFPrivateSimulatorDisplayBridge.*`
  Owns headless private display frames plus HID-based touch and keyboard injection.
- `packages/server/native/XCWAccessibilityBridge.*`
  Owns private CoreSimulator accessibility snapshots through `AccessibilityPlatformTranslation`.
- `packages/server/native/XCWPrivateSimulatorSession.*`
  Owns one private display bridge per booted simulator plus selectable hardware/software H.264 encode.
- `packages/server/native/bridge/XCWNativeBridge.*`
  Narrow C ABI for simulator control, chrome rendering, and native frame callbacks into Rust.
- `packages/server/native/bridge/XCWNativeSession.*`
  Wraps one Objective-C private simulator session handle for the Rust registry.
- `packages/server/native/XCWPrivateSimulatorBooter.*`
  Uses private `CoreSimulator` APIs for direct simulator boot without launching Simulator.app.
- `packages/server/native/XCWChromeRenderer.*`
  Renders Apple’s CoreSimulator device-type PDF chrome assets into PNGs for the browser.
- `packages/client/src/app/App.tsx`
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

- Keep simulator-native logic in Objective-C under `packages/server/native/`.
- Keep Rust server logic under `packages/server/`.
- Keep browser-only presentation logic in `packages/client/`.
- Keep NativeScript app runtime inspection logic in `packages/nativescript-inspector/`.
- Keep React Native app runtime inspection logic in `packages/react-native-inspector/`.
- Keep Flutter app runtime inspection logic in `packages/flutter-inspector/`.
- Prefer adding a native API endpoint before adding client-only assumptions.
- Do not add a Node or Swift dependency to solve work that already fits in Foundation/AppKit.
- When touching private API usage, keep the adaptation small and explicit and document any simulator/runtime assumptions here.
- Prefer stable CLI subcommands over hidden environment variables.

## Private API Notes

Private simulator behavior is implemented locally in:

- Boot path: `packages/server/native/XCWPrivateSimulatorBooter.*`
- Full live display bridge: `packages/server/native/DFPrivateSimulatorDisplayBridge.*`
- Accessibility bridge: `packages/server/native/XCWAccessibilityBridge.*`

The current repo uses the private boot path, private display bridge, and private accessibility translation bridge directly. The browser streams frames from that bridge, injects touch and keyboard events through the same native session layer, inspects accessibility through `AccessibilityPlatformTranslation`, and renders device chrome from `packages/server/native/XCWChromeRenderer.*`.
CoreSimulator service contexts resolve the active developer directory from `DEVELOPER_DIR`, then `xcode-select -p`, then `/Applications/Xcode.app/Contents/Developer`. The display bridge prefers direct CoreSimulator screen IOSurface callbacks and activates the SimulatorKit offscreen renderable view only if direct callbacks are unavailable.
Accessibility recovery may use simulator launchctl UIKit application state plus hit-tested translations to recover candidate foreground pids; the returned tree must still be rooted at tokenized `AXPTranslator` application objects, because `translationApplicationObjectForPid:` can omit the bridge delegate token after private display lifecycle changes. Full-tree snapshots merge those recovered roots with the private frontmost application translation. Shallow snapshots with `maxDepth <= 2` use the tokenized frontmost application translation directly when it is available, and only run the expensive recovery sweep if frontmost lookup fails, so agent-oriented describe loops avoid launchctl and hit-test recovery overhead. Interactive-only snapshots also prune non-actionable native AX leaves during Objective-C serialization before the Rust-side compacting pass; keep this native pruning conservative so selector taps still retain actionable rows plus their ancestors. When multiple candidate application roots are discovered, serialize all of them in preferred order: non-extension app roots first, then largest translated roots, with `.appex`/PlugIns processes de-prioritized so SpringBoard and Safari app roots stay primary while widgets and WebContent roots remain debuggable. Widget renderer extension roots may report local frames; normalize those roots and children against matching SpringBoard widget placeholder frames before returning the snapshot.
Physical chrome button support uses DeviceKit `chrome.json` input geometry for browser hit targets. Volume, action, mute, Apple Watch digital crown, Watch side button, and Watch left-side button dispatch through `IndigoHIDMessageForHIDArbitrary` with consumer/telephony/vendor HID usage pairs from the device chrome metadata; home, lock, and app-switcher remain on the existing SimulatorKit button paths. Apple Watch Digital Crown rotation dispatches through `IndigoHIDMessageForDigitalCrownEvent` when SimulatorKit exposes it, with `IndigoHIDMessageForScrollEvent(..., target=0x34)` as the fallback. tvOS simulators do not support direct screen touch; browser/API tap maps to Enter, swipe maps to arrow keys, and the native bridge rejects tvOS touch packets before they reach guest `SimulatorHID`. watchOS/tvOS skip dynamic pointer/mouse service warm-up because those guest runtimes abort on unsupported virtual services. Apple TV and Apple Watch simulators are fixed-orientation devices, so client and server rotation paths must not expose or dispatch device rotation for those families.
On macOS/Xcode 27-era CoreSimulator profiles, `mainScreenWidth`, `mainScreenHeight`, and `mainScreenScale` may be absent from `profile.plist`; DeviceKit chrome rendering must read `capabilities.plist` `ScreenDimensionsCapability` or the primary `displays` entry before falling back to the framebuffer mask PDF. If none of those sources produce usable display geometry, the chrome profile must fail instead of returning a tiny synthetic bezel that hides the stream.
Two-point multi-touch dispatch prefers the current SimulatorKit/Indigo packet constructor and falls back to SimDeck's manual Indigo packet adapter. On Xcode 26 SimulatorKit, the constructor expects pixel-space points and stable two-finger movement requires sending `LeftMouseDown` for both `began` and `moved`, then `LeftMouseUp` for `ended`/`cancelled`; using `LeftMouseDragged` for multi-touch moves only advances one contact in UIKit. Do not coalesce multi-touch move packets in the WebSocket or WebRTC control paths, because gesture recognizers need the intermediate two-contact samples.
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

This now builds the Rust server in `packages/server/` and copies the resulting binary to `build/simdeck`.

Codex worktrees can use the checked-in local environment config at
`.codex/local-environment.toml`. Its setup runs `npm run codex:setup`, which
hydrates root and `packages/client` `node_modules` plus
`packages/server/target` from the shared cache under
`~/.cache/simdeck/codex-worktree-cache` or from another SimDeck checkout before
falling back to `npm ci` for missing package installs and ensuring Homebrew
`pkgconf`/`x264` are available for native builds. Its Run action executes
`npm run codex:run`, which builds the CLI and client, saves fresh caches, and
restarts the local service.

Run the local service:

```sh
./build/simdeck
./build/simdeck -p 4311
```

Running without a subcommand starts or reuses the background service, prints
local and LAN HTTP URLs, and prints a six-digit pairing code for LAN browsers.
Pass a simulator name or UDID as the only argument to select it by default in
the UI. Use `./build/simdeck -a` or `./build/simdeck pair` when the service
should be registered as a LaunchAgent.

Use software H.264 when macOS screen recording starves the hardware encoder:

```sh
./build/simdeck service restart --video-codec h264-software
```

For LAN access:

```sh
./build/simdeck -p 4311 --bind 0.0.0.0 --advertise-host 192.168.1.50
```

Useful direct commands:

```sh
./build/simdeck list
./build/simdeck use <udid>
./build/simdeck boot <udid>
./build/simdeck shutdown
./build/simdeck erase
./build/simdeck install /path/to/App.app
./build/simdeck uninstall com.example.App
./build/simdeck open-url https://example.com
./build/simdeck launch com.apple.Preferences
./build/simdeck pasteboard set "hello"
./build/simdeck pasteboard get
./build/simdeck screenshot --output screen.png
./build/simdeck screenshot --with-bezel --output screen-bezel.png
./build/simdeck record --seconds 5 --output screen-recording.mp4
./build/simdeck describe --format agent --max-depth 4 -i
./build/simdeck wait-for --label "Welcome" --timeout-ms 5000
./build/simdeck tap 120 240
./build/simdeck tap --label "Continue" --wait-timeout-ms 5000
./build/simdeck tap "Continue"
./build/simdeck tap --id com.apple.settings.screenTime --expect-id BackButton
./build/simdeck back
./build/simdeck swipe 200 700 200 200
./build/simdeck gesture scroll-down
./build/simdeck pinch --start-distance 160 --end-distance 80
./build/simdeck rotate-gesture --radius 100 --degrees 90
./build/simdeck key-sequence --keycodes h,e,l,l,o
./build/simdeck key-combo --modifiers cmd --key a
./build/simdeck type "hello"
./build/simdeck button lock --duration-ms 1000
./build/simdeck home
```

Most simulator commands accept `[<udid>]`; when it is omitted, SimDeck uses
`--device`, `SIMDECK_DEVICE`, `SIMDECK_UDID`, the saved project default, or the
only booted simulator, in that order. For agent navigation, prefer
`describe -i`, `wait-for`, `tap --id/--label`, `tap "Text"`, `back`, and
`batch` over coordinate-only loops.

## Expectations For Future Changes

- If you add an API route, add the matching client affordance or document why it stays CLI-only.
- If you change the CLI invocation shape, update `README.md` and `skills/simdeck/SKILL.md` in the same pass.
- If you change a CLI flag, REST route, stream contract, or inspector method, update the matching page under `docs/` in the same pass.
- If you expand the private framework bridge, document the Xcode/runtime assumptions here.
- If a feature depends on a booted simulator, fail with a clear JSON error instead of silently returning an empty asset.
- Do not reintroduce removed `/stream.h264` handling. The supported live path is the Rust-managed WebRTC H.264 offer endpoint.
