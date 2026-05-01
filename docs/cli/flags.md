# Flags & Options

A consolidated list of the public SimDeck CLI flags, grouped by command.

::: tip Help output
Pass `--help` to any command to see the generated flag list from the binary:

```sh
simdeck ui --help
simdeck daemon start --help
simdeck tap --help
```

:::

## Global Flags

### `--server-url <url>`

| Default | unset                |
| ------- | -------------------- |
| Env     | `SIMDECK_SERVER_URL` |
| Type    | `http://` URL        |

Targets a specific running SimDeck daemon for commands that support the HTTP fast path. If unset, commands start or reuse the current project's daemon when needed.

## `ui`, `daemon start`, And `daemon restart`

`ui`, `daemon start`, and `daemon restart` accept the same server options. `ui` also accepts `--open`.

| Flag               | Default               | Description                                                                                             |
| ------------------ | --------------------- | ------------------------------------------------------------------------------------------------------- |
| `--port <u16>`     | `4310`                | HTTP port for the REST API, browser UI, and WebRTC offer endpoint.                                      |
| `--bind <ip>`      | `127.0.0.1`           | Bind address (`0.0.0.0` for [LAN access](/guide/lan-access), `::` for IPv6).                            |
| `--advertise-host` | matches local host    | Hostname or IP printed for LAN browser access.                                                          |
| `--client-root`    | bundled `client/dist` | Override the static browser client directory.                                                           |
| `--video-codec`    | `h264`                | One of `h264` or `h264-software`. See [Video Pipeline](/guide/video).                                   |
| `--low-latency`    | `false`               | Software H.264 profile for slower runners: caps at 15 fps and favors freshness.                         |
| `--stream-quality` | auto/default          | Optional realtime stream quality profile: `quality`, `balanced`, `smooth`, `economy`, or `ci-software`. |
| `--open`           | `false`               | `ui` only. Open the browser after the daemon is ready.                                                  |

The public commands generate an access token automatically. Use `simdeck daemon status` to read it for direct API callers.

## `describe`

| Flag               | Default                        | Description                                                               |
| ------------------ | ------------------------------ | ------------------------------------------------------------------------- |
| `--format`         | `json`                         | Output format: `json`, `compact-json`, or `agent`.                        |
| `--source`         | `auto`                         | Hierarchy source: `auto`, `nativescript`, `uikit`, or `native-ax`.        |
| `--max-depth`      | unlimited native / `80` daemon | Trim descendants after the requested depth.                               |
| `--include-hidden` | `false`                        | Include hidden in-app inspector views when supported.                     |
| `--direct`         | `false`                        | Skip the daemon and use the private native accessibility bridge directly. |
| `--point <x>,<y>`  | unset                          | Return the native element at a screen point.                              |

## Input Flags

Common input commands:

| Command          | Important flags                                                                                                                                                 |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tap`            | `--id`, `--label`, `--value`, `--element-type`, `--wait-timeout-ms`, `--poll-interval-ms`, `--normalized`, `--duration-ms`, `--pre-delay-ms`, `--post-delay-ms` |
| `touch`          | `--phase`, `--normalized`, `--down`, `--up`, `--delay-ms`                                                                                                       |
| `swipe`          | `--normalized`, `--duration-ms`, `--steps`, `--pre-delay-ms`, `--post-delay-ms`                                                                                 |
| `gesture`        | `--screen-width`, `--screen-height`, `--normalized`, `--duration-ms`, `--delta`, `--pre-delay-ms`, `--post-delay-ms`                                            |
| `pinch`          | `--start-distance`, `--end-distance`, `--angle-degrees`, `--normalized`, `--duration-ms`, `--steps`                                                             |
| `rotate-gesture` | `--radius`, `--degrees`, `--normalized`, `--duration-ms`, `--steps`                                                                                             |
| `type`           | `--stdin`, `--file`, `--delay-ms`                                                                                                                               |
| `key`            | `--modifiers`, `--duration-ms`, `--pre-delay-ms`, `--post-delay-ms`                                                                                             |
| `key-sequence`   | `--keycodes`, `--delay-ms`                                                                                                                                      |
| `key-combo`      | `--modifiers`, `--key`                                                                                                                                          |
| `button`         | `--duration-ms`                                                                                                                                                 |

Coordinates are screen points unless `--normalized` is present. Normalized coordinates are clamped to `0.0..1.0`.

## Evidence And Batch Flags

| Command          | Flags                                                |
| ---------------- | ---------------------------------------------------- |
| `screenshot`     | `--output <path>`, `--stdout`                        |
| `logs`           | `--seconds <f64>`, `--limit <usize>`                 |
| `pasteboard set` | `--stdin`, `--file`                                  |
| `batch`          | `--step`, `--file`, `--stdin`, `--continue-on-error` |

## Exit Codes

| Exit code | Meaning                                                                    |
| --------- | -------------------------------------------------------------------------- |
| `0`       | Success.                                                                   |
| `1`       | Command-level failure (bad usage, missing simulator, native bridge error). |
| `2`       | Clap parser errors.                                                        |

Errors print a short message to stderr; structured JSON is reserved for success output.
