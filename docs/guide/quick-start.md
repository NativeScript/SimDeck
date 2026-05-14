# Quick Start

## 1. Start SimDeck

```sh
simdeck
```

SimDeck prints a local browser URL, a LAN URL when one is available, and a pairing code for LAN browsers.

```text
SimDeck is ready

Local:   http://127.0.0.1:4310
Network: http://192.168.1.50:4310
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
simdeck boot <udid>
```

Android emulator IDs are prefixed with `android:`.

## 3. Install And Launch An App

```sh
simdeck install <udid> /path/to/App.app
simdeck install <udid> /path/to/App.ipa
simdeck launch <udid> com.example.App
simdeck open-url <udid> myapp://debug
```

For Android:

```sh
simdeck install android:<avd-name> /path/to/app.apk
simdeck launch android:<avd-name> com.example.app
```

## 4. Drive The UI

Use coordinates when you know them:

```sh
simdeck tap <udid> 120 240
simdeck swipe <udid> 200 700 200 200
simdeck type <udid> "hello"
```

Use selectors when you want automation to wait for UI state:

```sh
simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
simdeck describe <udid> --format agent --max-depth 3
```

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
