---
layout: home

hero:
  name: SimDeck
  text: Local iOS Simulator control plane
  tagline: A project-local CLI, native daemon, browser UI, and JS test API for streaming, inspecting, and driving iOS Simulators on macOS.
  actions:
    - theme: brand
      text: Get Started
      link: /guide/quick-start
    - theme: alt
      text: Why SimDeck?
      link: /guide/
    - theme: alt
      text: View on GitHub
      link: https://github.com/DjDeveloperr/SimDeck

features:
  - icon:
      src: /icons/monitor-smartphone.svg
      width: 28
      height: 28
    title: Browser-first simulator
    details: "`simdeck ui --open` starts or reuses a project daemon and opens a React UI with live WebTransport video, touch, keyboard, hardware-button, and rotation input."
  - icon:
      src: /icons/zap.svg
      width: 28
      height: 28
    title: Native macOS performance
    details: "A Rust HTTP server fronts an Objective-C bridge that talks to CoreSimulator, SimulatorKit, private display APIs, and HEVC or H.264 encoders."
  - icon:
      src: /icons/network.svg
      width: 28
      height: 28
    title: Stable HTTP control plane
    details: "The daemon exposes simulator lifecycle, input, accessibility, logs, chrome assets, and inspector control through a single REST API."
  - icon:
      src: /icons/scan-search.svg
      width: 28
      height: 28
    title: First-class inspectors
    details: "`describe` and the UI prefer NativeScript or UIKit in-app inspectors when available, then fall back to private CoreSimulator accessibility snapshots."
  - icon:
      src: /icons/puzzle.svg
      width: 28
      height: 28
    title: Built-in extensions
    details: "A VS Code extension opens the simulator inside the editor, and `simdeck/test` gives JS/TS tests a fast API for app automation."
  - icon:
      src: /icons/shield-check.svg
      width: 28
      height: 28
    title: Local-first by default
    details: "Binds to 127.0.0.1 by default, runs without a cloud account, and can be opened to your LAN with explicit bind and advertise flags."
---

<div class="vp-doc" style="max-width: 1152px; margin: 4rem auto 0; padding: 0 24px;">

## What you can do with SimDeck

SimDeck packages a full simulator workflow into one cross-tool surface:

- **Stream a Simulator into a browser tab.** Run `simdeck ui --open` and use the same warm project daemon from the browser and CLI.
- **Drive Simulators from JavaScript.** `simdeck/test` can launch apps, tap, wait for accessibility state, batch steps, and capture screenshots.
- **Embed a Simulator in your editor.** The bundled VS Code extension opens the same surface inside a panel.
- **Run Simulators on your LAN.** Bind to `0.0.0.0`, advertise a host, and connect from any other Mac, iPad, or laptop on the network.
- **Replace ad-hoc `simctl` scripts.** A single CLI handles `boot`, `shutdown`, app install/launch, URL opening, pasteboard, logs, screenshots, and UI input.

Read [Architecture](/guide/architecture) for a deeper tour, or jump straight into [Quick Start](/guide/quick-start).

</div>
