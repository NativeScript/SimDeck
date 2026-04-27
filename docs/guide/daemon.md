# Project Daemon

SimDeck runs one warm native host per project. The daemon owns the HTTP API, the WebTransport video endpoint, inspector routing, and lazy native simulator sessions.

Normal CLI commands start the daemon automatically when they need it. Use `simdeck daemon` only when you want to manage it explicitly.

## Start

```sh
simdeck daemon start
```

The command starts the daemon for the current project root and prints JSON:

```json
{
  "ok": true,
  "projectRoot": "/path/to/app",
  "pid": 12345,
  "url": "http://127.0.0.1:4310",
  "started": true
}
```

If a healthy daemon is already running for that project, `started` is `false` and the same daemon is reused.

## Open The UI

For day-to-day use, `ui` is the shortest path:

```sh
simdeck ui --open
```

This starts or reuses the project daemon, serves the bundled browser client, and opens the authenticated local URL.

## Options

`daemon start` and `ui` accept the same server options:

| Flag               | Default               | Notes                                                              |
| ------------------ | --------------------- | ------------------------------------------------------------------ |
| `--port <u16>`     | `4310`                | HTTP port. WebTransport listens on `port + 1`.                     |
| `--bind <ip>`      | `127.0.0.1`           | Bind address. Use `0.0.0.0` for [LAN access](/guide/lan-access).   |
| `--advertise-host` | matches local host    | Hostname or IP advertised to browser clients.                      |
| `--client-root`    | bundled `client/dist` | Override the static browser client directory.                      |
| `--video-codec`    | `hevc`                | One of `hevc`, `h264`, `h264-software`. See [Video](/guide/video). |
| `--open`           | `false`               | `ui` only. Open the browser after the daemon is ready.             |

Example:

```sh
simdeck ui --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
```

## Status

```sh
simdeck daemon status
```

The status output includes the daemon URL, PID, project root, and access token. Local same-origin browser use does not require copying the token; direct remote API callers should send it as `X-SimDeck-Token` or `Authorization: Bearer <token>`.

## Stop

```sh
simdeck daemon stop
```

This terminates the daemon for the current project and removes its metadata file from the system temp directory. The next CLI command that needs the daemon starts a fresh one.

## CoreSimulator Service Layer

The project daemon is different from Apple's CoreSimulator service. If `simctl` reports stale service state or the live display never produces a first frame, restart Apple's service layer:

```sh
simdeck core-simulator restart
```

You can also manage it explicitly:

```sh
simdeck core-simulator start
simdeck core-simulator shutdown
```
