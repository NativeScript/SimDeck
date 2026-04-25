# Flags & Options

A consolidated list of every flag accepted by the Simdeck CLI, grouped by where it applies.

## Global flags

There are currently no global flags. Every option is scoped to a subcommand.

::: tip Help output
Pass `--help` to any subcommand to see the same flag list directly from the binary, including any flags that may have been added after this page was written:

```sh
xcode-canvas-web serve --help
xcode-canvas-web service on --help
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

If you bind to `0.0.0.0` and don't pass `--advertise-host`, Simdeck warns at startup because the default `localhost` won't work for remote clients.

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

## Positional arguments

Subcommands that take positionals expect them in the order shown:

| Command    | Positionals         | Notes                                        |
| ---------- | ------------------- | -------------------------------------------- |
| `boot`     | `<udid>`            | Simulator UDID from `xcode-canvas-web list`. |
| `shutdown` | `<udid>`            |                                              |
| `open-url` | `<udid> <url>`      | Any URL scheme accepted by `simctl openurl`. |
| `launch`   | `<udid> <bundleId>` | App must already be installed.               |

## Exit codes

| Exit code | Meaning                                                                    |
| --------- | -------------------------------------------------------------------------- |
| `0`       | Success.                                                                   |
| `1`       | Command-level failure (bad usage, missing simulator, native bridge error). |
| `2`       | Reserved by Clap for usage / parser errors.                                |

Errors print a short message to stderr; structured JSON is reserved for success output.
