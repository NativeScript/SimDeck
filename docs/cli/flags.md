# Flags

Pass `--help` to any command for the generated flag list:

```sh
simdeck --help
simdeck tap --help
simdeck daemon start --help
```

## Global

| Flag                 | Env                  | Purpose                          |
| -------------------- | -------------------- | -------------------------------- |
| `--server-url <url>` | `SIMDECK_SERVER_URL` | Target a specific running daemon |

## Server Options

Used by `simdeck ui`, `daemon start`, `daemon restart`, `service on`, and `service restart`.

| Flag                         | Default        | Notes                                                                             |
| ---------------------------- | -------------- | --------------------------------------------------------------------------------- | ------ | ------------ |
| `--port <port>`              | `4310`         | HTTP port                                                                         |
| `--bind <ip>`                | `127.0.0.1`    | Use `0.0.0.0` or `::` for LAN access                                              |
| `--advertise-host <host>`    | detected       | Host printed for remote browsers                                                  |
| `--client-root <path>`       | bundled client | Static client directory                                                           |
| `--video-codec auto          | hardware       | software`                                                                         | `auto` | Encoder mode |
| `--stream-quality <profile>` | `full`         | `full`, `balanced`, `economy`, `low`, `tiny`, `ci-software`, and related profiles |
| `--local-stream-fps <fps>`   | `60`           | Local stream frame target                                                         |
| `--low-latency`              | off            | Conservative software H.264 profile                                               |
| `--open`                     | off            | `ui` only                                                                         |

## `describe`

| Flag               | Purpose                                           |
| ------------------ | ------------------------------------------------- | ------------ | ------------------- | ----- | --------- | -------------------- | --------------------- |
| `--format json     | compact-json                                      | agent`       | Choose output shape |
| `--source auto     | nativescript                                      | react-native | flutter             | uikit | native-ax | android-uiautomator` | Pick inspector source |
| `--max-depth <n>`  | Trim hierarchy depth                              |
| `--include-hidden` | Include hidden nodes when supported               |
| `--point <x>,<y>`  | Describe the element at a screen point            |
| `--direct`         | Skip daemon and use native accessibility directly |

## Input

| Command          | Useful flags                                                                                         |
| ---------------- | ---------------------------------------------------------------------------------------------------- |
| `tap`            | `--id`, `--label`, `--value`, `--element-type`, `--wait-timeout-ms`, `--normalized`, `--duration-ms` |
| `touch`          | `--phase`, `--normalized`, `--down`, `--up`, `--delay-ms`                                            |
| `swipe`          | `--normalized`, `--duration-ms`, `--steps`                                                           |
| `gesture`        | `--normalized`, `--duration-ms`, `--delta`                                                           |
| `pinch`          | `--start-distance`, `--end-distance`, `--angle-degrees`, `--normalized`                              |
| `rotate-gesture` | `--radius`, `--degrees`, `--normalized`                                                              |
| `type`           | `--stdin`, `--file`, `--delay-ms`                                                                    |
| `key`            | `--modifiers`, `--duration-ms`                                                                       |
| `key-sequence`   | `--keycodes`, `--delay-ms`                                                                           |
| `key-combo`      | `--modifiers`, `--key`                                                                               |
| `button`         | `--duration-ms`                                                                                      |

## Evidence And Batch

| Command          | Flags                                                |
| ---------------- | ---------------------------------------------------- |
| `screenshot`     | `--output <path>`, `--stdout`, `--with-bezel`        |
| `record`         | `--seconds <seconds>`, `--output <path>`, `--stdout` |
| `logs`           | `--seconds <seconds>`, `--limit <count>`             |
| `stats`          | `--pid <pid>`, `--watch`, `--interval <seconds>`     |
| `sample`         | `--pid <pid>`, `--seconds <seconds>`                 |
| `pasteboard set` | `--stdin`, `--file`                                  |
| `batch`          | `--step`, `--file`, `--stdin`, `--continue-on-error` |

## Exit Codes

| Code | Meaning                    |
| ---- | -------------------------- |
| `0`  | Success                    |
| `1`  | Runtime or command failure |
| `2`  | Argument parsing failure   |
