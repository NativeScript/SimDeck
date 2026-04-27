# Command Reference

Every subcommand exposed by `simdeck`. All of them assume the binary is on your `PATH` after `npm install -g simdeck`. Replace `simdeck` with `./build/simdeck` to run from a local checkout.

## `serve`

Start the HTTP and WebTransport servers in the foreground. This is the only command that holds the terminal open.

```sh
simdeck serve [--port <u16>] [--bind <ip>] [--advertise-host <host>]
                       [--client-root <path>] [--video-codec <codec>]
                       [--access-token <token>]
```

| Flag               | Default               | Description                                                                            |
| ------------------ | --------------------- | -------------------------------------------------------------------------------------- |
| `--port`           | `4310`                | HTTP port. WebTransport listens on `port + 1`.                                         |
| `--bind`           | `127.0.0.1`           | Bind address (`0.0.0.0` for [LAN access](/guide/lan-access), `::` for IPv6).           |
| `--advertise-host` | matches `--bind`      | Hostname or IP advertised to remote clients in the WebTransport URL template and cert. |
| `--client-root`    | bundled `client/dist` | Override the static client directory.                                                  |
| `--video-codec`    | `hevc`                | One of `hevc`, `h264`, `h264-software`. See [Video Pipeline](/guide/video).            |
| `--access-token`   | generated at startup  | Token accepted by `X-SimDeck-Token`, `Authorization: Bearer`, or the served UI cookie. |

When the server is up it prints something like:

```text
HTTP listening on http://127.0.0.1:4310
WebTransport listening on https://127.0.0.1:4311/wt/simulators/{udid}
Serving client from /usr/local/lib/node_modules/simdeck/client/dist
API access token: 9f...
```

`Ctrl-C` shuts both servers down cleanly.

## `service on`

Install SimDeck as a per-user `launchd` service. Same flags as `serve`:

```sh
simdeck service on [--port <u16>] [--bind <ip>] [--advertise-host <host>]
                            [--client-root <path>] [--video-codec <codec>]
                            [--access-token <token>]
```

The command writes `~/Library/LaunchAgents/dev.nativescript.simdeck.plist`, bootstraps it into `gui/<uid>`, and immediately kickstarts it. See [Background Service](/guide/service) for details.

Output (JSON):

```json
{
  "ok": true,
  "service": "dev.nativescript.simdeck",
  "plist": "/Users/you/Library/LaunchAgents/dev.nativescript.simdeck.plist",
  "stdoutLog": "/Users/you/Library/Logs/simdeck.log",
  "stderrLog": "/Users/you/Library/Logs/simdeck.err.log"
}
```

## `service off`

Remove the launchd service:

```sh
simdeck service off
```

Output (JSON):

```json
{
  "ok": true,
  "service": "dev.nativescript.simdeck",
  "plist": "/Users/you/Library/LaunchAgents/dev.nativescript.simdeck.plist"
}
```

The plist is removed, but the log files under `~/Library/Logs` are kept.

## `list`

Print every simulator the native bridge can see, as JSON:

```sh
simdeck list
```

```json
{
  "simulators": [
    {
      "udid": "9D7E5BB7-...",
      "name": "iPhone 15 Pro",
      "runtimeName": "iOS 18.0",
      "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-15-Pro",
      "isBooted": true
    }
  ]
}
```

This is roughly equivalent to `xcrun simctl list devices --json`, but the output is filtered down to the fields SimDeck exposes through `GET /api/simulators`.

## `describe-ui`

Print the current UI hierarchy. By default the command tries the running local
SimDeck service first so it can use NativeScript or UIKit in-app inspectors, then
falls back to the private native accessibility bridge.

```sh
simdeck describe-ui <udid> [--format json|compact-json|agent]
                         [--source auto|nativescript|uikit|native-ax]
                         [--max-depth <n>] [--include-hidden] [--direct]
                         [--point <x>,<y>] [--server-url <url>]
```

Use `--format agent` for compact hierarchy text intended for LLM planning, and
`--format compact-json` when a script needs parseable lower-token output.
`--point` returns the native element at a screen point and uses the native point
query directly.

## Warm service fast path

Most agent loops should keep `simdeck serve` or `simdeck service on` running and
set:

```sh
export SIMDECK_SERVER_URL=http://127.0.0.1:4310
```

Supported hot controls then use the local HTTP service and avoid repeated native
setup in short-lived CLI processes. This fast path covers `launch`, `open-url`,
normalized `touch`, normalized coordinate `tap`, normalized `swipe`, normalized
`gesture`, `key`, `key-sequence`, `key-combo`, `dismiss-keyboard`, `home`,
`app-switcher`, `button`, `rotate-left`, `rotate-right`, and
`toggle-appearance`. Commands that need local files, selector-to-point
resolution, screenshots, pasteboard, or batch execution stay on the direct
native path.

## `boot`

Boot a simulator by UDID:

```sh
simdeck boot <udid>
```

```json
{ "ok": true, "udid": "9D7E5BB7-...", "action": "boot" }
```

The native bridge prefers a private `CoreSimulator` direct boot when available and falls back to `xcrun simctl boot` otherwise.

## `shutdown`

Shut a simulator down by UDID:

```sh
simdeck shutdown <udid>
```

```json
{ "ok": true, "udid": "9D7E5BB7-...", "action": "shutdown" }
```

If the server has a live session attached for the UDID, the registry tears it down before issuing the shutdown.

## `open-url`

Open a URL inside the simulator. This goes through `xcrun simctl openurl`, which routes `https://` and `http://` to MobileSafari and any other scheme to whichever app handles it:

```sh
simdeck open-url <udid> https://example.com
```

```json
{ "ok": true, "udid": "9D7E5BB7-...", "url": "https://example.com" }
```

## `launch`

Launch an installed app by its bundle identifier. This goes through `xcrun simctl launch`:

```sh
simdeck launch <udid> com.apple.Preferences
```

```json
{ "ok": true, "udid": "9D7E5BB7-...", "bundleId": "com.apple.Preferences" }
```

If the simulator is shut down the command fails with a clear error. Boot it first with `simdeck boot <udid>`.
