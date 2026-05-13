# CLI

`simdeck` is the main entrypoint for opening the browser UI, managing the daemon, and scripting simulator actions.

## Common Use

```sh
simdeck
simdeck "iPhone 17 Pro Max"
simdeck -d
simdeck -k
simdeck -r
```

With no subcommand, SimDeck starts a foreground server and prints browser URLs. A single simulator name or UDID selects that device in the UI. The shorthand flags start, stop, and restart the detached project daemon.

## Command Shape

```sh
simdeck [SIMULATOR_NAME_OR_UDID]
simdeck [--server-url <url>] <command> [options]
```

Use `--server-url` or `SIMDECK_SERVER_URL` when a script should target a specific daemon:

```sh
SIMDECK_SERVER_URL=http://127.0.0.1:4310 simdeck list
```

## Most-Used Commands

```sh
simdeck list
simdeck boot <udid>
simdeck install <udid> /path/to/App.app
simdeck launch <udid> com.example.App
simdeck open-url <udid> https://example.com
simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
simdeck describe <udid> --format agent --max-depth 3
simdeck screenshot <udid> --output screen.png
simdeck logs <udid> --seconds 30 --limit 200
```

Most successful commands print JSON so they can be piped into tools such as `jq`.

## Help

```sh
simdeck --help
simdeck tap --help
simdeck daemon start --help
```

## Next

- [Commands](/cli/commands)
- [Flags](/cli/flags)
- [REST API](/api/rest)
