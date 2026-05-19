# Daemon

SimDeck runs a local server for the current project. The server owns the browser UI, REST API, live stream, inspector connections, and warm device sessions.

Most commands start or reuse the project daemon automatically. Manage it directly only when you need a specific lifecycle.

## Foreground

```sh
simdeck
```

Use this for normal interactive work. It prints browser URLs and stops when you press `q` or Ctrl-C.

## Detached

```sh
simdeck -d
simdeck daemon start
```

Both start or reuse a background daemon for the current project. Detached mode is useful for tests, editor integrations, and scripts.

```sh
simdeck daemon status
simdeck daemon stop
simdeck daemon restart
simdeck daemon killall
```

`daemon killall` stops SimDeck project daemons from every workspace.

## Open The Browser UI

```sh
simdeck ui --open
```

This starts or reuses the daemon, then opens the authenticated local URL.

## Common Server Options

`simdeck ui`, `daemon start`, `daemon restart`, and `service restart` use the same core options:

| Flag                         | Default        | Use it when                                |
| ---------------------------- | -------------- | ------------------------------------------ | ------ | ---------------------------------- |
| `--port <port>`              | `4310`         | The default port is busy                   |
| `--bind <ip>`                | `127.0.0.1`    | You need LAN access with `0.0.0.0` or `::` |
| `--advertise-host <host>`    | detected       | Remote browsers need a specific host or IP |
| `--video-codec auto          | hardware       | software`                                  | `auto` | You need to force encoder behavior |
| `--stream-quality <profile>` | `full`         | You want lower CPU or bandwidth use        |
| `--local-stream-fps <fps>`   | `60`           | You want a different local stream target   |
| `--client-root <path>`       | bundled client | You are serving a custom static client     |

Example:

```sh
simdeck daemon start --port 4320 --video-codec software --stream-quality low
```

## Always-On Service

Use the macOS user service when SimDeck should be reachable after login without starting a project daemon first:

```sh
simdeck service on
simdeck service restart --port 4310
simdeck service reset
simdeck service off
```

When the requested service port is occupied by a workspace daemon, the
LaunchAgent automatically moves to the next available service-discovery port,
up to 4320. Workspace daemons are left running.

`service on`, `service restart`, and `simdeck pair` preserve the installed
service token and pairing code. Use `service reset` when you explicitly want to
rotate those credentials and restart the LaunchAgent.

Prefer the project daemon for normal repository work. Use the service for long-lived agent or editor setups.

## Restart CoreSimulator

If `simctl` hangs, reports a stale service version, or devices never attach:

```sh
simdeck core-simulator restart
```

Then retry:

```sh
simdeck list
simdeck boot <udid>
```
