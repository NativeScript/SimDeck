# Background Service

SimDeck can install itself as a per-user `launchd` service so the server starts automatically at login and survives terminal closures, crashes, and reboots.

The service runs under your user account, not as `root`. It can therefore reach private CoreSimulator APIs and the iOS Simulator just like an interactive shell session would.

## Enable

Turn the service on with the same flags you would pass to `serve`:

```sh
simdeck service on --port 4310
```

You can pass any of:

| Flag               | Default               | Notes                                                              |
| ------------------ | --------------------- | ------------------------------------------------------------------ |
| `--port <u16>`     | `4310`                | HTTP port. WebTransport listens on `port + 1`.                     |
| `--bind <ip>`      | `127.0.0.1`           | Bind address (use `0.0.0.0` for [LAN access](/guide/lan-access)).  |
| `--advertise-host` | matches `--bind`      | Hostname advertised to remote clients.                             |
| `--client-root`    | bundled `client/dist` | Override the static client directory.                              |
| `--video-codec`    | `hevc`                | One of `hevc`, `h264`, `h264-software`. See [Video](/guide/video). |

The command:

1. Resolves the absolute path to the current `simdeck` binary.
2. Writes a launchd plist to `~/Library/LaunchAgents/dev.nativescript.simdeck.plist`.
3. Bootstraps the service into `gui/<uid>` and immediately kickstarts it.
4. Prints the resulting paths as JSON, including the stdout and stderr log paths under `~/Library/Logs/`.

After the command finishes you can close the terminal — the service keeps running.

## Inspect

The plist lives at:

```text
~/Library/LaunchAgents/dev.nativescript.simdeck.plist
```

Logs land at:

```text
~/Library/Logs/simdeck.log
~/Library/Logs/simdeck.err.log
```

You can verify the service is up with `launchctl`:

```sh
launchctl print "gui/$(id -u)/dev.nativescript.simdeck"
```

Or just by hitting the health endpoint:

```sh
curl http://127.0.0.1:4310/api/health
```

## Disable

Stop and remove the service:

```sh
simdeck service off
```

This:

1. Calls `launchctl bootout` for the service in your `gui/<uid>` domain.
2. Removes the plist from `~/Library/LaunchAgents`.
3. Prints the removed plist path as JSON.

Logs under `~/Library/Logs` are kept so you can review past output.

## Updating the service

When you install a new version of SimDeck, the launchd service still points at the old binary path until you re-bootstrap it:

```sh
simdeck service off
simdeck service on --port 4310
```

Re-run with the same flags you used originally. The plist is regenerated against the new binary.

## What's in the plist

The generated plist mirrors the CLI invocation you used:

```xml
<key>Label</key>
<string>dev.nativescript.simdeck</string>
<key>ProgramArguments</key>
<array>
  <string>/usr/local/bin/simdeck</string>
  <string>serve</string>
  <string>--port</string>
  <string>4310</string>
  <string>--bind</string>
  <string>127.0.0.1</string>
  <string>--client-root</string>
  <string>/usr/local/lib/node_modules/simdeck/client/dist</string>
  <string>--video-codec</string>
  <string>hevc</string>
</array>
<key>RunAtLoad</key>
<true/>
<key>KeepAlive</key>
<true/>
<key>StandardOutPath</key>
<string>/Users/you/Library/Logs/simdeck.log</string>
<key>StandardErrorPath</key>
<string>/Users/you/Library/Logs/simdeck.err.log</string>
```

You can hand-edit the plist if needed, but the easier path is to re-run `service on` with new flags.
