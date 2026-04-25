---
layout: home

hero:
  name: SimDeck
  text: Local iOS Simulator control plane
  tagline: A Rust server, native bridge, and browser client that turn the iOS Simulator into a streamable, scriptable surface for everything from VS Code to NativeScript apps.
  actions:
    - theme: brand
      text: Get Started
      link: /guide/quick-start
    - theme: alt
      text: Why SimDeck?
      link: /guide/
    - theme: alt
      text: View on GitHub
      link: https://github.com/DjDeveloperr/xcode-canvas-web

features:
  - icon:
      src: /icons/monitor-smartphone.svg
      width: 28
      height: 28
    title: Browser-first simulator
    details: A React UI streams a live iOS Simulator over WebTransport with full touch, keyboard, hardware-button, and rotation input. No screen recording, no Xcode window required.
  - icon:
      src: /icons/zap.svg
      width: 28
      height: 28
    title: Native macOS performance
    details: A Rust HTTP server fronts an Objective-C bridge that talks to private CoreSimulator and SimulatorKit APIs for direct boot, headless display capture, and HEVC or H.264 encode.
  - icon:
      src: /icons/network.svg
      width: 28
      height: 28
    title: Stable HTTP control plane
    details: One server exposes simulator lifecycle, input, accessibility, logs, and inspector control through a single REST API on a predictable port.
  - icon:
      src: /icons/scan-search.svg
      width: 28
      height: 28
    title: First-class inspectors
    details: Choose between AXe accessibility snapshots, the Swift in-app inspector agent, or the NativeScript runtime inspector — SimDeck routes the right one for each request.
  - icon:
      src: /icons/puzzle.svg
      width: 28
      height: 28
    title: Built-in extensions
    details: A VS Code extension opens the simulator inside the editor, and a launchd service keeps the server running across logins.
  - icon:
      src: /icons/shield-check.svg
      width: 28
      height: 28
    title: Local-first by default
    details: Binds to 127.0.0.1, runs without a cloud account, and exposes a self-signed WebTransport endpoint that only your browser uses.
---

<div class="vp-doc" style="max-width: 1152px; margin: 4rem auto 0; padding: 0 24px;">

## What you can do with SimDeck

SimDeck packages a full simulator workflow into one cross-tool surface:

- **Stream a Simulator into a browser tab.** No more juggling Xcode windows or screen recordings.
- **Drive Simulators from JavaScript.** A REST API plus the NativeScript inspector turn any iOS app into a programmable target.
- **Embed a Simulator in your editor.** The bundled VS Code extension opens the same surface inside a panel.
- **Run Simulators on your LAN.** Bind to `0.0.0.0`, advertise a host, and connect from any other Mac, iPad, or laptop on the network.
- **Replace ad-hoc `simctl` scripts.** A single CLI handles `boot`, `shutdown`, `open-url`, `launch`, and a managed background service.

Read [Architecture](/guide/architecture) for a deeper tour, or jump straight into [Quick Start](/guide/quick-start).

</div>
