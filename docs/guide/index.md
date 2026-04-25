# Introduction

SimDeck is a local-first control plane for the iOS Simulator. It bundles a Rust HTTP server, a native Objective-C bridge for private CoreSimulator and SimulatorKit APIs, and a React client into a single CLI you can run on any modern Mac.

The goal is simple: turn a booted Simulator into a streamable, scriptable surface that any tool — a browser, VS Code, a NativeScript runtime, an automation script — can drive over plain HTTP.

## Why SimDeck?

The default Simulator is great when it sits in front of you. It is much less great when:

- You want to see your app running while writing code in another window.
- You want to drive a Simulator from a remote machine on your LAN.
- You want to build automation around `simctl` without stitching together shell pipelines.
- You want to inspect a NativeScript or Swift app's view hierarchy without linking the Xcode debugger.
- You want a single, predictable URL that always points at "the Simulator on this Mac".

SimDeck addresses all of those by exposing one HTTP server, one WebTransport endpoint, and one CLI binary.

## What's in the box

SimDeck ships as a single npm package (`simdeck`) that installs three things:

1. **A native CLI server.** Rust + Objective-C, compiled on install. It serves the HTTP API and a self-signed WebTransport endpoint for live video frames.
2. **A bundled React client.** Talks to the local server, renders a streamable Simulator surface, and ships the inspector UIs.
3. **A `launchd` integration.** A single `service on` flag installs a per-user background service that survives logout and restart.

Optional companion packages:

- [`@nativescript/simdeck-inspector`](/inspector/nativescript) — a debug-only NativeScript inspector runtime.
- [`packages/inspector-agent`](/inspector/swift) — a Swift Package you can link from your iOS app to expose its UIKit hierarchy.
- [`packages/vscode-extension`](/extensions/vscode) — opens the simulator inside a VS Code panel.

## High-level architecture

The repository splits cleanly along the layers SimDeck talks to:

- **`server/`** holds the Rust HTTP server, WebTransport hub, inspector hub, and metrics. It serves the REST API at `/api/*`, live video at `/wt/simulators/{udid}`, and the inspector WebSocket at `/api/inspector/connect`.
- **`cli/`** holds the Objective-C native bridge that links private `CoreSimulator` and `SimulatorKit` APIs. The Rust server calls into it through a narrow C ABI for boot, frame capture, encode, and HID input.
- **`client/`** holds the React UI that renders the streamed simulator and the inspector tools.
- **`packages/`** holds optional companion packages (NativeScript inspector, Swift inspector agent, VS Code extension).
- **`scripts/`** holds repeatable build entrypoints used both locally and by CI.

For a full breakdown, see [Architecture](/guide/architecture).

## Where to next

- **[Installation](/guide/installation)** — how to install the CLI from npm or build it from source.
- **[Quick Start](/guide/quick-start)** — boot a simulator and stream it to your browser in under a minute.
- **[Architecture](/guide/architecture)** — the layout, data flow, and private-API surface.
- **[CLI Reference](/cli/commands)** — every command with its flags.
- **[HTTP API](/api/rest)** — every REST endpoint, with response shapes.
