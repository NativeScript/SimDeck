# Installation

Simdeck ships as a single npm package that contains the launcher, the client bundle, and a postinstall hook that compiles the native CLI on macOS.

## Prerequisites

Simdeck only runs on macOS. The native bridge links private `CoreSimulator` and `SimulatorKit` frameworks, so it cannot run on Linux or Windows.

| Requirement                        | Why                                                                                  |
| ---------------------------------- | ------------------------------------------------------------------------------------ |
| **macOS 13+**                      | Required for current `CoreSimulator` and Apple's HEVC hardware encoder.              |
| **Xcode + iOS Simulator runtimes** | The native bridge invokes `xcrun simctl` and the Simulator app.                      |
| **Node.js ≥ 18**                   | The launcher (`bin/xcode-canvas-web.mjs`) and the bundled client tooling.            |
| **Rust (stable)**                  | Required only when building from source. Installed via [rustup](https://rustup.rs/). |

The package is published as `darwin`-only via the `os` field, so `npm install` on Linux will succeed but skip the native build with a warning.

## Install from npm

The fastest path is the published CLI:

```sh
npm install -g xcode-canvas-web
```

This installs the launcher to your global `node_modules` and runs the native build automatically. After it finishes:

```sh
xcode-canvas-web --help
```

::: tip Future package name
Simdeck is being renamed. The npm package will move from `xcode-canvas-web` to a new name once the rename lands; the install command will switch to:

```sh
npm install -g simdeck
```

Until then, the documentation refers to the product as **Simdeck** and the package/binary as `xcode-canvas-web`.
:::

## Install from source

Clone the repo and install dependencies:

```sh
git clone https://github.com/DjDeveloperr/xcode-canvas-web.git
cd xcode-canvas-web
npm install
```

The root `npm install` triggers `scripts/npm-postinstall.mjs`, which compiles the native CLI in release mode via `scripts/build-cli.sh`. When it finishes, the binary lives at:

```text
build/xcode-canvas-web
build/xcode-canvas-web-bin
```

You can then run the local checkout directly:

```sh
./build/xcode-canvas-web serve --port 4310
```

Or install the local checkout globally:

```sh
npm install -g .
```

After a global install you can call `xcode-canvas-web` from anywhere.

## Build the React client

The client bundle ships pre-built when installed from npm. When working from source, build it explicitly:

```sh
./scripts/build-client.sh
```

This calls `npm install` and `npm run build` inside the `client/` workspace and writes the production bundle to `client/dist`. The Rust server serves that bundle at the HTTP root.

## Build everything

The root `package.json` exposes a one-shot build that compiles every component:

```sh
npm run build
```

This runs:

- `npm run build:cli` — Rust server + Objective-C bridge → `build/xcode-canvas-web`
- `npm run build:client` — Vite production build → `client/dist`
- `npm run build:nativescript-inspector` — TypeScript build of the NativeScript inspector

You can also run any one of those scripts on its own.

## Update or uninstall

To update from npm:

```sh
npm install -g xcode-canvas-web@latest
```

To remove the global install:

```sh
npm uninstall -g xcode-canvas-web
```

If you enabled the [background service](/guide/service), disable it first so launchd does not restart a deleted binary:

```sh
xcode-canvas-web service off
```
