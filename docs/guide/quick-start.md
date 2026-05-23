# Quick Start

## 1. Start SimDeck

```sh
simdeck
```

SimDeck prints a local browser URL, a LAN URL when one is available, and a pairing code for LAN browsers.

```text
SimDeck is ready

Local:   http://127.0.0.1:4311
Network: http://192.168.1.50:4311
Pair:    123 456
```

Open the local URL. Press `q` or Ctrl-C in the terminal to stop the foreground server.

To open a specific simulator by name or UDID:

```sh
simdeck "iPhone 17 Pro Max"
simdeck 9750DF52-0471-48FF-B49A-B184C4BD3A3D
```

## 2. Pick Or Boot A Device

The UI lists available iOS Simulators and Android emulators. You can also use the CLI:

```sh
simdeck list
simdeck use <udid>
simdeck boot <udid>
```

`simdeck use <udid>` saves the simulator default for this project directory so
later device commands can omit the UDID. Android emulator IDs are prefixed with
`android:`.

## 3. Install And Launch An App

```sh
simdeck install /path/to/App.app
simdeck install /path/to/App.ipa
simdeck launch com.example.App
simdeck open-url myapp://debug
```

For Android:

```sh
simdeck install android:<avd-name> /path/to/app.apk
simdeck launch android:<avd-name> com.example.app
```

## 4. Drive The UI

Use coordinates when you know them:

```sh
simdeck tap 120 240
simdeck swipe 200 700 200 200
simdeck type "hello"
```

Use selectors when you want automation to wait for UI state:

```sh
simdeck tap --label "Continue" --wait-timeout-ms 5000
simdeck tap --id com.apple.settings.screenTime --expect-id BackButton
simdeck tap "Continue"
simdeck back
simdeck describe --format agent --max-depth 3 --interactive
simdeck press @e3
```

`describe --format agent` prints refs such as `@e3`; use `press @e3` to target
one of those elements directly. `snapshot`, `press`, and `wait` are aliases for
`describe`, `tap`, and `wait-for`. Add `--expect-*` to a tap when the next
screen should be present before the command returns.

## 5. Keep It Running In The Background

```sh
simdeck -d
simdeck -k
simdeck -r
```

These are shortcuts for detached start, stop, and restart. See [Daemon](/guide/daemon) for details.

## Next

- [CLI commands](/cli/commands)
- [Video & streaming](/guide/video)
- [Inspectors](/inspector/)
- [Troubleshooting](/guide/troubleshooting)
