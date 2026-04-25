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
  Owns the CLI entrypoint, Rust subcommands, HTTP server, static asset serving, and WebTransport startup.
- `server/src/api/routes.rs`
  Defines REST routes for simulator control, health, metrics, and chrome assets.
- `server/src/transport/webtransport.rs`
  Exposes one WebTransport session path per simulator and streams binary video packets.
- `server/src/simulators/registry.rs`
  Tracks Rust-side simulator session state and lazy native attachment by UDID.
- `cli/XCWSimctl.*`
  Wraps `xcrun simctl` for discovery, lifecycle management, app launching, URL opening, appearance toggles, and simulator log capture.
- `cli/DFPrivateSimulatorDisplayBridge.*`
  Owns headless private display frames plus HID-based touch and keyboard injection.
- `cli/XCWAccessibilityBridge.*`
  Owns private CoreSimulator accessibility snapshots through `AccessibilityPlatformTranslation`.
- `cli/XCWPrivateSimulatorSession.*`
  Owns one private display bridge per booted simulator plus selectable HEVC/H.264 encode.
- `cli/native/XCWNativeBridge.*`
  Narrow C ABI for simulator control, chrome rendering, and native frame callbacks into Rust.
- `cli/native/XCWNativeSession.*`
  Wraps one Objective-C private simulator session handle for the Rust registry.
- `cli/XCWPrivateSimulatorBooter.*`
  Uses private `CoreSimulator` APIs for direct simulator boot when available, with `simctl` as the fallback path.
- `cli/XCWChromeRenderer.*`
  Renders Apple’s CoreSimulator device-type PDF chrome assets into PNGs for the browser.
- `client/src/app/App.tsx`
  Browser entrypoint for the React control surface.
- `packages/nativescript-inspector/src/index.ts`
  NativeScript in-app inspector runtime that connects to the Rust server over
  WebSocket, publishes NativeScript/UIKit hierarchies, and performs debug UIKit
  property edits from JavaScript.

## Working Rules

- Keep simulator-native logic in Objective-C under `cli/`.
- Keep Rust server logic under `server/`.
- Keep browser-only presentation logic in `client/`.
- Keep NativeScript app runtime inspection logic in `packages/nativescript-inspector/`.
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

## Build and Run

Build the browser bundle:

```sh
./scripts/build-client.sh
```

Build the native CLI:

```sh
./scripts/build-cli.sh
```

This now builds the Rust server in `server/` and copies the resulting binary to `build/simdeck`.

Run the local server:

```sh
./build/simdeck serve --port 4310
```

Use software H.264 when macOS screen recording starves the hardware encoder:

```sh
./build/simdeck serve --port 4310 --video-codec h264-software
```

For LAN access:

```sh
./build/simdeck serve --port 4310 --bind 0.0.0.0 --advertise-host 192.168.1.50
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
./build/simdeck describe-ui <udid>
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
- If you change a CLI flag, REST route, packet format, or inspector method, update the matching page under `docs/` in the same pass.
- If you expand the private framework bridge, document the Xcode/runtime assumptions here.
- If a feature depends on a booted simulator, fail with a clear JSON error instead of silently returning an empty asset.
- Do not reintroduce legacy `/stream.h264` handling. The supported live path is Rust-managed WebTransport.

## Near-Term Roadmap

- Compose the private frame stream and CoreSimulator chrome into a single server-side render path.
- Keep private Indigo multi-touch packet assumptions documented when Xcode runtimes change.
- Add simulator creation and log streaming commands.
