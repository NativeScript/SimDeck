# Xcode Canvas Web

Use this skill when developing, debugging, or testing an iOS app in a simulator
from an agent workflow. This includes UIKit, SwiftUI, React Native, Expo, and
NativeScript apps.

Xcode Canvas Web gives agents a simple CLI for full simulator control and a web
UI the user can watch inside their IDE. Use it to boot/select a simulator,
install and launch the app, inspect the current UI hierarchy as JSON, drive
touch/keyboard/gesture input, capture screenshots, and run repeatable end-to-end
flows without depending on AXe.

## Start The Viewer

Start the local server before interactive testing:

```sh
xcode-canvas-web serve --port 4310
```

If `xcode-canvas-web` is not on `PATH` and you are inside this repository, build
and use the local binary:

```sh
./scripts/build-cli.sh
./build/xcode-canvas-web serve --port 4310
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
xcode-canvas-web list
xcode-canvas-web boot <udid>
xcode-canvas-web shutdown <udid>
```

If CoreSimulator is wedged or a display stream never produces frames, restart
the service layer:

```sh
xcode-canvas-web core-simulator restart
```

## Install And Run Apps

Build the app using the project’s normal tooling, then install and launch the
resulting `.app` bundle:

```sh
xcode-canvas-web install <udid> /path/to/App.app
xcode-canvas-web launch <udid> com.example.App
```

Useful app-management commands:

```sh
xcode-canvas-web uninstall <udid> com.example.App
xcode-canvas-web erase <udid>
xcode-canvas-web open-url <udid> myapp://route
xcode-canvas-web open-url <udid> https://example.com
xcode-canvas-web toggle-appearance <udid>
```

For NativeScript apps, the CLI can always read native accessibility state. When
the app also includes the NativeScript inspector runtime, the server can expose
NativeScript/UIKit hierarchy details through the inspector source as well.

## Inspect UI State

Use hierarchy inspection before acting whenever possible. It returns JSON with
labels, values, roles, identifiers, frames, and children:

```sh
xcode-canvas-web describe-ui <udid>
xcode-canvas-web describe-ui <udid> --point 120,240
```

Prefer selector-based commands when stable labels or identifiers exist:

```sh
xcode-canvas-web tap <udid> --id LoginButton --wait-timeout-ms 5000
xcode-canvas-web tap <udid> --label "Continue" --element-type Button
```

Coordinates from `describe-ui` are screen coordinates. Add `--normalized` when
passing `0.0..1.0` coordinates directly.

## Touch And Gestures

Basic touch:

```sh
xcode-canvas-web tap <udid> 120 240
xcode-canvas-web touch <udid> 0.5 0.5 --phase began --normalized
xcode-canvas-web touch <udid> 0.5 0.5 --phase ended --normalized
xcode-canvas-web touch <udid> 120 240 --down --up --delay-ms 800
```

Swipe and gesture presets:

```sh
xcode-canvas-web swipe <udid> 200 700 200 200
xcode-canvas-web swipe <udid> 200 700 200 200 --duration-ms 500 --pre-delay-ms 100 --post-delay-ms 250
xcode-canvas-web gesture <udid> scroll-up
xcode-canvas-web gesture <udid> scroll-down
xcode-canvas-web gesture <udid> swipe-from-left-edge
xcode-canvas-web gesture <udid> swipe-from-right-edge
```

True two-touch gestures:

```sh
xcode-canvas-web pinch <udid> --start-distance 160 --end-distance 80
xcode-canvas-web pinch <udid> --start-distance 0.20 --end-distance 0.35 --normalized --duration-ms 250 --steps 8
xcode-canvas-web rotate-gesture <udid> --radius 100 --degrees 90
xcode-canvas-web rotate-gesture <udid> --radius 0.12 --degrees 45 --normalized --duration-ms 250 --steps 8
```

## Keyboard And Text

Send text, keys, sequences, and modifier combos:

```sh
xcode-canvas-web type <udid> "hello"
xcode-canvas-web type <udid> --stdin
xcode-canvas-web type <udid> --file message.txt
xcode-canvas-web key <udid> enter
xcode-canvas-web key <udid> 42 --duration-ms 500
xcode-canvas-web key-sequence <udid> --keycodes h,e,l,l,o --delay-ms 75
xcode-canvas-web key-combo <udid> --modifiers cmd,shift --key z
xcode-canvas-web dismiss-keyboard <udid>
```

## Hardware And System Controls

Everything the web UI exposes should also be available from the CLI:

```sh
xcode-canvas-web button <udid> home
xcode-canvas-web button <udid> lock --duration-ms 1000
xcode-canvas-web button <udid> side-button
xcode-canvas-web button <udid> siri
xcode-canvas-web button <udid> apple-pay
xcode-canvas-web home <udid>
xcode-canvas-web app-switcher <udid>
xcode-canvas-web rotate-left <udid>
xcode-canvas-web rotate-right <udid>
xcode-canvas-web toggle-appearance <udid>
```

Pasteboard:

```sh
xcode-canvas-web pasteboard set <udid> "text"
xcode-canvas-web pasteboard get <udid>
```

## Screenshots, Logs, And Metadata

Use screenshots for visual evidence and logs for diagnostics:

```sh
xcode-canvas-web screenshot <udid> --output screen.png
xcode-canvas-web screenshot <udid> --stdout > screen.png
xcode-canvas-web logs <udid> --seconds 30 --limit 200
xcode-canvas-web chrome-profile <udid>
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
xcode-canvas-web batch <udid> \
  --step "tap --label Continue --wait-timeout-ms 5000" \
  --step "type 'hello world'" \
  --step "gesture scroll-down" \
  --step "pinch --start-distance 0.20 --end-distance 0.35 --normalized"
```

Use `--continue-on-error` when collecting multiple failures in one pass is more
useful than stopping at the first failed step.

## Recommended Agent Loop

1. Start `xcode-canvas-web serve --port 4310`.
2. Open `http://127.0.0.1:4310?device=<udid>` in the IDE in-app browser when
   available.
3. Build the user’s app with its normal project commands.
4. Install and launch the `.app` with `xcode-canvas-web install` and `launch`.
5. Run `describe-ui` and choose selectors or coordinates.
6. Drive the app with `tap`, `type`, `gesture`, `pinch`, `rotate-gesture`, and
   `batch`.
7. Capture screenshots and logs when verifying behavior or debugging failures.

## Notes

- `describe-ui` uses the built-in private CoreSimulator accessibility bridge,
  not AXe.
- Keep app-specific build steps in the app project. Xcode Canvas Web controls
  the simulator and viewer; it does not replace Xcode, `xcodebuild`,
  NativeScript CLI, Expo CLI, or other app build tools.
- If CLI flags change, update this skill so agents can continue to use the
  simulator without guessing.
