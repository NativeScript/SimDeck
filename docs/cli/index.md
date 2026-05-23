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

Use `simdeck use <udid>` once per project directory to make that simulator the
default for later device commands. Most commands accept `[<udid>]`; `--device`,
`SIMDECK_DEVICE`, and `SIMDECK_UDID` override the saved project default when a
one-off target is needed.

Use `--server-url` or `SIMDECK_SERVER_URL` when a script should target a specific daemon:

```sh
SIMDECK_SERVER_URL=http://127.0.0.1:4310 simdeck list
```

## Most-Used Commands

```sh
simdeck list
simdeck use <udid>
simdeck boot <udid>
simdeck install /path/to/App.app
simdeck install /path/to/App.ipa
simdeck launch com.example.App
simdeck open-url https://example.com
simdeck tap --label "Continue" --wait-timeout-ms 5000
simdeck tap "Continue"
simdeck describe --format agent --max-depth 3 --interactive
simdeck screenshot --output screen.png
simdeck screenshot --with-bezel --output screen-bezel.png
simdeck record --seconds 5 --output screen-recording.mp4
simdeck logs --seconds 30 --limit 200
simdeck stats
simdeck sample --seconds 3
```

The explicit form still works, for example `simdeck launch <udid> com.example.App`.

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
