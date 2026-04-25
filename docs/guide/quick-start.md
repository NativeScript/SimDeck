# Quick Start

This guide walks you from a fresh install to a Simulator streaming in your browser in three steps.

## 1. Start the server

After [installing](/guide/installation), launch the local server:

```sh
simdeck serve --port 4310
```

The server prints the HTTP and WebTransport URLs as it boots:

```text
HTTP listening on http://127.0.0.1:4310
WebTransport listening on https://127.0.0.1:4311/wt/simulators/{udid}
Serving client from /usr/local/lib/node_modules/simdeck/client/dist
```

Two listeners come up:

- **HTTP** on `--port` (default `4310`) for the REST API and the static React client.
- **WebTransport** on `port + 1` (default `4311`) for binary video frames. The server generates a self-signed certificate per session and advertises its hash through `GET /api/health`.

Leave the server running in a terminal, or move it into the [background service](/guide/service).

## 2. Open the client

Visit:

```text
http://127.0.0.1:4310
```

The React client connects to `GET /api/health` to discover the WebTransport URL template and certificate hash, lists every Simulator on the machine, and lets you boot, stream, and interact with one.

::: tip First-frame delay
On a cold boot the server has to launch the Simulator, attach the private display bridge, and wait for a keyframe before any video flows. The first frame typically shows up within a second; subsequent reloads of the same Simulator are near-instant.
:::

## 3. Boot a simulator from the CLI

You can drive simulators directly from the command line in addition to the browser:

```sh
simdeck list
simdeck boot <udid>
simdeck open-url <udid> https://example.com
simdeck launch <udid> com.apple.Preferences
simdeck shutdown <udid>
```

`list` returns the same data the React client renders — including which simulators are booted and which have an attached private display session.

See the full [CLI Reference](/cli/commands) for every command and flag.

## What just happened?

When you opened the browser:

1. The browser fetched `/api/health` and learned where the WebTransport endpoint lives.
2. It opened a WebTransport session at `wss://127.0.0.1:4311/wt/simulators/<udid>` and pinned the self-signed certificate by hash.
3. The Rust transport hub asked the native bridge for a fresh keyframe and started forwarding binary video packets.
4. Touch and keyboard events round-trip through `POST /api/simulators/<udid>/touch` and `/key`, which the native bridge replays through HID.

You can read more about the layers involved in [Architecture](/guide/architecture) and [WebTransport](/api/webtransport).

## Common follow-ups

- **Run the server on a LAN-reachable port.** See [LAN Access](/guide/lan-access).
- **Keep the server running across logouts.** See [Background Service](/guide/service).
- **Stream is choppy or stuck on a black frame.** See [Video Pipeline](/guide/video) and [Troubleshooting](/guide/troubleshooting).
- **Embed the client in VS Code.** See the [VS Code extension](/extensions/vscode).
