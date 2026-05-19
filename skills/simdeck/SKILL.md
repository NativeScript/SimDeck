---
name: simdeck
description: Use for simulator lifecycle, app install/launch, live viewing, UI inspection, touch/keyboard automation, screenshots, recordings, logs, pasteboard, hardware controls, and repeatable simulator flows.
---

# SimDeck Agent Guide

SimDeck automates iOS Simulators and Android emulators. Use the CLI for automation and the browser UI for live human visibility. iOS works with NativeScript, UIKit, SwiftUI, React Native, Expo, and Flutter apps; Android works through ADB, emulator lifecycle, screenshots, logs, and UIAutomator hierarchy dumps.

SimDeck uses one daemon per workspace (CWD). Use `simdeck ui` and it will print JSON with `url` key. If it was already running, it prints the existing daemon URL and `started` is set to `false`. Contains workspace in `projectRoot` key. Example response:

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
simdeck ui
simdeck pair # prints LAN/Tailscale pairing URLs, code, and iOS QR
simdeck -k # kills the daemon
simdeck -r  # restarts the daemon
simdeck daemon killall # kills all daemons on the machine, use with care
```

Usually `http://127.0.0.1:4310` or `http://127.0.0.1:4310?device=<UDID>`.
Port may increment if multiple daemons are running.
Use `simdeck pair` when a native iOS client needs to pair. It starts or
refreshes the global LaunchAgent-backed service, detects LAN and Tailscale IPv4
addresses, and prints a QR with a `simdeck://pair` URL that carries the pairing
code plus alternate server addresses.
The LaunchAgent service token is stable across `simdeck pair`, `simdeck service
on`, and `simdeck service restart`; use `simdeck service reset` only when you
need to rotate the token and restart the service.
If a workspace daemon is already on 4310, the LaunchAgent service moves to the
next available service-discovery port, up to 4320, and leaves the workspace
daemon running.

Always first run `simdeck ui` to open the URL reported by the `simdeck ui` in the in-app browser using Browser Use tool if available.

If Browser Use is not available, only then use `simdeck ui --open` - it would open the default browser - taking focus away from the app.

## Device And App

Device commands take `<UDID>` immediately after the command.

```bash
simdeck list
simdeck list --format json
simdeck boot <UDID>
simdeck shutdown <UDID>
simdeck erase <UDID>
simdeck core-simulator restart
simdeck install <UDID> /path/to/App.app
simdeck install <UDID> /path/to/App.ipa
simdeck install android:<AVD_NAME> /path/to/app.apk
simdeck launch <UDID> com.example.App
simdeck uninstall <UDID> com.example.App
simdeck open-url <UDID> myapp://route
simdeck open-url <UDID> https://example.com
simdeck toggle-appearance <UDID>
```

`simdeck list` defaults to compact JSON for token-efficient agent selection.
Use `simdeck list --format json` only when you need full paths and display
metadata.

Build apps with project tooling.

Android devices use IDs like `android:Pixel_8_API_36`. `simdeck list` discovers
AVDs from the Android SDK.

## Fast Agent Inspection

Use targeted checks for test loops. `describe` is a diagnostic snapshot of the whole hierarchy. For verification, prefer the daemon APIs exposed by `simdeck/test`: `query`, `waitFor`, `assert`, selector `tap`, and `batch`.

```bash
simdeck describe <UDID>
simdeck describe <UDID> --format agent --max-depth 4
simdeck describe <UDID> --format compact-json
simdeck describe <UDID> --point 120,240
simdeck describe <UDID> --source auto
simdeck describe <UDID> --source nativescript
simdeck describe <UDID> --source react-native
simdeck describe <UDID> --source flutter
simdeck describe <UDID> --source uikit
simdeck describe <UDID> --source native-ax
simdeck describe <UDID> --source android-uiautomator
simdeck describe <UDID> --direct
simdeck wait-for <UDID> --label "Welcome" --timeout-ms 5000
simdeck assert <UDID> --id login.button --source auto --max-depth 8
```

Use `--source auto` with the project daemon. Use `--direct` or `--source native-ax` for the private CoreSimulator accessibility bridge. Use `--source android-uiautomator` for Android emulator UIAutomator hierarchies. NativeScript, React Native, and Flutter inspector runtimes can add richer hierarchy data.
For Android IDs, `describe` uses `uiautomator dump`; use `--format agent` or
`--format compact-json` the same way as iOS.

Prefer selectors, coordinates only when needed. Selector taps go through the daemon and wait for the element server-side.

```bash
simdeck tap <UDID> --id LoginButton --wait-timeout-ms 5000
simdeck tap <UDID> --label "Continue" --element-type Button
simdeck tap <UDID> 120 240
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
simdeck button <UDID> volume-up
simdeck button <UDID> volume-down
simdeck button <UDID> action --duration-ms 1000
simdeck button <UDID> mute
simdeck button <UDID> digital-crown
simdeck crown <UDID> --delta 50
simdeck button <UDID> left-side-button
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

## Timing, Batch

```bash
simdeck tap <UDID> --label "Continue" --wait-timeout-ms 5000
simdeck swipe <UDID> 200 700 200 200 --pre-delay-ms 100 --post-delay-ms 250
simdeck button <UDID> lock --duration-ms 1000
```

Prefer to use `wait-for` or `assert` in a batch to wait for UI state instead of fixed delays. `sleep 500` in a batch waits 500 ms. Use `sleep 0.5s` or `sleep --seconds 0.5` when you want to write seconds explicitly.

Use `batch` when steps are known; use discrete commands when a later step depends on parsing previous output.

```bash
simdeck batch <UDID> \
  --step "tap --label Continue --wait-timeout-ms 5000" \
  --step "type 'hello world'" \
  --step "gesture scroll-down" \
  --step "pinch --start-distance 0.20 --end-distance 0.35 --normalized"
```

Batch rules: one source (`--step`, `--file`, or `--stdin`); keep `<UDID>` at batch level; ordered steps; fail-fast by default; `--continue-on-error` for best effort. Step commands: `tap`, `wait-for`, `assert`, `swipe`, `gesture`, `pinch`, `rotate-gesture`, `touch`, `type`, `button`, `key`, `key-sequence`, `key-combo`, `sleep`.

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
simdeck screenshot <UDID> --with-bezel --output screen-bezel.png
simdeck screenshot <UDID> --stdout > screen.png
simdeck record <UDID> --seconds 5 --output screen-recording.mp4
simdeck record <UDID> --seconds 5 --stdout > screen-recording.mp4
simdeck logs <UDID> --seconds 30 --limit 200
simdeck chrome-profile <UDID>
simdeck processes <UDID>
simdeck stats <UDID>
simdeck stats <UDID> --watch
simdeck sample <UDID> --seconds 3
```

Use screenshots for still evidence, `--with-bezel` when the device frame matters, and `record` for short MP4 screen recordings. Use `stats` for simulator app CPU, memory, disk write, network receive/send rates, connections, hang, and crash/termination signals. Use `sample` only when a short CPU stack capture is worth the extra pause. Prefer describe for token-efficient state dumps, if they have enough context.

## Default Loop

1. Start UI, list, boot/select `<UDID>`, open viewer if in-app browser available
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
daemon ports. Use the manual `startSimDeckReactNativeInspector(...)` API
when you need custom host/path/security options.
