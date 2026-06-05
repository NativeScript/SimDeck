# Flags

Pass `--help` to any command for the generated flag list:

```sh
simdeck --help
simdeck tap --help
simdeck service restart --help
```

## Global

| Flag                  | Env                  | Purpose                           |
| --------------------- | -------------------- | --------------------------------- |
| `--server-url <url>`  | `SIMDECK_SERVER_URL` | Target a specific running service |
| `--device <selector>` | `SIMDECK_DEVICE`     | One-off simulator override        |

`SIMDECK_UDID` is also accepted for compatibility. Device commands resolve in
this order: positional UDID, `--device`, `SIMDECK_DEVICE`, `SIMDECK_UDID`, the
project default from `simdeck use <udid>`, then auto-inference from the service.

## Server options

Used by `simdeck`, `service start`, `service restart`, `service on`, and `service reset`.
When `service restart` is run without `--port`, it preserves the installed
LaunchAgent port or the current singleton service port before falling back to
`4310`.

| Flag                         | Default        | Notes                                                                                                                  |
| ---------------------------- | -------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `--port <port>` / `-p`       | `4310`         | HTTP port; `service restart` preserves the existing service port when omitted                                          |
| `--bind <ip>`                | `127.0.0.1`    | Use `0.0.0.0` or `::` for LAN access                                                                                   |
| `--advertise-host <host>`    | detected       | Host printed for remote browsers                                                                                       |
| `--client-root <path>`       | bundled client | Static client directory                                                                                                |
| `--video-codec <mode>`       | `auto`         | `auto`, `hardware`, or `software`                                                                                      |
| `--android-gpu <mode>`       | `host`         | Android emulator renderer: `host`, `auto`, `software`, `lavapipe`, `swiftshader`, `swangle`, or `swiftshader_indirect` |
| `--stream-quality <profile>` | `full`         | `full`, `balanced`, `smooth`, `economy`, `low`, `tiny`, `ci-software`, and related profiles                            |
| `--local-stream-fps <fps>`   | `60`           | Local stream frame target                                                                                              |
| `--low-latency`              | off            | Conservative software H.264 profile                                                                                    |
| `--open`                     | off            | Open the browser after starting the service                                                                            |
| `--autostart` / `-a`         | off            | Register the service as a macOS LaunchAgent                                                                            |

## `describe`

Alias: `snapshot`.

| Flag                  | Purpose                                                                                                                                      |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `--format <format>`   | `json`, `compact-json`, or `agent`                                                                                                           |
| `--source <source>`   | `native-ax` by default; accepts `auto`, `nativescript`, `react-native`, `flutter`, `swiftui`, `uikit`, `native-ax`, or `android-uiautomator` |
| `--max-depth <n>`     | Trim hierarchy depth                                                                                                                         |
| `--include-hidden`    | Include hidden nodes when supported                                                                                                          |
| `-i`, `--interactive` | Keep only actionable elements plus ancestors                                                                                                 |
| `--point <x>,<y>`     | Describe the element at a screen point                                                                                                       |
| `--direct`            | Skip service and use native accessibility directly                                                                                           |

## Input

| Command          | Useful flags                                                                                                                                                            |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tap` / `press`  | `--id`, `--label`, `--value`, `--element-type`, `--index`, `--wait-timeout-ms`, `--expect-id`, `--expect-label`, `--expect-timeout-ms`, `--normalized`, `--duration-ms` |
| `touch`          | `--phase`, `--normalized`, `--down`, `--up`, `--delay-ms`                                                                                                               |
| `swipe`          | `--normalized`, `--duration-ms`, `--steps`                                                                                                                              |
| `gesture`        | `--normalized`, `--duration-ms`, `--delta`                                                                                                                              |
| `pinch`          | `--start-distance`, `--end-distance`, `--angle-degrees`, `--normalized`                                                                                                 |
| `rotate-gesture` | `--radius`, `--degrees`, `--normalized`                                                                                                                                 |
| `type`           | `--stdin`, `--file`, `--delay-ms`                                                                                                                                       |
| `key`            | `--modifiers`, `--duration-ms`                                                                                                                                          |
| `key-sequence`   | `--keycodes`, `--delay-ms`                                                                                                                                              |
| `key-combo`      | `--modifiers`, `--key`                                                                                                                                                  |
| `button`         | `--duration-ms`                                                                                                                                                         |
| `back`           | `--timeout-ms`, `--poll-interval-ms`, `--no-fallback-swipe`                                                                                                             |

`describe --format agent` prints element refs like `@e3`; pass one as
`simdeck press @e3` to target that element by traversal order.
Use `tap --expect-*` to wait for a post-tap state in the same command; use
`back` for app-owned navigation instead of first discovering the current back
button label.

## Evidence and batch

| Command          | Flags                                                |
| ---------------- | ---------------------------------------------------- |
| `screenshot`     | `--output <path>`, `--stdout`, `--with-bezel`        |
| `record`         | `--seconds <seconds>`, `--output <path>`, `--stdout` |
| `logs`           | `--seconds <seconds>`, `--limit <count>`             |
| `stats`          | `--pid <pid>`, `--watch`, `--interval <seconds>`     |
| `sample`         | `--pid <pid>`, `--seconds <seconds>`                 |
| `pasteboard set` | `--stdin`, `--file`                                  |
| `batch`          | `--step`, `--file`, `--stdin`, `--continue-on-error` |

## Camera Simulation

| Command         | Flags                                                                              |
| --------------- | ---------------------------------------------------------------------------------- |
| `camera start`  | `--file <path-or-url>`, `--webcam [id]`, `--mirror auto\|on\|off`                  |
| `camera switch` | `--file <path-or-url>`, `--webcam [id]`, `--placeholder`, `--mirror auto\|on\|off` |
| `camera status` | none                                                                               |
| `camera stop`   | none                                                                               |

## Exit codes

| Code | Meaning                    |
| ---- | -------------------------- |
| `0`  | Success                    |
| `1`  | Runtime or command failure |
| `2`  | Argument parsing failure   |
