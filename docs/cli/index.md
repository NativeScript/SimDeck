# CLI

The `simdeck` binary is the only entrypoint SimDeck ships. It hosts the HTTP server, manages the launchd service, and exposes a small set of simulator-control subcommands that are convenient from scripts.

## Synopsis

```sh
simdeck [--server-url <url>] <COMMAND> [OPTIONS]
```

Set `SIMDECK_SERVER_URL=http://127.0.0.1:4310` or pass `--server-url` to route supported hot controls through an already-running local service. That avoids repeated native setup for agent loops while preserving the same JSON command output.

## Top-level commands

| Command                    | Purpose                                                                                     |
| -------------------------- | ------------------------------------------------------------------------------------------- |
| `serve`                    | Start the HTTP and WebTransport servers in the foreground.                                  |
| `service on/off`           | Install or remove the per-user `launchd` service. See [Background Service](/guide/service). |
| `list`                     | Print every simulator known to the native bridge as JSON.                                   |
| `boot <udid>`              | Boot the given simulator.                                                                   |
| `shutdown <udid>`          | Shut the given simulator down.                                                              |
| `open-url <udid> <url>`    | Open a URL inside the simulator (Safari for `https://`, deep links otherwise).              |
| `launch <udid> <bundleId>` | Launch an installed app by its bundle identifier.                                           |

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
