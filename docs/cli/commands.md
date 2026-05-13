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
| `simdeck daemon status`          | Show daemon URL, PID, token, and log path   |
| `simdeck daemon stop`            | Stop the current project daemon             |
| `simdeck daemon killall`         | Stop all project daemons                    |
| `simdeck service on/off/restart` | Manage the optional always-on macOS service |

Examples:

```sh
simdeck ui --port 4320 --open
simdeck ui --open
simdeck daemon restart --video-codec software --stream-quality low
```

## Device Lifecycle

```sh
simdeck list
simdeck boot <udid>
simdeck shutdown <udid>
simdeck erase <udid>
```

Android emulators appear as IDs such as `android:Pixel_8_API_36`.

## Apps And URLs

```sh
simdeck install <udid> /path/to/App.app
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
```

Default source selection prefers a connected framework inspector, then the Swift in-app agent, then native accessibility.

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
simdeck screenshot <udid> --stdout > screen.png
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
