# Quick Start

This guide walks you from a fresh install to a Simulator streaming in your browser and controllable from the CLI.

## 1. Open The UI

After [installing](/guide/installation), start a foreground SimDeck daemon and open one of the printed browser URLs:

```sh
simdeck
```

The command prints local and LAN URLs:

```text
🚀 SimDeck is ready

      Local:   http://127.0.0.1:4310
    Network:   http://192.168.1.50:4310
       Pair:   123 456

q or ^C to stop server
```

This foreground daemon is scoped to the current workspace and exits when the command exits, when you press `q`, or when you press Ctrl-C. Local browsers open without a prompt. LAN browsers pair once with the printed code, then receive the API cookie. Use `simdeck ui --open` or `simdeck daemon start` when you want a reusable background daemon.

For shorthand background lifecycle commands:

```sh
simdeck -d  # detached start
simdeck -k  # kill background daemon
simdeck -r  # restart background daemon
```

One HTTP listener runs inside the daemon:

- **HTTP** on `--port` (default `4310`) for the REST API, static React client, inspector WebSocket, and WebRTC offer endpoint.

## 2. Pick A Simulator

The opened UI lists every simulator. You can also list and boot from the CLI:

```sh
simdeck list
simdeck boot <udid>
```

To focus a specific simulator by name or UDID at launch:

```sh
simdeck "iPhone 17 Pro Max"
simdeck 9750DF52-0471-48FF-B49A-B184C4BD3A3D
```

::: tip First-frame delay
On a cold boot the daemon has to launch the Simulator, attach the private display bridge, and wait for a keyframe before video flows. The first frame typically shows up within a second; subsequent reloads of the same Simulator are near-instant.
:::

## 3. Drive It

The same daemon backs browser controls, CLI commands, and tests:

```sh
simdeck open-url <udid> https://example.com
simdeck launch <udid> com.apple.Preferences
simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
simdeck describe <udid> --format agent --max-depth 2
```

Coordinate commands use screen points by default. Pass `--normalized` when sending `0.0..1.0` coordinates:

```sh
simdeck tap <udid> 0.5 0.5 --normalized
```

See the full [CLI Reference](/cli/commands) for every command and flag.

## What just happened?

When you opened the UI:

1. The browser fetched `/api/health` and confirmed the active H.264 encoder mode.
2. It posted a WebRTC SDP offer to `/api/simulators/<udid>/webrtc/offer`.
3. The Rust transport hub asked the native bridge for a fresh keyframe and started writing H.264 samples to the WebRTC video track.
4. Touch and keyboard events round-trip through `POST /api/simulators/<udid>/touch` and `/key`, which the native bridge replays through HID.

You can read more about the layers involved in [Architecture](/guide/architecture) and [Video Pipeline](/guide/video).

## Common follow-ups

- **Run the server on a LAN-reachable port.** See [LAN Access](/guide/lan-access).
- **Manage the warm native host explicitly.** See [Project Daemon](/guide/daemon).
- **Write JS/TS app tests.** See [Testing](/guide/testing).
- **Stream is choppy or stuck on a black frame.** See [Video Pipeline](/guide/video) and [Troubleshooting](/guide/troubleshooting).
- **Embed the client in VS Code.** See the [VS Code extension](/extensions/vscode).
