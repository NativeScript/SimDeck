# Flags & Options

A consolidated list of every flag accepted by the SimDeck CLI, grouped by where it applies.

## Global flags

There are currently no global flags. Every option is scoped to a subcommand.

::: tip Help output
Pass `--help` to any subcommand to see the same flag list directly from the binary, including any flags that may have been added after this page was written:

```sh
simdeck serve --help
simdeck service on --help
```

:::

## `serve` and `service on` flags

These two subcommands accept the same flags because the service command writes them straight into the launchd plist.

### `--port <u16>`

| Default | `4310`          |
| ------- | --------------- |
| Type    | unsigned 16-bit |

The HTTP port. WebTransport listens on `port + 1`.

### `--bind <ip>`

| Default | `127.0.0.1`  |
| ------- | ------------ |
| Type    | IPv4 or IPv6 |

Bind address for both the HTTP server and the WebTransport server. Common values:

- `127.0.0.1` — localhost only.
- `0.0.0.0` — every IPv4 interface.
- `::` — every IPv4 and IPv6 interface (when supported by the OS dual-stack config).
- A specific interface IP.

### `--advertise-host <host>`

| Default | matches `--bind` (or `localhost` for unspecified addresses) |
| ------- | ----------------------------------------------------------- |
| Type    | hostname or IP literal                                      |

Hostname or IP that gets baked into the WebTransport URL template advertised at `GET /api/health`, and added to the certificate's Subject Alternative Names.

If you bind to `0.0.0.0` and don't pass `--advertise-host`, SimDeck warns at startup because the default `localhost` won't work for remote clients.

### `--client-root <path>`

| Default | `client/dist` next to the binary, falling back to `./client/dist` |
| ------- | ----------------------------------------------------------------- |
| Type    | filesystem path                                                   |

Override the static client directory. The Rust server serves the contents at the HTTP root and falls through to a 404 for missing files.

### `--video-codec <codec>`

| Default | `hevc`                                 |
| ------- | -------------------------------------- |
| Type    | one of `hevc`, `h264`, `h264-software` |

Encoder used by the native bridge. See [Video Pipeline](/guide/video) for when to switch.

### `--access-token <token>`

| Default | generated at startup |
| ------- | -------------------- |
| Type    | string               |

HTTP API and WebTransport access token. The served browser UI receives it automatically through a strict same-site cookie, so normal local use does not require copying the token. Direct API callers should send either `X-SimDeck-Token: <token>` or `Authorization: Bearer <token>`.

## Global CLI flags

### `--server-url <url>`

| Default | unset                          |
| ------- | ------------------------------ |
| Env     | `SIMDECK_SERVER_URL`           |
| Type    | `http://` URL for local server |

When set, supported hot controls delegate to the warm local SimDeck service instead of starting a fresh native control path in the CLI process. This is fastest for agent-driven loops. Supported delegated controls include `launch`, `open-url`, normalized `touch`, normalized coordinate `tap`, normalized `swipe`, normalized `gesture`, `key`, `key-sequence`, `key-combo`, `button`, `dismiss-keyboard`, `home`, `app-switcher`, `rotate-left`, `rotate-right`, and `toggle-appearance`.

## Positional arguments

Subcommands that take positionals expect them in the order shown:

| Command    | Positionals         | Notes                                        |
| ---------- | ------------------- | -------------------------------------------- |
| `boot`     | `<udid>`            | Simulator UDID from `simdeck list`.          |
| `shutdown` | `<udid>`            |                                              |
| `open-url` | `<udid> <url>`      | Any URL scheme accepted by `simctl openurl`. |
| `launch`   | `<udid> <bundleId>` | App must already be installed.               |

## `describe-ui` flags

| Flag                 | Default                                | Description                                                                      |
| -------------------- | -------------------------------------- | -------------------------------------------------------------------------------- |
| `--format`           | `json`                                 | Output format: `json`, `compact-json`, or `agent`.                               |
| `--source`           | `auto`                                 | Hierarchy source: `auto`, `nativescript`, `uikit`, or `native-ax`.               |
| `--max-depth`        | unlimited native / `80` service        | Trim descendants after the requested depth.                                      |
| `--include-hidden`   | `false`                                | Include hidden in-app inspector views when supported.                            |
| `--direct`           | `false`                                | Skip the local service and use the private native accessibility bridge directly. |
| `--point <x>,<y>`    | unset                                  | Return the native element at a screen point.                                     |
| `--server-url <url>` | global flag or `http://127.0.0.1:4310` | Local service URL used for source-aware hierarchy requests.                      |

## Exit codes

| Exit code | Meaning                                                                    |
| --------- | -------------------------------------------------------------------------- |
| `0`       | Success.                                                                   |
| `1`       | Command-level failure (bad usage, missing simulator, native bridge error). |
| `2`       | Reserved by Clap for usage / parser errors.                                |

Errors print a short message to stderr; structured JSON is reserved for success output.
