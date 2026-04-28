# Installation

SimDeck ships as a single npm package that contains the launcher, the client bundle, and the native CLI binary.

## Prerequisites

SimDeck only runs on macOS. The native bridge links private `CoreSimulator` and `SimulatorKit` frameworks, so it cannot run on Linux or Windows.

| Requirement                        | Why                                                                                  |
| ---------------------------------- | ------------------------------------------------------------------------------------ |
| **macOS 13+**                      | Required for current `CoreSimulator` and Apple's HEVC hardware encoder.              |
| **Xcode + iOS Simulator runtimes** | The native bridge invokes `xcrun simctl` and the Simulator app.                      |
| **Node.js ≥ 18**                   | The launcher (`bin/simdeck.mjs`) and the bundled client tooling.                     |
| **Rust (stable)**                  | Required only when building from source. Installed via [rustup](https://rustup.rs/). |

The package contains a macOS native binary. Non-macOS installs are unsupported.

## Install from npm

The fastest path is the published CLI:

```sh
npm install -g simdeck
```

This installs the launcher and bundled native binary to your global `node_modules`. After it finishes:

```sh
simdeck --help
```

The global install prints the next setup steps:

```sh
simdeck ui --open
npx skills add NativeScript/SimDeck --skill simdeck -a codex -g
simdeck service on
```

Install the `nativescript.simdeck` VS Code extension if you want the simulator
view inside VS Code.

`simdeck service on` is recommended when agents or editor integrations should be
able to reach SimDeck any time after login. It installs a localhost macOS
LaunchAgent and can be removed with `simdeck service off`.

## Install the Codex skill

SimDeck includes an agent skill at `skills/simdeck/SKILL.md`. Install it with
[skills.sh](https://skills.sh/) so Codex can choose the right commands and
inspection loops automatically:

```sh
npx skills add NativeScript/SimDeck --skill simdeck -a codex -g
```

Restart Codex after installing the skill.

## Install from source

Clone the repo and install dependencies:

```sh
git clone https://github.com/NativeScript/SimDeck.git
cd simdeck
npm install
```

Build the source checkout before running it directly or installing it globally:

```sh
npm run build
```

The native CLI build writes:

```text
build/simdeck
build/simdeck-bin
```

You can then run the local checkout directly:

```sh
./build/simdeck ui --open
```

Or install the local checkout globally:

```sh
npm install -g .
```

After a global install you can call `simdeck` from anywhere.

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

- `npm run build:cli` — Rust server + Objective-C bridge → `build/simdeck`
- `npm run build:client` — Vite production build → `client/dist`
- `npm run build:nativescript-inspector` — TypeScript build of the NativeScript inspector
- `npm run build:react-native-inspector` — TypeScript build of the React Native inspector
- `npm run build:simdeck-test` — TypeScript build of `simdeck/test`

You can also run any one of those scripts on its own.

## Update or uninstall

To update from npm:

```sh
npm install -g simdeck@latest
```

To remove the global install:

```sh
npm uninstall -g simdeck
```

If a project daemon is running, stop it before uninstalling so your shell does not keep talking to the old binary:

```sh
simdeck daemon stop
```
