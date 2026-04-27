# Introduction

SimDeck is a local-first control plane for the iOS Simulator. It bundles a Rust HTTP server, a native Objective-C bridge for CoreSimulator and SimulatorKit APIs, a React browser UI, and a JS/TS test API into one project-local CLI.

The goal is simple: turn a booted Simulator into a streamable, scriptable surface that any tool — a browser, VS Code, a NativeScript runtime, or an automated test — can drive through one local daemon.

## Why SimDeck?

The default Simulator is great when it sits in front of you. It is much less great when:

- You want to see your app running while writing code in another window.
- You want to drive a Simulator from a remote machine on your LAN.
- You want to build automation around `simctl` without stitching together shell pipelines.
- You want to inspect a NativeScript or Swift app's view hierarchy without linking the Xcode debugger.
- You want a project daemon that stays warm instead of cold-starting native simulator control for every command.

SimDeck addresses all of those with one CLI, one HTTP API, one WebTransport endpoint, and one daemon per project.

## What's in the box

SimDeck ships as a single npm package (`simdeck`) that installs:

1. **A native CLI and project daemon.** Rust + Objective-C, compiled on install. It serves the HTTP API and a self-signed WebTransport endpoint for live video frames.
2. **A bundled React client.** `simdeck ui --open` starts or reuses the daemon, renders a live Simulator surface, and ships the inspector UI.
3. **A JS/TS testing package.** `simdeck/test` gives app tests a small API for launching, tapping, querying accessibility state, batching actions, and taking screenshots.

Optional companion packages:

- [`@nativescript/simdeck-inspector`](/inspector/nativescript) — a debug-only NativeScript inspector runtime.
- [`@simdeck/react-native-inspector`](/inspector/react-native) — a debug-only React Native inspector runtime.
- [`packages/inspector-agent`](/inspector/swift) — a Swift Package you can link from your iOS app to expose its UIKit hierarchy.
- [`packages/vscode-extension`](/extensions/vscode) — opens the simulator inside a VS Code panel.

## High-level architecture

The repository splits cleanly along the layers SimDeck talks to:

- **`server/`** holds the CLI entrypoint, project daemon, Rust HTTP server, WebTransport hub, inspector hub, and metrics. It serves the REST API at `/api/*`, live video at `/wt/simulators/{udid}`, and the inspector WebSocket at `/api/inspector/connect`.
- **`cli/`** holds the Objective-C native bridge that links private `CoreSimulator` and `SimulatorKit` APIs. The Rust server calls into it through a narrow C ABI for boot, frame capture, encode, and HID input.
- **`client/`** holds the React UI that renders the streamed simulator and the inspector tools.
- **`packages/`** holds companion packages: NativeScript inspector, React Native inspector, Swift inspector agent, VS Code extension, and `simdeck/test`.
- **`scripts/`** holds repeatable build entrypoints used both locally and by CI.

For a full breakdown, see [Architecture](/guide/architecture).

## Where to next

- **[Installation](/guide/installation)** — how to install the CLI from npm or build it from source.
- **[Quick Start](/guide/quick-start)** — boot a simulator and stream it to your browser in under a minute.
- **[Project Daemon](/guide/daemon)** — how `ui`, `daemon start`, and automatic daemon reuse work.
- **[Testing](/guide/testing)** — use `simdeck/test` and run the simulator-backed integration suite.
- **[Architecture](/guide/architecture)** — the layout, data flow, and private-API surface.
- **[CLI Reference](/cli/commands)** — every command with its flags.
- **[HTTP API](/api/rest)** — every REST endpoint, with response shapes.
