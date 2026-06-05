# How it works

This page is a short mental model for users and contributors. For daily usage, start with [Quick start](/guide/quick-start) and [CLI commands](/cli/commands).

## Pieces

| Piece              | What it does                                                        |
| ------------------ | ------------------------------------------------------------------- |
| CLI                | Starts SimDeck and exposes scriptable commands                      |
| Local service      | Serves the browser UI, API, streams, metrics, and inspector routing |
| Browser client     | Shows the live device, toolbar, inspector panes, and diagnostics    |
| Native bridge      | Handles simulator-specific work on macOS                            |
| Inspector runtimes | Optional app packages that publish richer UI trees                  |
| `simdeck/test`     | JS/TS wrapper for automation                                        |

## Request flow

Most user actions follow the same path:

1. Browser, CLI, or test sends a command to the service.
2. The service checks the selected device and starts a warm session when needed.
3. SimDeck performs the requested simulator or emulator action.
4. The command returns JSON, a screenshot, a recording, logs, or updated stream state.

This is why a long-lived service feels faster than repeatedly calling lower-level simulator tools.

## Video flow

The browser opens a live WebRTC stream for the selected device. SimDeck sends fresh frames, drops stale ones when a client falls behind, and lets the browser request refreshes. iOS frames come from the native display bridge and are encoded on the Mac; Android frames come from the emulator `-share-vid` shared display surface and are encoded on the Mac.

Tune this from the user-facing controls or with:

```sh
simdeck service restart --video-codec software --stream-quality low
```

## Inspector flow

`simdeck describe` and the browser inspector use the best available source:

1. Framework runtime inspector, such as NativeScript, React Native, or Flutter.
2. Swift in-app agent for UIKit or SwiftUI apps.
3. Native accessibility snapshot as the universal fallback.

The response tells you which source was used and why a requested source fell back.

## Repository layout

| Folder                    | Purpose                                           |
| ------------------------- | ------------------------------------------------- |
| `packages/server/`        | CLI entrypoint, service, API, streaming, metrics  |
| `packages/server/native/` | macOS simulator bridge                            |
| `packages/client/`        | Browser UI                                        |
| `packages/`               | Inspectors, VS Code extension, and `simdeck/test` |
| `scripts/`                | Build, packaging, and integration helpers         |
| `docs/`                   | Documentation site                                |

## Contributor boundary

Keep platform-specific simulator work in the native layer, server behavior in `packages/server/`, and browser presentation in `packages/client/`. Add API support before adding UI assumptions that cannot be scripted.
