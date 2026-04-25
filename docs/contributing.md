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
- **AXe** if you want to test the accessibility fallback path.

## First-time setup

Clone, install dependencies, and build everything:

```sh
git clone https://github.com/DjDeveloperr/xcode-canvas-web.git
cd xcode-canvas-web
npm install
npm run build
```

`npm install` runs the postinstall hook that compiles the Rust + Objective-C CLI. `npm run build` rebuilds everything top-to-bottom (Rust binary, React bundle, NativeScript inspector).

## Running locally

```sh
npm run dev
```

This starts the Rust server in the background and runs the Vite dev server for the React client. The server log lands at `build/cli.log`.

To run only the production server:

```sh
./build/xcode-canvas-web serve --port 4310
```

## Layout

| Folder                             | What lives here                                                                        |
| ---------------------------------- | -------------------------------------------------------------------------------------- |
| `server/`                          | Rust HTTP server, WebTransport hub, inspector hub, registry, metrics, launchd service. |
| `cli/`                             | Objective-C native bridge for private CoreSimulator and SimulatorKit APIs.             |
| `client/`                          | React UI served at `/`.                                                                |
| `packages/nativescript-inspector/` | TypeScript runtime for the NativeScript inspector.                                     |
| `packages/inspector-agent/`        | Swift Package for the Swift in-app inspector agent.                                    |
| `packages/vscode-extension/`       | VS Code extension that opens the simulator inside an editor panel.                     |
| `scripts/`                         | Repeatable build entrypoints used by both local dev and CI.                            |
| `bin/`                             | Node launcher that locates and runs the compiled binary.                               |
| `docs/`                            | This documentation site (VitePress).                                                   |

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
- The supported live video path is WebTransport-only. Do not bring back legacy `/stream.h264` handling.
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

## Full CI pipeline

```sh
npm run ci
```

This is the same script that GitHub Actions runs:

1. `npm run lint` — formatting and lint checks.
2. `npm run build` — Rust + Objective-C, React client, NativeScript inspector.
3. `npm run test` — Rust and TypeScript tests.
4. `npm run package:vscode-extension` — VS Code `.vsix`.

A clean `npm run ci` is required for any PR.

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
- Include the server log (foreground or `~/Library/Logs/xcode-canvas-web*.log`) when filing video-stream bugs.

## License

SimDeck is licensed under the Apache License 2.0. By contributing you agree to license your changes under the same terms.
