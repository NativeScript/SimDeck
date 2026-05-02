---
name: simdeck
description: Agent guide for SimDeck, iOS Simulator control panel. Use for simulator lifecycle, app install/launch, live viewing, UI inspection, touch/keyboard automation, screenshots, logs, pasteboard, hardware controls, and repeatable simulator flows.
---

# SimDeck Agent Guide

SimDeck automates iOS Simulators. Use the CLI for automation and the browser UI for live human visibility. Works with UIKit, SwiftUI, React Native, Expo, and NativeScript apps.

## Start And View

SimDeck uses one warm daemon per project. Check it with `simdeck daemon status`; start it or open the browser UI when needed:

```bash
simdeck
simdeck "iPhone 17 Pro Max"
simdeck -d
simdeck -k
simdeck -r
simdeck daemon start
simdeck daemon restart
simdeck daemon killall
simdeck ui --open
npm run build:cli && ./build/simdeck ui --open
simdeck daemon start --video-codec software
simdeck daemon start --video-codec software --low-latency
simdeck ui --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
```

`simdeck` without a subcommand starts a foreground workspace daemon, prints local and LAN HTTP URLs, prints a six-digit pairing code for LAN browsers, and stops on `q` or Ctrl-C. The optional single argument is a simulator name or UDID to select by default. Use `-d` for detached start, `-k` to kill the background daemon, and `-r` to restart it.

Viewer: `http://127.0.0.1:4310` or `http://127.0.0.1:4310?device=<UDID>`.
The browser uses WebRTC H.264 video for both hardware and software encoders.
Add `--low-latency` on less capable runners to cap software H.264 at 15 fps,
drop stale pending frames more aggressively, and cap the longest edge at 1170 px
before latency piles up.
For local high-refresh testing, pass `--local-stream-fps <15-120>` on `ui`,
`daemon start`, or `daemon restart`. The default stays 60 fps; 120 fps is the
maximum local cap.
For remote browsers where Safari stalls but Chrome works, run the daemon with a
TURN server and relay-only ICE:
`SIMDECK_WEBRTC_ICE_SERVERS=turns:turn.example.com:5349?transport=tcp`,
`SIMDECK_WEBRTC_ICE_USERNAME`, `SIMDECK_WEBRTC_ICE_CREDENTIAL`, and
`SIMDECK_WEBRTC_ICE_TRANSPORT_POLICY=relay`.
SimDeck Studio provider runners keep SimDeck bound to loopback and run
`scripts/studio-provider-bridge.mjs` as an outbound bridge; Studio hosts the UI
and proxies REST requests through that bridge while WebRTC media negotiates
directly with the runner.
For an ad-hoc local provider that can be opened from another browser or phone,
run `simdeck studio expose "iPhone 17 Pro"` and keep that process running. It
prints the unique Studio simulator URL and active stream settings. This defaults
to software H.264 with realtime stream settings so remote viewers drop stale
frames instead of building latency. Studio providers default to the `smooth`
stream quality profile (1170 px, dynamic up to 60 fps, higher bitrate to reduce
artifacts); override with
`--stream-quality quality|balanced|smooth|economy|ci-software`, or pass
`--video-codec hardware` when a dedicated hardware encoder is preferable.

The local viewer gets the API token automatically. LAN browsers pair with the printed code before receiving the API cookie. Direct HTTP calls need `X-SimDeck-Token` or `Authorization: Bearer <token>`.

For fastest agent loops against a known daemon, export:

```bash
export SIMDECK_SERVER_URL=http://127.0.0.1:4310
```

Hot controls then delegate through the selected daemon instead of cold-starting native control each time. This is supported for launch/open-url, normalized touch/tap/swipe/gesture, key/key-sequence/key-combo, hardware buttons, dismiss-keyboard, home/app-switcher, rotate, and appearance toggles. Use direct commands when you need screen-coordinate selector resolution, install/uninstall, screenshots, pasteboard, or batch.

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

## Fast Agent Inspection

Use targeted checks for test loops. `describe` is a diagnostic snapshot of the whole hierarchy; it is useful for planning, but it is expensive. For verification, prefer the daemon APIs exposed by `simdeck/test`: `query`, `waitFor`, `assert`, selector `tap`, and `batch`.

```bash
simdeck describe <UDID>
simdeck describe <UDID> --format agent --max-depth 4
simdeck describe <UDID> --format compact-json
simdeck describe <UDID> --point 120,240
simdeck describe <UDID> --source auto
simdeck describe <UDID> --source nativescript
simdeck describe <UDID> --source uikit
simdeck describe <UDID> --source native-ax
simdeck describe <UDID> --direct
```

Use `--source auto` with the project daemon. Use `--direct` or `--source native-ax` for the private CoreSimulator accessibility bridge. NativeScript inspector runtime can add richer hierarchy data.

Prefer selectors, coordinates only when needed. Selector taps go through the daemon and wait for the element server-side.

```bash
simdeck tap <UDID> --id LoginButton --wait-timeout-ms 5000
simdeck tap <UDID> --label "Continue" --element-type Button
simdeck tap <UDID> 120 240
```

For persistent app integration tests, use `simdeck/test` instead of shelling out repeatedly:

```ts
import { connect } from "simdeck/test";

const simdeck = await connect();
try {
  await simdeck.launch(udid, "com.example.App");
  await simdeck.waitFor(udid, { id: "login.button" }, { maxDepth: 8 });
  await simdeck.tap(udid, 0.5, 0.5);
  await simdeck.assert(udid, { label: "Welcome" }, { maxDepth: 8 });
  const matches = await simdeck.query(udid, { id: "account.name" });
  console.log(matches);
} finally {
  simdeck.close();
}
```

Use `tree()`/`describe` only when a test needs to print the whole UI for debugging. In a normal agent loop, do not fetch the full tree after every action; verify the specific element or text that proves the step succeeded.

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

Input dispatch success does not prove the app reacted. Prefer selector waits/asserts, then use screenshot/logs/viewer when visual evidence matters.

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

For JS tests, batch can combine action and verification without extra CLI process startup:

```ts
await simdeck.batch(udid, [
  { action: "tap", selector: { label: "Continue" }, waitTimeoutMs: 5000 },
  {
    action: "waitFor",
    selector: { label: "Continue Tapped" },
    timeoutMs: 5000,
  },
  { action: "assert", selector: { id: "fixture.status" } },
]);
```

## Evidence

```bash
simdeck screenshot <UDID> --output screen.png
simdeck screenshot <UDID> --stdout > screen.png
simdeck stream <UDID> --frames 120 > stream.h264
simdeck logs <UDID> --seconds 30 --limit 200
simdeck chrome-profile <UDID>
```

Use screenshots for still evidence. Use `stream` when a diagnostic needs raw
H.264 samples for an external player or capture pipeline.

## Default Loop

1. Serve, list, boot/select `<UDID>`, optionally open viewer.
2. Build with project tools; install and launch with SimDeck.
3. Use one `describe --format agent --max-depth 4` to understand an unfamiliar screen.
4. Interact with selectors first; use coordinates only when needed.
5. Verify with `waitFor`/`assert`/`query`, not repeated full `describe` dumps.
6. Batch known flows; keep `describe` as a failure/debug artifact.

Final check: UDID explicit, daemon URL set for fast loops when targeting a specific daemon, selectors/coordinates inspected, timing intentional, complex text uses `--stdin`/`--file`, results verified, CLI/API/inspector changes reflected here and in docs.
