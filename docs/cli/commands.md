# Commands

Replace `simdeck` with `./build/simdeck` when running from a source checkout.

## UI And Daemon

| Command                          | Purpose                                     |
| -------------------------------- | ------------------------------------------- |
| `simdeck`                        | Start a foreground browser session          |
| `simdeck <name-or-udid>`         | Start and select a device                   |
| `simdeck -d`                     | Start or reuse the detached project daemon  |
| `simdeck -k`                     | Stop the detached project daemon            |
| `simdeck -r`                     | Restart the detached project daemon         |
| `simdeck ui --open`              | Open the browser UI from a daemon           |
| `simdeck pair`                   | Show native iOS pairing code and QR         |
| `simdeck daemon status`          | Show daemon URL, PID, token, and log path   |
| `simdeck daemon stop`            | Stop the current project daemon             |
| `simdeck daemon killall`         | Stop all project daemons                    |
| `simdeck service on/off/restart` | Manage the optional always-on macOS service |

Examples:

```sh
simdeck ui --port 4320 --open
simdeck ui --open
simdeck pair
simdeck daemon restart --video-codec software --stream-quality low
```

`simdeck pair` uses the global LaunchAgent-backed service instead of a
project-local daemon. It binds the service for LAN access, preserves an existing
service token and pairing code when present, detects LAN and Tailscale IPv4
addresses, and prints a `simdeck://pair` QR for the native iOS app. The service
uses port 4310; workspace daemons start at 4311 and probe upward.

When the service is active, `simdeck` and `simdeck ui` print the existing
service endpoints instead of launching a project daemon. Use `simdeck daemon
start` or `simdeck daemon restart` when you explicitly want a workspace daemon.

`simdeck service restart` also preserves the installed service token so native
clients remain paired across service restarts. Use `simdeck service reset` to
rotate the token and pairing code, then restart the LaunchAgent.

## Device Lifecycle

```sh
simdeck list
simdeck list --format json
simdeck use <udid>
simdeck boot <udid>
simdeck shutdown
simdeck erase
```

Android emulators appear as IDs such as `android:Pixel_8_API_36`.
`list` defaults to compact JSON. Use `--format json` for the full simulator
inventory, including paths and display metadata.

`simdeck use <udid>` saves a default simulator for the current project
directory. After that, most device commands can omit `<udid>`; explicit UDIDs
still override the default.

## Apps And URLs

```sh
simdeck install /path/to/App.app
simdeck install /path/to/App.ipa
simdeck install android:<avd-name> /path/to/app.apk
simdeck uninstall com.example.App
simdeck launch com.example.App
simdeck open-url https://example.com
simdeck toggle-appearance
```

## Inspect UI

```sh
simdeck describe
simdeck describe --format agent --max-depth 4
simdeck describe --format agent --max-depth 4 --interactive
simdeck snapshot --format agent --max-depth 4 -i
simdeck describe --format compact-json
simdeck describe --source nativescript
simdeck describe --source react-native
simdeck describe --source flutter
simdeck describe --source uikit
simdeck describe --source native-ax
simdeck describe --point 120,240
simdeck wait-for --label "Welcome" --timeout-ms 5000
simdeck wait --label "Welcome" --timeout-ms 5000
simdeck assert --id login.button --source auto --max-depth 8
```

The default source is native accessibility for fast agent loops. Use `--source auto` when you want SimDeck to prefer a connected framework inspector, then the Swift in-app agent, then native accessibility. Use `--interactive` or `-i` to keep actionable elements and the ancestor context needed to find them. `snapshot` is an alias for `describe`. Agent-format output labels nodes with refs such as `@e3`, which can be passed back to `tap` or `press`. For quick agent loops, set the project default once and keep snapshots shallow.

## Performance

```sh
simdeck processes
simdeck stats
simdeck stats --pid 12345
simdeck stats --watch
simdeck sample
simdeck sample --pid 12345 --seconds 3
```

Performance data is simulator-only and uses host-process telemetry for matching app, extension, helper, and web-content PIDs. `stats` reports CPU, memory, disk write rate, network receive/send rates, connection count, hang state, and recent crash or termination signals. `sample` captures a short macOS `sample` report for the selected or foreground app process.

## Input

Coordinates are screen points unless `--normalized` is present. `tap "Continue"` is shorthand for a label tap on the selected device. `press` is an alias for `tap`, and refs from `describe --format agent` work as direct targets. Add `--expect-id`, `--expect-label`, or another `--expect-*` selector when the tap should wait for the next screen before returning. Use `--device <udid>` or `SIMDECK_DEVICE=<udid>` for one-off overrides.

```sh
simdeck tap 120 240
simdeck tap 0.5 0.5 --normalized
simdeck tap --label "Continue" --wait-timeout-ms 5000
simdeck tap --id com.apple.settings.screenTime --expect-id BackButton
simdeck tap "Continue"
simdeck press @e3
simdeck swipe 200 700 200 200
simdeck gesture scroll-down
simdeck pinch --start-distance 160 --end-distance 80
simdeck rotate-gesture --radius 100 --degrees 90
simdeck type "hello"
simdeck type --file message.txt
simdeck key enter
simdeck key-sequence --keycodes h,e,l,l,o
simdeck key-combo --modifiers cmd --key a
```

System controls:

```sh
simdeck button lock --duration-ms 1000
simdeck button volume-up
simdeck button action
simdeck button digital-crown
simdeck crown --delta 50
simdeck dismiss-keyboard
simdeck button software-keyboard
simdeck home
simdeck back
simdeck app-switcher
simdeck rotate-left
simdeck rotate-right
```

## Batch

```sh
simdeck batch \
  --step "tap --label Continue --wait-timeout-ms 5000 --expect-label Done" \
  --step "type 'hello world'" \
  --step "back"
```

Use `wait-for` or `assert` steps instead of fixed sleeps when possible.

## Maestro YAML

Run common Maestro flows through SimDeck's daemon-backed iOS Simulator API:

```sh
simdeck maestro test flow.yaml --artifacts-dir artifacts/maestro
```

The compatibility runner supports the core local commands: `launchApp`, `openLink`, `tapOn`, `inputText`, `eraseText`, `pressKey`, `assertVisible`, `assertNotVisible`, `scrollUntilVisible`, `swipe`, `takeScreenshot`, and `waitForAnimationToEnd`.

## Evidence

```sh
simdeck screenshot --output screen.png
simdeck screenshot --with-bezel --output screen-bezel.png
simdeck screenshot --stdout > screen.png
simdeck record --seconds 5 --output screen-recording.mp4
simdeck record --seconds 5 --stdout > screen-recording.mp4
simdeck pasteboard set "hello"
simdeck pasteboard get
simdeck logs --seconds 30 --limit 200
simdeck chrome-profile
```

Diagnostic iOS H.264 stream:

```sh
simdeck stream --frames 120 > stream.h264
```

## Studio And Providers

For hosted Studio workflows:

```sh
simdeck studio expose [simulator]
simdeck provider connect --studio-url <url> --host-id <id> --host-token <token>
simdeck provider run
simdeck provider status
```

These commands are mainly for managed remote simulator hosts.

## CoreSimulator Service

```sh
simdeck core-simulator restart
simdeck core-simulator start
simdeck core-simulator shutdown
```

Use this when Apple's simulator service is stale or unresponsive.
