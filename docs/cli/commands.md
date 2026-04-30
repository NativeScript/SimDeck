# Command Reference

Every public subcommand exposed by `simdeck`. Replace `simdeck` with `./build/simdeck` when running from a local checkout.

## UI And Daemon

### No Subcommand

Start a foreground workspace daemon and print clean local and LAN browser URLs:

```sh
simdeck
simdeck "iPhone 17 Pro Max"
simdeck 9750DF52-0471-48FF-B49A-B184C4BD3A3D
simdeck --detached
simdeck --kill
simdeck --restart
```

The single optional argument is a simulator name or UDID to select by default in the UI. The foreground daemon binds to all interfaces, advertises the detected LAN address when available, prints the LAN HTTP URL plus a six-digit LAN pairing code, and stops when the command exits, when you press `q`, or when you press Ctrl-C.

Shorthand lifecycle flags are available without a subcommand:

- `-d`, `--detached` starts or reuses the background project daemon, like `simdeck daemon start`.
- `-k`, `--kill` stops the background project daemon and returns the killed PID.
- `-r`, `--restart` stops the background project daemon, starts a fresh one, and returns the same JSON shape as `daemon start`.

### `ui`

Start or reuse the project daemon and serve the browser UI.

```sh
simdeck ui [--port 4310] [--bind 127.0.0.1] [--advertise-host <host>]
           [--client-root <path>] [--video-codec h264|h264-software]
           [--low-latency] [--open]
```

`--open` opens the authenticated local URL after the daemon is ready.

### `daemon start`

Start or reuse the project daemon without opening the browser:

```sh
simdeck daemon start [--port 4310] [--bind 127.0.0.1]
                     [--advertise-host <host>] [--client-root <path>]
                     [--video-codec h264|h264-software] [--low-latency]
```

Output:

```json
{
  "ok": true,
  "projectRoot": "/path/to/app",
  "pid": 12345,
  "pairingCode": "123456",
  "url": "http://127.0.0.1:4310",
  "started": true
}
```

### `daemon status`

Print daemon metadata for the current project:

```sh
simdeck daemon status
```

### `daemon restart`

Stop the daemon for the current project, then start a fresh one with the same
options as `daemon start`:

```sh
simdeck daemon restart [--port 4310] [--bind 127.0.0.1]
                       [--advertise-host <host>] [--client-root <path>]
                       [--video-codec h264|h264-software] [--low-latency]
```

### `daemon stop`

Stop the daemon for the current project:

```sh
simdeck daemon stop
```

### `daemon killall`

Stop SimDeck project daemons across all workspaces:

```sh
simdeck daemon killall
```

### `service`

Manage the optional always-on macOS user service. Use `simdeck daemon` for the
normal per-project process; use `simdeck service` when you want a LaunchAgent
that starts after login and stays available.

```sh
simdeck service on [--port 4310] [--bind 127.0.0.1]
                   [--advertise-host <host>] [--client-root <path>]
                   [--video-codec h264|h264-software] [--low-latency]
                   [--access-token <token>]
simdeck service restart [same options as service on]
simdeck service off
```

`service on` installs `~/Library/LaunchAgents/dev.nativescript.simdeck.plist`
and starts a LaunchAgent that serves SimDeck after login. It is intended for
agents and editor integrations that should be able to open the UI without first
starting a project daemon.

### `core-simulator`

Manage Apple's CoreSimulator service layer:

```sh
simdeck core-simulator restart
simdeck core-simulator start
simdeck core-simulator shutdown
```

Use this when `simctl` reports stale service state or simulator display attachment gets stuck before the first frame.

## Simulator Lifecycle

```sh
simdeck list
simdeck boot <udid>
simdeck shutdown <udid>
simdeck erase <udid>
```

`list` returns the same simulator inventory the browser UI renders. Lifecycle commands return JSON and use the native bridge, preferring private CoreSimulator paths when available and falling back to `xcrun simctl`.

## Apps And URLs

```sh
simdeck install <udid> /path/to/App.app
simdeck uninstall <udid> com.example.App
simdeck launch <udid> com.example.App
simdeck open-url <udid> https://example.com
simdeck toggle-appearance <udid>
```

`launch` and `open-url` use the warm daemon fast path when available.

## Inspect

```sh
simdeck describe <udid>
simdeck describe <udid> --format agent --max-depth 4
simdeck describe <udid> --format compact-json
simdeck describe <udid> --source nativescript
simdeck describe <udid> --source uikit
simdeck describe <udid> --source native-ax
simdeck describe <udid> --point 120,240
simdeck describe <udid> --direct
```

By default, `describe` uses the project daemon so it can prefer connected NativeScript or UIKit in-app inspectors, then fall back to the private CoreSimulator accessibility bridge. `--direct` skips the daemon and uses the native accessibility bridge directly.

Use `--format agent` for compact hierarchy text intended for agent planning, and `--format compact-json` when a script needs parseable lower-token output.

## Input

Coordinates are screen points unless `--normalized` is present.

```sh
simdeck tap <udid> 120 240
simdeck tap <udid> 0.5 0.5 --normalized
simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
simdeck touch <udid> 0.5 0.5 --phase began --normalized
simdeck touch <udid> 120 240 --down --up --delay-ms 800
simdeck swipe <udid> 200 700 200 200
simdeck gesture <udid> scroll-down
simdeck pinch <udid> --start-distance 160 --end-distance 80
simdeck rotate-gesture <udid> --radius 100 --degrees 90
simdeck key <udid> enter
simdeck key-sequence <udid> --keycodes h,e,l,l,o
simdeck key-combo <udid> --modifiers cmd --key a
simdeck type <udid> "hello"
simdeck type <udid> --file message.txt
simdeck button <udid> lock --duration-ms 1000
simdeck dismiss-keyboard <udid>
simdeck home <udid>
simdeck app-switcher <udid>
simdeck rotate-left <udid>
simdeck rotate-right <udid>
```

Use selectors (`--id`, `--label`, `--value`, `--element-type`) when possible. Use `--stdin` or `--file` for text containing quotes, newlines, or shell-sensitive characters.

## Batch

Run a known sequence through one command:

```sh
simdeck batch <udid> \
  --step "tap --label Continue --wait-timeout-ms 5000" \
  --step "type 'hello world'" \
  --step "gesture scroll-down"
```

Batch input can come from `--step`, `--file`, or `--stdin`. It fails fast by default; pass `--continue-on-error` for best-effort execution.

## Evidence

```sh
simdeck screenshot <udid> --output screen.png
simdeck screenshot <udid> --stdout > screen.png
simdeck pasteboard set <udid> "hello"
simdeck pasteboard get <udid>
simdeck logs <udid> --seconds 30 --limit 200
simdeck chrome-profile <udid>
```

`logs` fetches recent simulator logs. `chrome-profile` returns the CoreSimulator chrome layout used by the browser viewport.

## HTTP Fast Path

Supported hot controls use the daemon automatically. To target a specific daemon, set:

```sh
export SIMDECK_SERVER_URL=http://127.0.0.1:4310
```

This avoids repeated native setup in short-lived CLI processes. Commands that need local files, screenshots, pasteboard, or direct AX point queries still use the direct native path when appropriate.
