# Install

## Requirements

- macOS on Apple Silicon for the full experience.
- Xcode with the simulator runtimes you want to use.
- Node.js 18 or newer.

## Platform support

The npm package installs a native CLI for macOS, Windows, and Linux. Only the
macOS build includes the private simulator bridge, so iOS simulators need a
Mac. Android emulators work everywhere, including the live browser stream.

| Capability                                       | macOS                | Windows / Linux        |
| ------------------------------------------------ | -------------------- | ---------------------- |
| iOS simulator control, inspection, and streaming | Yes                  | No                     |
| Android emulator control and inspection          | Yes                  | Yes                    |
| Android emulator live H.264 browser stream       | VideoToolbox or x264 | OpenH264 software      |
| `--video-codec hardware`                         | VideoToolbox         | Falls back to software |

On Windows and Linux the CLI prints a `Live video:` note after the service
URLs, and `GET /api/health` reports `liveVideo.encoder: "openh264"` with
`liveVideo.iosSimulator: false`. Selecting an iOS simulator through the API
there returns `501` with the same explanation. See
[Video and streaming](./video.md) for the encoder details.

Check Xcode selection if you have multiple installs:

```sh
xcode-select -p
```

## Install from npm

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

## Install the agent skill

Install the SimDeck skill so agents use the stable commands:

```sh
npx skills add NativeScript/SimDeck --skill simdeck -g
```

Restart your agent after installing the skill.

## VS Code

Install the
[SimDeck VS Code extension](https://marketplace.visualstudio.com/items?itemName=NativeScript.simdeck-vscode)
if you want the simulator inside an editor panel.

## Build from source

Source builds also need Rust stable and x264:

```sh
brew install x264
```

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

## Android emulator support

Install the Android SDK command-line tools if you want SimDeck to manage Android
emulators too.

## Update or remove

```sh
npm install -g simdeck@latest
npm uninstall -g simdeck
```

Stop a running service before uninstalling:

```sh
simdeck service stop
```
