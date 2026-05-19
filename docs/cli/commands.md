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
addresses, and prints a `simdeck://pair` QR for the native iOS app. If the
requested service port is already in use by a workspace daemon, the LaunchAgent
uses the next available port after it.

`simdeck service restart` also preserves the installed service token so native
clients remain paired across service restarts. Use `simdeck service reset` to
rotate the token and pairing code, then restart the LaunchAgent.

## Device Lifecycle

```sh
simdeck list
simdeck list --format json
simdeck boot <udid>
simdeck shutdown <udid>
simdeck erase <udid>
```

Android emulators appear as IDs such as `android:Pixel_8_API_36`.
`list` defaults to compact JSON. Use `--format json` for the full simulator
inventory, including paths and display metadata.

## Apps And URLs

```sh
simdeck install <udid> /path/to/App.app
simdeck install <udid> /path/to/App.ipa
simdeck install android:<avd-name> /path/to/app.apk
simdeck uninstall <udid> com.example.App
simdeck launch <udid> com.example.App
simdeck open-url <udid> https://example.com
simdeck toggle-appearance <udid>
```

## Inspect UI

```sh
simdeck describe <udid>
simdeck describe <udid> --format agent --max-depth 4
simdeck describe <udid> --format compact-json
simdeck describe <udid> --source nativescript
simdeck describe <udid> --source react-native
simdeck describe <udid> --source flutter
simdeck describe <udid> --source uikit
simdeck describe <udid> --source native-ax
simdeck describe <udid> --point 120,240
simdeck wait-for <udid> --label "Welcome" --timeout-ms 5000
simdeck assert <udid> --id login.button --source auto --max-depth 8
```

Default source selection prefers a connected framework inspector, then the Swift in-app agent, then native accessibility.

## Performance

```sh
simdeck processes <udid>
simdeck stats <udid>
simdeck stats <udid> --pid 12345
simdeck stats <udid> --watch
simdeck sample <udid>
simdeck sample <udid> --pid 12345 --seconds 3
```

Performance data is simulator-only and uses host-process telemetry for matching app, extension, helper, and web-content PIDs. `stats` reports CPU, memory, disk write rate, network receive/send rates, connection count, hang state, and recent crash or termination signals. `sample` captures a short macOS `sample` report for the selected or foreground app process.

## Input

Coordinates are screen points unless `--normalized` is present.

```sh
simdeck tap <udid> 120 240
simdeck tap <udid> 0.5 0.5 --normalized
simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
simdeck swipe <udid> 200 700 200 200
simdeck gesture <udid> scroll-down
simdeck pinch <udid> --start-distance 160 --end-distance 80
simdeck rotate-gesture <udid> --radius 100 --degrees 90
simdeck type <udid> "hello"
simdeck type <udid> --file message.txt
simdeck key <udid> enter
simdeck key-sequence <udid> --keycodes h,e,l,l,o
simdeck key-combo <udid> --modifiers cmd --key a
```

System controls:

```sh
simdeck button <udid> lock --duration-ms 1000
simdeck button <udid> volume-up
simdeck button <udid> action
simdeck button <udid> digital-crown
simdeck crown <udid> --delta 50
simdeck dismiss-keyboard <udid>
simdeck home <udid>
simdeck app-switcher <udid>
simdeck rotate-left <udid>
simdeck rotate-right <udid>
```

## Batch

```sh
simdeck batch <udid> \
  --step "tap --label Continue --wait-timeout-ms 5000" \
  --step "type 'hello world'" \
  --step "wait-for --label 'hello world' --timeout-ms 5000"
```

Use `wait-for` or `assert` steps instead of fixed sleeps when possible.

## Evidence

```sh
simdeck screenshot <udid> --output screen.png
simdeck screenshot <udid> --with-bezel --output screen-bezel.png
simdeck screenshot <udid> --stdout > screen.png
simdeck record <udid> --seconds 5 --output screen-recording.mp4
simdeck record <udid> --seconds 5 --stdout > screen-recording.mp4
simdeck pasteboard set <udid> "hello"
simdeck pasteboard get <udid>
simdeck logs <udid> --seconds 30 --limit 200
simdeck chrome-profile <udid>
```

Diagnostic iOS H.264 stream:

```sh
simdeck stream <udid> --frames 120 > stream.h264
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
