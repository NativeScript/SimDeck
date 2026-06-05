---
name: simdeck
description: Use for simulator lifecycle, app install/launch, live viewing, UI inspection, touch/keyboard automation, screenshots, recordings, logs, pasteboard, hardware controls, and repeatable simulator flows.
---

# SimDeck Agent Guide

SimDeck automates iOS Simulators and Android emulators. Use the CLI for automation and the browser UI for live human visibility. iOS works with NativeScript, UIKit, SwiftUI, React Native, Expo, and Flutter apps; Android live viewing uses the emulator `-share-vid` shared display surface plus SimDeck's native host H.264 encoder, with lifecycle, screenshots, logs, and UIAutomator hierarchy dumps also handled through ADB.

SimDeck uses one long-running local service. Run `simdeck` first; it starts or
reuses the background service and prints the browser URL plus pairing code.
Pass `--open` only when you want SimDeck to open the default browser itself.
Pass `-p <port>` or `--port <port>` when the service should use a non-default
port. Pass `-a` or `--autostart` when the service should also be registered as
a macOS LaunchAgent.

Example response from `simdeck`:

```json
{
  "ok": true,
  "pairingCode": "401974",
  "pid": 91285,
  "projectRoot": "-",
  "started": true,
  "url": "http://127.0.0.1:4310"
}
```

```bash
simdeck
simdeck --open
simdeck -p 4311
simdeck -a
simdeck pair # prints LAN/Tailscale pairing URLs, code, and iOS QR
simdeck service stop
simdeck service kill # stops all SimDeck services from any binary
simdeck service restart
```

Usually `http://127.0.0.1:4310`. Pass a simulator name or UDID to `simdeck`
when the browser URL should open with `?device=<UDID>`. Use `simdeck -p 4311`
when the default port is not the right fit.
Use `simdeck pair` when a native iOS client needs to pair. It starts or
refreshes the LaunchAgent-backed service, detects LAN and Tailscale IPv4
addresses, and prints a QR with a `simdeck://pair` URL. Normal service restarts
preserve the token and pairing code; use `simdeck service reset` only when you
need to rotate them.
Use `simdeck service kill` or `simdeck service killall` when you need a clean
slate across installed and source-checkout binaries.

Optionally, when a user might want to drive the current simulator from their
phone, run `simdeck link [sim]` to print an `https://app.simdeck.sh/open`
universal link that opens SimDeck Studio on iOS (or the launchpad in a browser)
pre-targeted at the resolved simulator.

Always first run `simdeck` and open the reported URL in the in-app browser using the Browser tool if available.

If Browser Use is not available, only then use `simdeck --open`; it opens the default browser and may take focus away from the app.

## Device and app

Start by choosing a project default device. `simdeck use <UDID>` stores the
selection for the current workspace/CWD so later commands can omit the UDID.
Explicit UDIDs, `--device`, `SIMDECK_DEVICE`, and `SIMDECK_UDID` still work for
one-off overrides. Prefer short forms in agent loops, such as
`simdeck tap "Continue"` and `simdeck snapshot --format agent --max-depth 2 -i`.

```bash
simdeck list
simdeck list --format json
simdeck use <UDID>
simdeck boot <UDID>
simdeck shutdown
simdeck erase
simdeck core-simulator restart
simdeck install /path/to/App.app
simdeck install /path/to/App.ipa
simdeck install android:<AVD_NAME> /path/to/app.apk
simdeck launch com.example.App
simdeck uninstall com.example.App
simdeck open-url myapp://route
simdeck open-url https://example.com
simdeck toggle-appearance
simdeck camera sources
simdeck camera start com.example.App --file /absolute/path/to/camera.mov
simdeck camera start com.example.App --webcam
simdeck camera switch --placeholder
simdeck camera status
simdeck camera stop
```

`simdeck list` defaults to compact JSON for token-efficient agent selection.
Use `simdeck list --format json` only when you need full paths and display
metadata.

Build apps with project tooling.

Android devices use IDs like `android:Pixel_8_API_36`. `simdeck list` discovers
AVDs from the Android SDK.
SimDeck-owned Android boots use the emulator shared-video surface plus
`--android-gpu host` by default. Use `simdeck service restart --android-gpu auto`
or `--android-gpu swiftshader_indirect` only when host GPU rendering is unstable.

## Fast agent inspection

Use targeted checks for test loops. `describe` is a diagnostic snapshot of the whole hierarchy. For verification, prefer the service APIs exposed by `simdeck/test`: `action`, `query`, `waitFor`, `assert`, selector `tap`, and `batch`.

```bash
simdeck describe
simdeck describe --format agent --max-depth 4
simdeck describe --format agent --max-depth 4 --interactive
simdeck snapshot --format agent --max-depth 4 -i
simdeck describe --format compact-json
simdeck describe --point 120,240
simdeck describe --source auto
simdeck describe --source nativescript|react-native|flutter|swiftui|uikit
simdeck describe --source native-ax
simdeck describe --source android-uiautomator
simdeck describe --direct
simdeck wait-for --label "Welcome" --timeout-ms 5000
simdeck wait --label "Welcome" --timeout-ms 5000
simdeck assert --id login.button --source auto --max-depth 8
```

The default source is `native-ax`, which is the fastest and most universal path for agents. Use `--source auto` when you want richer NativeScript, React Native, Flutter, SwiftUI, or UIKit inspector data before native accessibility fallback. Use `--direct` or `--source native-ax` for the private CoreSimulator accessibility bridge. Use `--source android-uiautomator` for Android emulator UIAutomator hierarchies.
For Android IDs, `describe` uses `uiautomator dump`; use `--format agent` or
`--format compact-json` the same way as iOS.
Use `--interactive` or `-i` when an agent only needs controls and actionable framework nodes; SimDeck keeps ancestor context so the output is still navigable. Agent output labels nodes with refs such as `@e3`; reuse them with `simdeck press @e3`. `snapshot`, `press`, and `wait` are aliases for `describe`, `tap`, and `wait-for`.

Prefer selectors, coordinates only when needed. Selector taps go through the service and wait for the element server-side. Use `--expect-id`, `--expect-label`, or another `--expect-*` selector when the tap should also wait for the next screen before returning.

```bash
simdeck tap --id LoginButton --wait-timeout-ms 5000
simdeck tap --id com.apple.settings.screenTime --expect-id BackButton
simdeck tap --label "Continue" --element-type Button
simdeck tap 120 240
simdeck tap "Continue"
simdeck press @e3
```

For persistent app integration tests, use `simdeck/test` instead of shelling out repeatedly:

```ts
import { connect } from "simdeck/test";

const udid = "your-device-udid";
const simdeck = await connect({ udid });

try {
  await simdeck.launch("com.example.App");
  await simdeck.waitFor({ id: "login.button" }, { maxDepth: 8 });
  await simdeck.tap(0.5, 0.5);
  await simdeck.assert({ label: "Welcome" }, { maxDepth: 8 });
  const matches = await simdeck.query({ id: "account.name" });
  console.log(matches);
} finally {
  simdeck.close();
}
```

`simdeck/test` state-query helpers default to `source: "native-ax"` for fast
agent control. Pass `source: "auto"` only when you intentionally want richer
framework inspector trees before native accessibility fallback.

Use `tree()`/`describe` only when a test needs to print the whole UI for debugging. In a normal agent loop, do not fetch the full tree after every action; verify the specific element or text that proves the step succeeded.

## Interact

```bash
simdeck tap 120 240
simdeck touch 0.5 0.5 --phase began --normalized
simdeck touch 0.5 0.5 --phase ended --normalized
simdeck touch 120 240 --down --up --delay-ms 800
simdeck swipe 200 700 200 200
simdeck swipe 200 700 200 200 --duration-ms 500 --pre-delay-ms 100 --post-delay-ms 250
simdeck gesture scroll-up
simdeck gesture scroll-down
simdeck gesture swipe-from-left-edge
simdeck gesture swipe-from-right-edge
simdeck pinch --start-distance 160 --end-distance 80
simdeck pinch --start-distance 0.20 --end-distance 0.35 --normalized --duration-ms 250 --steps 8
simdeck rotate-gesture --radius 100 --degrees 90
simdeck rotate-gesture --radius 0.12 --degrees 45 --normalized --duration-ms 250 --steps 8
simdeck type 'hello'
simdeck type --stdin
simdeck type --file message.txt
simdeck key enter
simdeck key 42 --duration-ms 500
simdeck key-sequence --keycodes h,e,l,l,o --delay-ms 75
simdeck key-combo --modifiers cmd,shift --key z
simdeck dismiss-keyboard
simdeck button software-keyboard
simdeck button home
simdeck back
simdeck button lock --duration-ms 1000
simdeck button side-button
simdeck button volume-up
simdeck button volume-down
simdeck button action --duration-ms 1000
simdeck button mute
simdeck button digital-crown
simdeck crown --delta 50
simdeck button left-side-button
simdeck button siri
simdeck button apple-pay
simdeck home
simdeck app-switcher
simdeck rotate-left
simdeck rotate-right
simdeck pasteboard set 'text'
simdeck pasteboard get
```

Use `--stdin` or `--file` for text with quotes, newlines, shell variables, or shell-sensitive characters.

## Timing and batch

```bash
simdeck tap --label "Continue" --wait-timeout-ms 5000
simdeck tap --label "Continue" --expect-label "Done"
simdeck back
simdeck swipe 200 700 200 200 --pre-delay-ms 100 --post-delay-ms 250
simdeck button lock --duration-ms 1000
```

Prefer to use `wait-for` or `assert` in a batch to wait for UI state instead of fixed delays. `sleep 500` in a batch waits 500 ms. Use `sleep 0.5s` or `sleep --seconds 0.5` when you want to write seconds explicitly.

Use `batch` when steps are known; use discrete commands when a later step depends on parsing previous output.

```bash
simdeck batch \
  --step "tap --label Continue --wait-timeout-ms 5000 --expect-label Done" \
  --step "type 'hello world'" \
  --step "back" \
  --step "pinch --start-distance 0.20 --end-distance 0.35 --normalized"
```

Batch rules: one source (`--step`, `--file`, or `--stdin`); set the default with `simdeck use <UDID>` or keep `<UDID>` at batch level; ordered steps; fail-fast by default; `--continue-on-error` for best effort. Step commands: `tap`, `back`, `wait-for`, `assert`, `swipe`, `gesture`, `pinch`, `rotate-gesture`, `touch`, `type`, `button`, `key`, `key-sequence`, `key-combo`, `sleep`.

For JS tests, batch can combine action and verification without extra CLI process startup:

```ts
await simdeck.batch([
  { action: "tap", selector: { label: "Continue" }, waitTimeoutMs: 5000 },
  {
    action: "waitFor",
    selector: { label: "Continue Tapped" },
    timeoutMs: 5000,
  },
  { action: "assert", selector: { id: "fixture.status" } },
]);
```

For app-style flows, SimDeck can run a practical subset of Maestro YAML:

```bash
simdeck maestro test flow.yaml --artifacts-dir artifacts/maestro
```

## Evidence

```bash
simdeck screenshot --output screen.png
simdeck screenshot --with-bezel --output screen-bezel.png
simdeck screenshot --stdout > screen.png
simdeck record --seconds 5 --output screen-recording.mp4
simdeck record --seconds 5 --stdout > screen-recording.mp4
simdeck logs --seconds 30 --limit 200
simdeck chrome-profile
simdeck processes
simdeck stats
simdeck stats --watch
simdeck sample --seconds 3
```

Use screenshots for still evidence, `--with-bezel` when the device frame matters, and `record` for short MP4 screen recordings. Use `stats` for simulator app CPU, memory, disk write, network receive/send rates, connections, hang, and crash/termination signals. Use `sample` only when a short CPU stack capture is worth the extra pause. Prefer describe for token-efficient state dumps, if they have enough context.

## Camera Simulation

Use `camera start` when a booted iOS simulator app needs a deterministic camera
feed. The CLI talks to the SimDeck daemon, which owns the camera source and
shared-memory writer, then injects the camera shim into the app with
`simctl launch` and relaunches the bundle. Use `camera switch` to swap between
placeholder, media, and webcam sources without relaunching while the daemon feed
is alive.

```bash
simdeck camera sources
simdeck camera start com.example.App --file /absolute/path/to/feed.mov --mirror off
simdeck camera switch --placeholder
simdeck camera start com.example.App --webcam "FaceTime HD Camera"
simdeck camera stop
```

Media paths must be absolute. URLs are treated as video streams. Webcam support
depends on macOS camera availability and permission for SimDeck.

## Default loop

1. Start UI, list, `simdeck use <UDID>`, boot/select the device, open viewer if in-app browser available
2. Build with project tools; install and launch with SimDeck.
3. Use one `describe --format agent --max-depth 4` to understand an unfamiliar screen.
4. Interact with selectors first; use coordinates only when needed.
5. Verify with `waitFor`/`assert`/`query`, not repeated full `describe` dumps.
6. Batch known flows; keep `describe` as a failure/debug artifact.

### Optional inspector plugins

For a richer hierarchy, if user wants to opt-in

### NativeScript Inspector

NativeScript apps can connect directly to the running server from JS and expose
their view hierarchy plus raw UIKit backing views

```ts
import { startSimDeckInspector } from "@nativescript/simdeck-inspector";

if (__DEV__) {
  startSimDeckInspector({ port: 4310 });
}
```

The runtime connects to `GET /api/inspector/connect` as a WebSocket

### React Native Inspector

React Native apps can expose their component tree and Metro dev-mode source
locations with the inspector package:

```ts
import "react-native-simdeck/auto";
import "expo-router/entry";
```

Import it before `expo-router/entry` or `AppRegistry.registerComponent(...)`.
The auto entrypoint no-ops outside development, reads
`EXPO_PUBLIC_SIMDECK_PORT` when present, and otherwise scans common SimDeck
service ports. Use the manual `startSimDeckReactNativeInspector(...)` API
when you need custom host/path/security options.
