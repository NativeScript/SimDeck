---
name: simdeck
description: Agent guide for SimDeck, iOS Simulator control panel. Use for simulator lifecycle, app install/launch, live viewing, UI inspection, touch/keyboard automation, screenshots, logs, pasteboard, hardware controls, and repeatable simulator flows.
---

# SimDeck Agent Guide

SimDeck automates iOS Simulators. Use the CLI for automation and the browser UI for live human visibility. Works with UIKit, SwiftUI, React Native, Expo, and NativeScript apps.

## Start And View

`simdeck service` is likely to be running in background. Check if it is running using `simdeck service status`. If not, start a local server:

```bash
simdeck serve --port 4310
./scripts/build-cli.sh && ./build/simdeck serve --port 4310
simdeck serve --port 4310 --video-codec h264-software
simdeck serve --port 4310 --bind 0.0.0.0 --advertise-host 192.168.1.50
```

Viewer: `http://127.0.0.1:4310` or `http://127.0.0.1:4310?device=<UDID>`.

The viewer gets the API token automatically. Direct HTTP calls need `X-SimDeck-Token` or `Authorization: Bearer <token>`.

## Device And App

Device commands take `<UDID>` immediately after the command.

```bash
simdeck list
simdeck boot <UDID>
simdeck shutdown <UDID>
simdeck erase <UDID>
simdeck core-simulator restart
simdeck install <UDID> /path/to/App.app
simdeck launch <UDID> com.example.App
simdeck uninstall <UDID> com.example.App
simdeck open-url <UDID> myapp://route
simdeck open-url <UDID> https://example.com
simdeck toggle-appearance <UDID>
```

Build apps with project tooling. SimDeck controls the simulator.

## Inspect

Inspect before acting; always use `--format agent` for compact planning.

```bash
simdeck describe-ui <UDID>
simdeck describe-ui <UDID> --format agent --max-depth 4
simdeck describe-ui <UDID> --format compact-json
simdeck describe-ui <UDID> --point 120,240
simdeck describe-ui <UDID> --source auto
simdeck describe-ui <UDID> --source nativescript
simdeck describe-ui <UDID> --source uikit
simdeck describe-ui <UDID> --source native-ax
simdeck describe-ui <UDID> --direct
```

Use `--source auto` with `serve`. Use `--direct` or `--source native-ax` for the private CoreSimulator accessibility bridge. NativeScript inspector runtime can add richer hierarchy data.

Prefer selectors, coordinates only when needed. `describe-ui` coordinates are screen coordinates; add `--normalized` only for `0.0..1.0` inputs.

```bash
simdeck tap <UDID> --id LoginButton --wait-timeout-ms 5000
simdeck tap <UDID> --label "Continue" --element-type Button
simdeck tap <UDID> 120 240
```

## Interact

```bash
simdeck tap <UDID> 120 240
simdeck touch <UDID> 0.5 0.5 --phase began --normalized
simdeck touch <UDID> 0.5 0.5 --phase ended --normalized
simdeck touch <UDID> 120 240 --down --up --delay-ms 800
simdeck swipe <UDID> 200 700 200 200
simdeck swipe <UDID> 200 700 200 200 --duration-ms 500 --pre-delay-ms 100 --post-delay-ms 250
simdeck gesture <UDID> scroll-up
simdeck gesture <UDID> scroll-down
simdeck gesture <UDID> swipe-from-left-edge
simdeck gesture <UDID> swipe-from-right-edge
simdeck pinch <UDID> --start-distance 160 --end-distance 80
simdeck pinch <UDID> --start-distance 0.20 --end-distance 0.35 --normalized --duration-ms 250 --steps 8
simdeck rotate-gesture <UDID> --radius 100 --degrees 90
simdeck rotate-gesture <UDID> --radius 0.12 --degrees 45 --normalized --duration-ms 250 --steps 8
simdeck type <UDID> 'hello'
simdeck type <UDID> --stdin
simdeck type <UDID> --file message.txt
simdeck key <UDID> enter
simdeck key <UDID> 42 --duration-ms 500
simdeck key-sequence <UDID> --keycodes h,e,l,l,o --delay-ms 75
simdeck key-combo <UDID> --modifiers cmd,shift --key z
simdeck dismiss-keyboard <UDID>
simdeck button <UDID> home
simdeck button <UDID> lock --duration-ms 1000
simdeck button <UDID> side-button
simdeck button <UDID> siri
simdeck button <UDID> apple-pay
simdeck home <UDID>
simdeck app-switcher <UDID>
simdeck rotate-left <UDID>
simdeck rotate-right <UDID>
simdeck pasteboard set <UDID> 'text'
simdeck pasteboard get <UDID>
```

Use `--stdin` or `--file` for text with quotes, newlines, shell variables, or shell-sensitive characters.

## Timing And Batch

Input dispatch success does not prove the app reacted. Prefer selector waits, then verify with UI inspection, screenshot, logs, or viewer.

```bash
simdeck tap <UDID> --label "Continue" --wait-timeout-ms 5000
simdeck swipe <UDID> 200 700 200 200 --pre-delay-ms 100 --post-delay-ms 250
simdeck button <UDID> lock --duration-ms 1000
```

Use `batch` when steps are known; use discrete commands when a later step depends on parsing previous output.

```bash
simdeck batch <UDID> \
  --step "tap --label Continue --wait-timeout-ms 5000" \
  --step "type 'hello world'" \
  --step "gesture scroll-down" \
  --step "pinch --start-distance 0.20 --end-distance 0.35 --normalized"
```

Batch rules: one source (`--step`, `--file`, or `--stdin`); keep `<UDID>` at batch level; ordered steps; fail-fast by default; `--continue-on-error` for best effort. Step commands: `tap`, `swipe`, `gesture`, `pinch`, `rotate-gesture`, `touch`, `type`, `button`, `key`, `key-sequence`, `key-combo`, `sleep`.

## Evidence

```bash
simdeck screenshot <UDID> --output screen.png
simdeck screenshot <UDID> --stdout > screen.png
simdeck logs <UDID> --seconds 30 --limit 200
simdeck chrome-profile <UDID>
```

se screenshots for still evidence.

## Default Loop

1. Serve, list, boot/select `<UDID>`, optionally open viewer.
2. Build with project tools; install and launch with SimDeck.
3. `describe-ui --format agent --max-depth 4`.
4. Interact with selectors first; use coordinates only when needed.
5. Batch known flows; verify outcomes separately.

Final check: UDID explicit, selectors/coordinates inspected, timing intentional, complex text uses `--stdin`/`--file`, results verified, CLI/API/packet/inspector changes reflected here and in docs.
