---
layout: home

hero:
  name: SimDeck
  text: Simulator control panel
  tagline: Stream simulator from your favorite IDE, with built-in tools to enhance DX for developing mobile apps
  actions:
    - theme: brand
      text: Get Started
      link: /guide/quick-start
    - theme: alt
      text: Why SimDeck?
      link: /guide/
    - theme: alt
      text: View on GitHub
      link: https://github.com/NativeScript/SimDeck

features:
  - icon:
      src: /icons/monitor-smartphone.svg
      width: 28
      height: 28
    title: Browser-first simulator
    details: "`simdeck` starts a foreground project daemon and prints local/LAN URLs for a client live WebRTC H.264 video & low-latency input."
  - icon:
      src: /icons/zap.svg
      width: 28
      height: 28
    title: Native macOS performance
    details: "A Rust HTTP server fronts an Objective-C bridge that talks to CoreSimulator, SimulatorKit, private display APIs."
  - icon:
      src: /icons/network.svg
      width: 28
      height: 28
    title: Streaming remote
    details: "WebRTC/WebSocket transports allow remote streaming/tunneling the simulator stream efficiently. Share your simulator streams with coworkers, or use in CI."
  - icon:
      src: /icons/scan-search.svg
      width: 28
      height: 28
    title: First-class inspectors
    details: "`describe` and the UI prefer NativeScript, React Native, UIKit, SwiftUI in-app inspectors when available, fall back to accessibility tree."
  - icon:
      src: /icons/puzzle.svg
      width: 28
      height: 28
    title: Built-in extension
    details: "VS Code extension opens the simulator inside the editor, and `simdeck/test` gives JS/TS tests a fast API for app automation."
  - icon:
      src: /icons/shield-check.svg
      width: 28
      height: 28
    title: Remote streaming
    details: "Local first but simulator streams can be shared using tools like Cloudflare Tunnel."
---

<div class="vp-doc" style="max-width: 1152px; margin: 4rem auto 0; padding: 0 24px;">

## What you can do with SimDeck

SimDeck packages a full simulator workflow into one cross-tool surface:

- **Stream a Simulator into a browser tab.** Run `simdeck` and open one of the printed URLs, or use `simdeck ui --open` for a reusable background daemon.
- **Debug view hierarchy.** Integrates with NativeScript, React Native, UIKit, SwiftUI and Flutter and allows debugging views/layout.
- **Drive Simulators from JavaScript.** `simdeck/test` can launch apps, tap, wait for accessibility state, batch steps, and capture screenshots.
- **Embed a Simulator in your editor.** The bundled VS Code extension opens the same surface inside a panel.
- **Replace ad-hoc `simctl` scripts.** A single CLI handles `boot`, `shutdown`, app install/launch, URL opening, pasteboard, logs, screenshots, and UI input.

Read [Architecture](/guide/architecture) for a deeper tour, or jump straight into [Quick Start](/guide/quick-start).

</div>
