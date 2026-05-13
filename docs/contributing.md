# Contributing

This page covers local setup, checks, and the boundaries to keep in mind when changing SimDeck.

## Setup

Requirements:

- macOS with Xcode and iOS Simulator runtimes.
- Node.js 18 or newer.
- Rust stable.
- x264 (`brew install x264`).
- Optional Android SDK tools for Android emulator work.

```sh
git clone https://github.com/NativeScript/SimDeck.git
cd SimDeck
npm install
npm run build
```

Run the built CLI:

```sh
./build/simdeck
```

Run the development server:

```sh
npm run dev
```

The server log is written to `build/cli.log`.

## Repository Map

| Folder      | Purpose                                                |
| ----------- | ------------------------------------------------------ |
| `server/`   | CLI entrypoint, daemon, API, stream transport, metrics |
| `cli/`      | macOS simulator bridge                                 |
| `client/`   | Browser UI                                             |
| `packages/` | Inspectors, VS Code extension, and `simdeck/test`      |
| `scripts/`  | Build, package, and test helpers                       |
| `docs/`     | VitePress documentation                                |

## Working Rules

- Keep simulator-native logic in `cli/`.
- Keep server behavior in `server/`.
- Keep browser presentation in `client/`.
- Keep runtime inspector logic in the matching `packages/*-inspector` package.
- Prefer a stable CLI command or API route over hidden environment behavior.
- Update docs when changing CLI flags, API routes, stream behavior, or inspector methods.

## Build And Check

```sh
npm run format
npm run lint
npm run test
npm run ci
```

What `npm run ci` covers:

1. Formatting and lint checks.
2. Full build.
3. Rust and client tests.
4. VS Code extension package.

## Integration Tests

iOS:

```sh
npm run build:cli
npm run build:client
npm run test:integration:cli
```

Verbose iOS run:

```sh
npm run test:integration:cli:verbose
```

Android:

```sh
npm run build:cli
npm run build:simdeck-test
npm run test:integration:android
```

Useful variables:

| Variable                                 | Purpose                         |
| ---------------------------------------- | ------------------------------- |
| `SIMDECK_INTEGRATION_VERBOSE=1`          | Print more detail               |
| `SIMDECK_INTEGRATION_SHOW_SIMULATOR=1`   | Show Simulator.app during tests |
| `SIMDECK_INTEGRATION_KEEP_SIMULATOR=1`   | Keep temporary iOS simulator    |
| `SIMDECK_INTEGRATION_ANDROID_AVD=<name>` | Pick an Android AVD             |
| `SIMDECK_INTEGRATION_BOOT_ANDROID=1`     | Boot Android locally            |

## Docs

```sh
npm run docs:dev
npm run docs:build
```

The docs site lives under `docs/` and deploys to GitHub Pages from `.github/workflows/docs.yml`.

## Issues And PRs

For simulator or stream bugs, include:

- Reproduction steps.
- `simdeck --version`.
- macOS and Xcode versions.
- Foreground daemon output or `build/cli.log`.
- Any relevant screenshots or CLI output.

## License

SimDeck is licensed under Apache-2.0. Contributions are licensed under the same terms.
