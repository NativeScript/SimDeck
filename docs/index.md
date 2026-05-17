---
layout: home

hero:
  name: SimDeck
  text: Simulator control in your browser
  tagline: Stream, inspect, and automate iOS Simulators and Android emulators from one local tool.
  actions:
    - theme: brand
      text: Quick Start
      link: /guide/quick-start
    - theme: alt
      text: Install
      link: /guide/installation
    - theme: alt
      text: CLI Reference
      link: /cli/

features:
  - icon:
      src: /icons/monitor-smartphone.svg
      width: 28
      height: 28
    title: Browser simulator view
    details: Run `simdeck`, open the printed URL, and control the device from a local or LAN browser.
  - icon:
      src: /icons/zap.svg
      width: 28
      height: 28
    title: Fast local control
    details: Boot devices, install apps, open URLs, type, tap, swipe, rotate, capture screenshots and recordings, and read logs from the CLI.
  - icon:
      src: /icons/scan-search.svg
      width: 28
      height: 28
    title: Useful inspection
    details: Use accessibility, NativeScript, React Native, Flutter, Swift, UIKit, SwiftUI, WebKit, and DevTools views when available.
  - icon:
      src: /icons/puzzle.svg
      width: 28
      height: 28
    title: Tests and editor workflows
    details: Use `simdeck/test` for JS/TS automation and the VS Code extension to keep a simulator panel inside the editor.
  - icon:
      src: /icons/network.svg
      width: 28
      height: 28
    title: Local first, shareable
    details: Bind to localhost for daily use, or expose a paired LAN session when another device or teammate needs access.
  - icon:
      src: /icons/shield-check.svg
      width: 28
      height: 28
    title: Built for agents
    details: Compact `describe` output, selector-based input, batch commands, and clear JSON errors keep automation predictable.
---

<div class="vp-doc" style="max-width: 1152px; margin: 4rem auto 0; padding: 0 24px;">

## Start here

```sh
npx simdeck
```

That starts a foreground SimDeck server for the current workspace and prints browser URLs. Open the local URL to view and control the simulator. Press `q` or Ctrl-C to stop it.

Common next steps:

- [Install the CLI](/guide/installation) for repeated use.
- [Open a simulator in the browser](/guide/quick-start).
- [Drive a simulator from the CLI](/cli/commands).
- [Troubleshoot startup, stream, or inspector issues](/guide/troubleshooting).

</div>
