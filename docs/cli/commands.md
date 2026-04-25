# Command Reference

Every subcommand exposed by `xcode-canvas-web`. All of them assume the binary is on your `PATH` after `npm install -g xcode-canvas-web`. Replace `xcode-canvas-web` with `./build/xcode-canvas-web` to run from a local checkout.

## `serve`

Start the HTTP and WebTransport servers in the foreground. This is the only command that holds the terminal open.

```sh
xcode-canvas-web serve [--port <u16>] [--bind <ip>] [--advertise-host <host>]
                       [--client-root <path>] [--video-codec <codec>]
```

| Flag               | Default               | Description                                                                            |
| ------------------ | --------------------- | -------------------------------------------------------------------------------------- |
| `--port`           | `4310`                | HTTP port. WebTransport listens on `port + 1`.                                         |
| `--bind`           | `127.0.0.1`           | Bind address (`0.0.0.0` for [LAN access](/guide/lan-access), `::` for IPv6).           |
| `--advertise-host` | matches `--bind`      | Hostname or IP advertised to remote clients in the WebTransport URL template and cert. |
| `--client-root`    | bundled `client/dist` | Override the static client directory.                                                  |
| `--video-codec`    | `hevc`                | One of `hevc`, `h264`, `h264-software`. See [Video Pipeline](/guide/video).            |

When the server is up it prints something like:

```text
HTTP listening on http://127.0.0.1:4310
WebTransport listening on https://127.0.0.1:4311/wt/simulators/{udid}
Serving client from /usr/local/lib/node_modules/xcode-canvas-web/client/dist
```

`Ctrl-C` shuts both servers down cleanly.

## `service on`

Install Simdeck as a per-user `launchd` service. Same flags as `serve`:

```sh
xcode-canvas-web service on [--port <u16>] [--bind <ip>] [--advertise-host <host>]
                            [--client-root <path>] [--video-codec <codec>]
```

The command writes `~/Library/LaunchAgents/dev.nativescript.xcode-canvas-web.plist`, bootstraps it into `gui/<uid>`, and immediately kickstarts it. See [Background Service](/guide/service) for details.

Output (JSON):

```json
{
  "ok": true,
  "service": "dev.nativescript.xcode-canvas-web",
  "plist": "/Users/you/Library/LaunchAgents/dev.nativescript.xcode-canvas-web.plist",
  "stdoutLog": "/Users/you/Library/Logs/xcode-canvas-web.log",
  "stderrLog": "/Users/you/Library/Logs/xcode-canvas-web.err.log"
}
```

## `service off`

Remove the launchd service:

```sh
xcode-canvas-web service off
```

Output (JSON):

```json
{
  "ok": true,
  "service": "dev.nativescript.xcode-canvas-web",
  "plist": "/Users/you/Library/LaunchAgents/dev.nativescript.xcode-canvas-web.plist"
}
```

The plist is removed, but the log files under `~/Library/Logs` are kept.

## `list`

Print every simulator the native bridge can see, as JSON:

```sh
xcode-canvas-web list
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

This is roughly equivalent to `xcrun simctl list devices --json`, but the output is filtered down to the fields Simdeck exposes through `GET /api/simulators`.

## `boot`

Boot a simulator by UDID:

```sh
xcode-canvas-web boot <udid>
```

```json
{ "ok": true, "udid": "9D7E5BB7-...", "action": "boot" }
```

The native bridge prefers a private `CoreSimulator` direct boot when available and falls back to `xcrun simctl boot` otherwise.

## `shutdown`

Shut a simulator down by UDID:

```sh
xcode-canvas-web shutdown <udid>
```

```json
{ "ok": true, "udid": "9D7E5BB7-...", "action": "shutdown" }
```

If the server has a live session attached for the UDID, the registry tears it down before issuing the shutdown.

## `open-url`

Open a URL inside the simulator. This goes through `xcrun simctl openurl`, which routes `https://` and `http://` to MobileSafari and any other scheme to whichever app handles it:

```sh
xcode-canvas-web open-url <udid> https://example.com
```

```json
{ "ok": true, "udid": "9D7E5BB7-...", "url": "https://example.com" }
```

## `launch`

Launch an installed app by its bundle identifier. This goes through `xcrun simctl launch`:

```sh
xcode-canvas-web launch <udid> com.apple.Preferences
```

```json
{ "ok": true, "udid": "9D7E5BB7-...", "bundleId": "com.apple.Preferences" }
```

If the simulator is shut down the command fails with a clear error. Boot it first with `xcode-canvas-web boot <udid>`.
