# Commands

Replace `simdeck` with `./build/simdeck` when running from a source checkout.

## UI and service

| Command                        | Purpose                                     |
| ------------------------------ | ------------------------------------------- |
| `simdeck`                      | Start or reuse the service and print URLs   |
| `simdeck <name-or-udid>`       | Start and select a device                   |
| `simdeck --open`               | Open the browser UI                         |
| `simdeck -p 4311`              | Use a non-default service port              |
| `simdeck -a`                   | Register the service for login autostart    |
| `simdeck pair`                 | Show native iOS pairing code and QR         |
| `simdeck link [sim]`           | Print `https://app.simdeck.sh/open` link    |
| `simdeck service status`       | Show service URL, PID, token, and log path  |
| `simdeck service stop`         | Stop the current background service         |
| `simdeck service kill`         | Stop every SimDeck service process          |
| `simdeck service restart`      | Restart the current background service      |
| `simdeck service on/off/reset` | Manage the optional always-on macOS service |

Examples:

```sh
simdeck -p 4320 --open
simdeck --open
simdeck pair
simdeck service kill
simdeck service restart --video-codec software --stream-quality low
```

`simdeck` starts or reuses one local service. It uses port 4310 unless you pass
`-p` or `--port`. If that port belongs to a SimDeck service from another
binary, `simdeck` keeps it running and uses the next available port. Normal
commands reuse the matching warm service.

`simdeck pair` installs or refreshes the LaunchAgent-backed service. It binds
the service for LAN access, preserves an existing service token and pairing code
when present, detects LAN and Tailscale IPv4 addresses, and prints a
`simdeck://pair` QR for the native iOS app.

`simdeck service restart` also preserves the installed service token so native
clients remain paired across service restarts. Use `simdeck service reset` to
rotate the token and pairing code, then restart the LaunchAgent.

`simdeck link` emits a universal link that opens SimDeck Studio on iOS (or the
launchpad in browsers) pre-targeted at the resolved simulator. The simulator can
be a UDID, a name, or omitted to use the workspace selection from `simdeck use`.
Pass `--json` to also get the resolved name and the alternate addresses embedded
in the link.

## Device lifecycle

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

## Apps and URLs

```sh
simdeck install /path/to/App.app
simdeck install /path/to/App.ipa
simdeck install android:<avd-name> /path/to/app.apk
simdeck uninstall com.example.App
simdeck launch com.example.App
simdeck open-url https://example.com
simdeck toggle-appearance
```

## Camera Simulation

```sh
simdeck camera sources
simdeck camera start com.example.App --file /absolute/path/to/feed.mov --mirror off
simdeck camera start com.example.App --webcam
simdeck camera switch --placeholder
simdeck camera switch --file /absolute/path/to/frame.png
simdeck camera status
simdeck camera stop
```

`camera start` is iOS-simulator-only. It starts the daemon-owned camera feed,
relaunches the target bundle with the SimDeck camera injector, and makes
`AVCaptureDevice`, `AVCaptureVideoDataOutput`, `AVCapturePhotoOutput`, and
`AVCaptureVideoPreviewLayer` consume the simulated feed. `camera switch` changes
the running daemon source without relaunching the app. Media files must use
absolute paths; URL sources are treated as video streams.

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

Run common Maestro flows through SimDeck's service-backed iOS Simulator API:

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

## Studio and providers

For hosted Studio workflows:

```sh
simdeck studio expose [simulator]
simdeck provider connect --studio-url <url> --host-id <id> --host-token <token>
simdeck provider run
simdeck provider status
```

These commands are mainly for managed remote simulator hosts.

## CoreSimulator service

```sh
simdeck core-simulator restart
simdeck core-simulator start
simdeck core-simulator shutdown
```

Use this when Apple's simulator service is stale or unresponsive.
