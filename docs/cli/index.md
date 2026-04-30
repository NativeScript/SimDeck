# CLI

The `simdeck` binary is the only entrypoint SimDeck ships. It opens the browser UI, manages a warm project daemon, and exposes simulator-control subcommands for scripts and tests.

## Synopsis

```sh
simdeck [SIMULATOR_NAME_OR_UDID]
simdeck [-d|--detached]
simdeck [-k|--kill]
simdeck [-r|--restart]
simdeck [--server-url <url>] <COMMAND> [OPTIONS]
```

With no subcommand, `simdeck` starts a foreground workspace daemon and prints local/LAN browser URLs. A single argument selects that simulator by name or UDID in the UI. Use `-d`, `-k`, and `-r` as short aliases for detached start, stop, and restart.

Most commands automatically start or reuse the project daemon when that is the fastest path. Set `SIMDECK_SERVER_URL=http://127.0.0.1:4310` or pass `--server-url` to target a specific already-running daemon.

## Top-level commands

| Command                                                 | Purpose                                                     |
| ------------------------------------------------------- | ----------------------------------------------------------- |
| _(none)_                                                | Start a foreground UI daemon until `q` or Ctrl-C.           |
| `ui`                                                    | Start or reuse the project daemon and serve the browser UI. |
| `daemon start/status/stop`                              | Manage the project daemon explicitly.                       |
| `core-simulator ...`                                    | Restart or manage Apple's CoreSimulator service layer.      |
| `list`                                                  | Print every simulator known to the native bridge as JSON.   |
| `boot` / `shutdown` / `erase`                           | Manage simulator lifecycle.                                 |
| `install` / `uninstall` / `launch` / `open-url`         | Manage apps and URLs inside a simulator.                    |
| `describe`                                              | Print accessibility or in-app inspector hierarchy data.     |
| `tap` / `swipe` / `gesture` / `type` / `key` / `button` | Drive simulator input.                                      |
| `screenshot` / `logs` / `pasteboard` / `chrome-profile` | Collect evidence and device metadata.                       |

Every subcommand returns an exit code that follows shell conventions: zero on success, non-zero on failure.

## Output format

Most subcommands print JSON. This makes the CLI easy to consume from scripts:

```sh
simdeck list | jq '.simulators[] | select(.isBooted)'
```

Errors print a short human-readable message to stderr and a non-zero exit code. They do not print structured JSON for failure cases.

## See also

- **[Command Reference](/cli/commands)** — every subcommand in detail.
- **[Flags & Options](/cli/flags)** — global flags and per-subcommand options.
- **[REST API](/api/rest)** — the HTTP equivalent of every CLI subcommand.
