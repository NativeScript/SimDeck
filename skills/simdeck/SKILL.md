# SimDeck

Use this skill when developing, debugging, or testing an iOS app in a simulator
from an agent workflow. This includes UIKit, SwiftUI, React Native, Expo, and
NativeScript apps.

SimDeck gives agents a simple CLI for full simulator control and a web
UI the user can watch inside their IDE. Use it to boot/select a simulator,
install and launch the app, inspect the current UI hierarchy as JSON, drive
touch/keyboard/gesture input, capture screenshots, and run repeatable end-to-end
flows without depending on AXe.

## Start The Viewer

Start the local server before interactive testing:

```sh
simdeck serve --port 4310
```

If `simdeck` is not on `PATH` and you are inside this repository, build
and use the local binary:

```sh
./scripts/build-cli.sh
./build/simdeck serve --port 4310
```

If the IDE has an in-app browser capability, open the web UI so the user can see
the app being built and tested live:

```text
http://127.0.0.1:4310
```

To open the viewer focused on one simulator, include the simulator UDID:

```text
http://127.0.0.1:4310?device=<udid>
```

The web UI is for human visibility and manual checks. The CLI is the primary
agent control surface.

## Pick A Simulator

List simulators and choose a booted device, or boot the one you need:

```sh
simdeck list
simdeck boot <udid>
simdeck shutdown <udid>
```

If CoreSimulator is wedged or a display stream never produces frames, restart
the service layer:

```sh
simdeck core-simulator restart
```

## Install And Run Apps

Build the app using the project’s normal tooling, then install and launch the
resulting `.app` bundle:

```sh
simdeck install <udid> /path/to/App.app
simdeck launch <udid> com.example.App
```

Useful app-management commands:

```sh
simdeck uninstall <udid> com.example.App
simdeck erase <udid>
simdeck open-url <udid> myapp://route
simdeck open-url <udid> https://example.com
simdeck toggle-appearance <udid>
```

For NativeScript apps, the CLI can always read native accessibility state. When
the app also includes the NativeScript inspector runtime, the server can expose
NativeScript/UIKit hierarchy details through the inspector source as well.

## Inspect UI State

Use hierarchy inspection before acting whenever possible. It returns JSON with
labels, values, roles, identifiers, frames, and children:

```sh
simdeck describe-ui <udid>
simdeck describe-ui <udid> --point 120,240
```

Prefer selector-based commands when stable labels or identifiers exist:

```sh
simdeck tap <udid> --id LoginButton --wait-timeout-ms 5000
simdeck tap <udid> --label "Continue" --element-type Button
```

Coordinates from `describe-ui` are screen coordinates. Add `--normalized` when
passing `0.0..1.0` coordinates directly.

## Touch And Gestures

Basic touch:

```sh
simdeck tap <udid> 120 240
simdeck touch <udid> 0.5 0.5 --phase began --normalized
simdeck touch <udid> 0.5 0.5 --phase ended --normalized
simdeck touch <udid> 120 240 --down --up --delay-ms 800
```

Swipe and gesture presets:

```sh
simdeck swipe <udid> 200 700 200 200
simdeck swipe <udid> 200 700 200 200 --duration-ms 500 --pre-delay-ms 100 --post-delay-ms 250
simdeck gesture <udid> scroll-up
simdeck gesture <udid> scroll-down
simdeck gesture <udid> swipe-from-left-edge
simdeck gesture <udid> swipe-from-right-edge
```

True two-touch gestures:

```sh
simdeck pinch <udid> --start-distance 160 --end-distance 80
simdeck pinch <udid> --start-distance 0.20 --end-distance 0.35 --normalized --duration-ms 250 --steps 8
simdeck rotate-gesture <udid> --radius 100 --degrees 90
simdeck rotate-gesture <udid> --radius 0.12 --degrees 45 --normalized --duration-ms 250 --steps 8
```

## Keyboard And Text

Send text, keys, sequences, and modifier combos:

```sh
simdeck type <udid> "hello"
simdeck type <udid> --stdin
simdeck type <udid> --file message.txt
simdeck key <udid> enter
simdeck key <udid> 42 --duration-ms 500
simdeck key-sequence <udid> --keycodes h,e,l,l,o --delay-ms 75
simdeck key-combo <udid> --modifiers cmd,shift --key z
simdeck dismiss-keyboard <udid>
```

## Hardware And System Controls

Everything the web UI exposes should also be available from the CLI:

```sh
simdeck button <udid> home
simdeck button <udid> lock --duration-ms 1000
simdeck button <udid> side-button
simdeck button <udid> siri
simdeck button <udid> apple-pay
simdeck home <udid>
simdeck app-switcher <udid>
simdeck rotate-left <udid>
simdeck rotate-right <udid>
simdeck toggle-appearance <udid>
```

Pasteboard:

```sh
simdeck pasteboard set <udid> "text"
simdeck pasteboard get <udid>
```

## Screenshots, Logs, And Metadata

Use screenshots for visual evidence and logs for diagnostics:

```sh
simdeck screenshot <udid> --output screen.png
simdeck screenshot <udid> --stdout > screen.png
simdeck logs <udid> --seconds 30 --limit 200
simdeck chrome-profile <udid>
```

The CLI intentionally omits AXe-style record-video, MJPEG streaming, raw JPEG
streaming, and BGRA piping. Use `screenshot` for still PNG capture and the web
UI’s live stream for visual monitoring.

## Batch Agent Flows

Use `batch` for repeatable end-to-end tests. Batch accepts exactly one source:
repeated `--step`, `--file`, or `--stdin`.

Step lines support `tap`, `swipe`, `gesture`, `pinch`, `rotate-gesture`,
`touch`, `type`, `button`, `key`, `key-sequence`, `key-combo`, and `sleep`.

```sh
simdeck batch <udid> \
  --step "tap --label Continue --wait-timeout-ms 5000" \
  --step "type 'hello world'" \
  --step "gesture scroll-down" \
  --step "pinch --start-distance 0.20 --end-distance 0.35 --normalized"
```

Use `--continue-on-error` when collecting multiple failures in one pass is more
useful than stopping at the first failed step.

## Recommended Agent Loop

1. Start `simdeck serve --port 4310`.
2. Open `http://127.0.0.1:4310?device=<udid>` in the IDE in-app browser when
   available.
3. Build the user’s app with its normal project commands.
4. Install and launch the `.app` with `simdeck install` and `launch`.
5. Run `describe-ui` and choose selectors or coordinates.
6. Drive the app with `tap`, `type`, `gesture`, `pinch`, `rotate-gesture`, and
   `batch`.
7. Capture screenshots and logs when verifying behavior or debugging failures.

## Notes

- `describe-ui` uses the built-in private CoreSimulator accessibility bridge,
  not AXe.
- Keep app-specific build steps in the app project. SimDeck controls
  the simulator and viewer; it does not replace Xcode, `xcodebuild`,
  NativeScript CLI, Expo CLI, or other app build tools.
- If CLI flags change, update this skill so agents can continue to use the
  simulator without guessing.
