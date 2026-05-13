# How It Works

This page is a short mental model for users and contributors. For daily usage, start with [Quick Start](/guide/quick-start) and [CLI commands](/cli/commands).

## Pieces

| Piece              | What it does                                                        |
| ------------------ | ------------------------------------------------------------------- |
| CLI                | Starts SimDeck and exposes scriptable commands                      |
| Local daemon       | Serves the browser UI, API, streams, metrics, and inspector routing |
| Browser client     | Shows the live device, toolbar, inspector panes, and diagnostics    |
| Native bridge      | Handles simulator-specific work on macOS                            |
| Inspector runtimes | Optional app packages that publish richer UI trees                  |
| `simdeck/test`     | JS/TS wrapper for automation                                        |

## Request Flow

Most user actions follow the same path:

1. Browser, CLI, or test sends a command to the daemon.
2. The daemon checks the selected device and starts a warm session when needed.
3. SimDeck performs the requested simulator or emulator action.
4. The command returns JSON, a screenshot, logs, or updated stream state.

This is why a long-lived daemon feels faster than repeatedly calling lower-level simulator tools.

## Video Flow

The browser opens a live stream for the selected device. SimDeck sends fresh frames, drops stale ones when a client falls behind, and lets the browser request refreshes. The UI can use WebRTC or H.264-over-WebSocket fallback depending on browser support and network behavior.

Tune this from the user-facing controls or with:

```sh
simdeck daemon restart --video-codec software --stream-quality low
```

## Inspector Flow

`simdeck describe` and the browser inspector use the best available source:

1. Framework runtime inspector, such as NativeScript, React Native, or Flutter.
2. Swift in-app agent for UIKit or SwiftUI apps.
3. Native accessibility snapshot as the universal fallback.

The response tells you which source was used and why a requested source fell back.

## Repository Layout

| Folder      | Purpose                                           |
| ----------- | ------------------------------------------------- |
| `server/`   | CLI entrypoint, daemon, API, streaming, metrics   |
| `cli/`      | macOS simulator bridge                            |
| `client/`   | Browser UI                                        |
| `packages/` | Inspectors, VS Code extension, and `simdeck/test` |
| `scripts/`  | Build, packaging, and integration helpers         |
| `docs/`     | Documentation site                                |

## Contributor Boundary

Keep platform-specific simulator work in the native layer, server behavior in `server/`, and browser presentation in `client/`. Add API support before adding UI assumptions that cannot be scripted.
