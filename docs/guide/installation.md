# Install

## Requirements

- macOS on Apple Silicon.
- Xcode with the simulator runtimes you want to use.
- Node.js 18 or newer.
- Optional Android SDK tools for Android emulator support.
- Optional Rust stable when building from source.

Check Xcode selection if you have multiple installs:

```sh
xcode-select -p
```

## Install From npm

Use `npx` for a quick try:

```sh
npx simdeck
```

Install globally for regular use:

```sh
npm install -g simdeck@latest
simdeck
```

The package installs the launcher, browser client, and native binary.

## Install The Codex Skill

For Codex workflows, install the SimDeck skill so agents use the stable commands:

```sh
npx skills add NativeScript/SimDeck --skill simdeck -a codex -g
```

Restart Codex after installing the skill.

## VS Code

Install the VS Code extension if you want the simulator inside an editor panel:

```sh
npm run package:vscode
npm run install:vscode
```

From the marketplace, use `nativescript.simdeck-vscode` when available.

## Build From Source

```sh
git clone https://github.com/NativeScript/SimDeck.git
cd SimDeck
npm install
npm run build
./build/simdeck
```

Useful build scripts:

| Script                 | What it builds                            |
| ---------------------- | ----------------------------------------- |
| `npm run build`        | Native CLI and browser client             |
| `npm run build:cli`    | Native CLI only                           |
| `npm run build:client` | Browser client only                       |
| `npm run build:all`    | CLI, client, inspectors, and test package |
| `npm run build:docs`   | Documentation site                        |

## Update Or Remove

```sh
npm install -g simdeck@latest
npm uninstall -g simdeck
```

Stop a running daemon before uninstalling:

```sh
simdeck daemon stop
```
