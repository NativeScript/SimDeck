# Xcode Canvas Web Agents Guide

This repository is a local-first simulator control plane. The product goal is a native CLI server that can manage iOS simulators, expose an HTTP API, and serve a React web client that shows live simulator output in the browser.

## Product Shape

- `cli/` is the native boundary.
- `client/` is the browser UI.
- `skills/xcode-canvas-web/SKILL.md` is the operator guide for using the tool from Codex.
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
  Wraps `xcrun simctl` for discovery, lifecycle management, app launching, URL opening, and screenshot capture.
- `cli/DFPrivateSimulatorDisplayBridge.*`
  Owns headless private display frames plus HID-based touch and keyboard injection.
- `cli/XCWPrivateSimulatorSession.*`
  Owns one private display bridge per booted simulator plus selectable HEVC/H.264 encode.
- `cli/native/XCWNativeBridge.*`
  Narrow C ABI for simulator control, chrome rendering, and native frame callbacks into Rust.
- `cli/native/XCWNativeSession.*`
  Wraps one Objective-C private simulator session handle for the Rust registry.
- `cli/XCWPrivateSimulatorBooter.*`
  Uses private `CoreSimulator` APIs for direct simulator boot when available, with `simctl` as the fallback path.
- `cli/XCWPrivateSimulatorChromeBridge.*`
  Experimental private `SimulatorKit` chrome bridge for simulator chrome exploration.
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
- Chrome asset bridge: `cli/XCWPrivateSimulatorChromeBridge.*`
- Full live display bridge: `cli/DFPrivateSimulatorDisplayBridge.*`

The current repo uses the private boot path and private display bridge directly. The browser streams frames from that bridge and injects touch and keyboard events through the same native session layer.

## Build and Run

Build the browser bundle:

```sh
./scripts/build-client.sh
```

Build the native CLI:

```sh
./scripts/build-cli.sh
```

This now builds the Rust server in `server/` and copies the resulting binary to `build/xcode-canvas-web`.

Run the local server:

```sh
./build/xcode-canvas-web serve --port 4310
```

Use software H.264 when macOS screen recording starves the hardware encoder:

```sh
./build/xcode-canvas-web serve --port 4310 --video-codec h264-software
```

For LAN access:

```sh
./build/xcode-canvas-web serve --port 4310 --bind 0.0.0.0 --advertise-host 192.168.1.50
```

Useful direct commands:

```sh
./build/xcode-canvas-web list
./build/xcode-canvas-web boot <udid>
./build/xcode-canvas-web shutdown <udid>
./build/xcode-canvas-web open-url <udid> https://example.com
./build/xcode-canvas-web launch <udid> com.apple.Preferences
```

## Expectations For Future Changes

- If you add an API route, add the matching client affordance or document why it stays CLI-only.
- If you change the CLI invocation shape, update `README.md` and `skills/xcode-canvas-web/SKILL.md` in the same pass.
- If you change a CLI flag, REST route, packet format, or inspector method, update the matching page under `docs/` in the same pass.
- If you expand the private framework bridge, document the Xcode/runtime assumptions here.
- If a feature depends on a booted simulator, fail with a clear JSON error instead of silently returning an empty asset.
- Do not reintroduce legacy `/stream.h264` handling. The supported live path is Rust-managed WebTransport.

## Near-Term Roadmap

- Compose the private frame stream and CoreSimulator chrome into a single server-side render path.
- Add richer input surfaces such as gesture synthesis, text entry helpers, and chrome-button actions on top of the HID bridge.
- Add simulator creation, erase, pasteboard, install, and log streaming commands.
