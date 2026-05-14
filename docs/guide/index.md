# Overview

SimDeck is a local tool for viewing, controlling, inspecting, and automating mobile simulators.

Run `simdeck` from your project. It starts a local server, serves a browser UI, and exposes the same controls through the CLI and HTTP API.

## What You Can Do

- View a live iOS Simulator or Android emulator in a browser.
- Tap, swipe, type, press hardware buttons, rotate, and open URLs.
- Install, launch, uninstall, boot, shut down, and erase devices.
- Capture screenshots, logs, pasteboard text, and accessibility trees.
- Inspect app UI through built-in accessibility or optional in-app inspectors.
- Write JS/TS automation with `simdeck/test`.
- Share a paired browser session over your LAN.
- Open the simulator view inside VS Code.

## Daily Workflow

```sh
simdeck
```

Open the local URL, pick a device, and use the toolbar or CLI commands:

```sh
simdeck list
simdeck boot <udid>
simdeck install <udid> /path/to/App.app
simdeck install <udid> /path/to/App.ipa
simdeck launch <udid> com.example.App
simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
simdeck describe <udid> --format agent --max-depth 3
```

Use `simdeck -d` for a detached background daemon, `simdeck -k` to stop it, and `simdeck -r` to restart it.

## Pick A Page

- [Install](/guide/installation): requirements and setup.
- [Quick Start](/guide/quick-start): first browser session.
- [Daemon](/guide/daemon): foreground, detached, and always-on modes.
- [Video & Streaming](/guide/video): stream quality and codec choices.
- [LAN Access](/guide/lan-access): pairing and remote browser access.
- [Testing](/guide/testing): `simdeck/test` and integration tests.
- [Troubleshooting](/guide/troubleshooting): practical fixes for common failures.
