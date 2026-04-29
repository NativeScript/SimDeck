# Contributing

SimDeck welcomes contributions. This page covers the toolchain, the layout, and the working rules to follow when proposing a change.

## Toolchain

You'll need:

- **macOS 13+** with the iOS Simulator runtimes installed.
- **Xcode command-line tools**: `xcode-select --install`.
- **Node.js ≥ 18** and npm.
- **Rust stable** via [rustup](https://rustup.rs/).

Optional:

- **`prettier`** for formatting (installed via `npm install`).
- **`cargo fmt`** and **`cargo clippy`** for Rust formatting and lints (ship with rustup).

## First-time setup

Clone, install dependencies, and build the CLI plus browser client:

```sh
git clone https://github.com/NativeScript/SimDeck.git
cd simdeck
npm install
npm run build
```

`npm install` installs JavaScript tooling only. `npm run build` rebuilds the Rust binary and React bundle. Use `npm run build:all` when you also need the NativeScript inspector, React Native inspector, and `simdeck/test` outputs.

## Running locally

```sh
npm run dev
```

This starts the Rust server in the background and runs the Vite dev server for the React client. The server log lands at `build/cli.log`.

To run only the production server:

```sh
./build/simdeck
```

## Layout

| Folder                             | What lives here                                                                                            |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `server/`                          | CLI entrypoint, project daemon, Rust HTTP server, stream transports, inspector hub, registry, and metrics. |
| `cli/`                             | Objective-C native bridge for private CoreSimulator and SimulatorKit APIs.                                 |
| `client/`                          | React UI served at `/`.                                                                                    |
| `packages/nativescript-inspector/` | TypeScript runtime for the NativeScript inspector.                                                         |
| `packages/inspector-agent/`        | Swift Package for the Swift in-app inspector agent.                                                        |
| `packages/simdeck-test/`           | JS/TS testing API for daemon-backed simulator automation.                                                  |
| `packages/vscode-extension/`       | VS Code extension that opens the simulator inside an editor panel.                                         |
| `scripts/`                         | Repeatable build entrypoints used by both local dev and CI.                                                |
| `bin/`                             | Node launcher that locates and runs the compiled binary.                                                   |
| `docs/`                            | This documentation site (VitePress).                                                                       |

## Working rules

If you contribute, keep these invariants in mind. They are also enforced by the `AGENTS.md` guide that lives at the repo root.

- Simulator-native logic stays in Objective-C under `cli/`.
- Rust server logic stays under `server/`.
- Browser-only presentation logic stays in `client/`.
- NativeScript app runtime inspection logic stays in `packages/nativescript-inspector/`.
- Prefer adding a server endpoint before adding client-only assumptions.
- Don't add a Node or Swift dependency to solve work that already fits in Foundation/AppKit.
- When touching private API usage, keep the adaptation small and explicit and document any simulator/runtime assumptions in `AGENTS.md`.
- Prefer stable CLI subcommands over hidden environment variables.
- The supported live video paths are WebTransport and the experimental WebRTC offer endpoint. Do not bring back legacy `/stream.h264` handling.
- If a feature depends on a booted simulator, fail with a clear JSON error instead of silently returning an empty asset.

## Linting and formatting

Format the entire repo:

```sh
npm run format
```

Check formatting in CI mode (no writes):

```sh
npm run format:check
```

Run all lints:

```sh
npm run lint
```

This runs:

- `prettier --check .`
- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `tsc --noEmit` for the React client.

## Tests

```sh
npm run test
```

This runs the Cargo test suite for the server and the Vitest suite for the client.

Build the JS/TS testing package with:

```sh
npm run build:simdeck-test
```

The simulator-backed CLI integration suite is separate because it creates,
boots, drives, erases, and deletes a temporary iOS simulator. The suite also
builds and installs a tiny SwiftUI fixture app directly with `swiftc` so install,
uninstall, and opt-in launch checks use a deterministic local app bundle:

```sh
npm run build:cli
npm run build:client
npm run test:integration:cli
```

For an interactive local run that opens Simulator.app and prints each CLI/HTTP
step with timings:

```sh
npm run test:integration:cli:verbose
```

The integration runner captures `describe` after each control step. If iOS
shows a known system URL-opening confirmation, the runner handles it and then
captures the UI again before continuing.

Verbose mode prints CLI commands, command output, timings, and UI checkpoints.
Set `SIMDECK_INTEGRATION_TRACE_HTTP=1` if you also need raw HTTP request logs.

Set `SIMDECK_INTEGRATION_KEEP_SIMULATOR=1` with the verbose command if you want
the temporary simulator left around for inspection after the suite exits.

GitHub Actions runs this suite on macOS after the normal build/test pipeline.
The integration suite does not require the live video display bridge; REST input
routes use the non-display native input path, and the video stream is covered by
lower-level protocol tests.

## Full CI pipeline

```sh
npm run ci
```

This is the normal local CI script:

1. `npm run lint` — formatting and lint checks.
2. `npm run build:all` — Rust + Objective-C, React client, NativeScript inspector, React Native inspector, and `simdeck/test`.
3. `npm run test` — Rust and TypeScript tests.
4. `npm run package:vscode-extension` — VS Code `.vsix`.

GitHub Actions runs `npm run ci`, then `npm run test:integration:cli` for the
temp-simulator CLI and REST control sweep. A clean `npm run ci` and integration
run are required for any PR that changes simulator control behavior.

## Documentation

This site is a VitePress project under `docs/`. To preview it:

```sh
npm run docs:dev
```

To build the static site:

```sh
npm run docs:build
```

The build artefact lands at `docs/.vitepress/dist`. The docs deploy workflow (`.github/workflows/docs.yml`) publishes that directory to GitHub Pages on every push to `main`.

When you change something in the repo that the docs already cover — a CLI flag, a route, a packet field, an inspector method — please update the matching docs page in the same PR.

## Filing issues and PRs

- Open an issue for anything that requires discussion before code.
- For straightforward fixes, a PR is fine without a paired issue.
- Include reproduction steps and the macOS / Xcode version when filing simulator-related bugs.
- Include the server log (`build/cli.log` from `npm run dev`, or foreground daemon output from a reproduction) when filing video-stream bugs.

## License

SimDeck is licensed under the Apache License 2.0. By contributing you agree to license your changes under the same terms.
