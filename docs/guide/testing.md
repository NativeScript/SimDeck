# Testing

SimDeck supports two testing workflows:

- App-level JS/TS automation through `simdeck/test`.
- Repository integration tests that drive real simulators and emulators.

## App Tests With `simdeck/test`

```ts
import { connect } from "simdeck/test";

const udid = process.env.SIMDECK_UDID!;
const sim = await connect({ udid });

try {
  await sim.launch("com.example.App");
  await sim.tap(0.5, 0.5);
  await sim.waitFor({ label: "Continue" });
  await sim.screenshot();
  await sim.screenshot({ withBezel: true });
  await sim.record({ seconds: 5 });
} finally {
  sim.close();
}
```

`connect()` starts the project daemon if needed, reuses a healthy daemon, and only stops daemons it started itself. Pass `udid` to `connect()` to make it the default for session methods; methods still accept an explicit UDID as their first argument. Use `sim.device("<other-udid>")` to create a session bound to another simulator.
State-query helpers default to `source: "native-ax"` so agent loops use the
fast universal accessibility path. Pass `source: "auto"` when a test
intentionally wants connected NativeScript, React Native, Flutter, SwiftUI, or
UIKit inspector trees before the native accessibility fallback.

## Useful Test Methods

| Method                                          | Purpose                        |
| ----------------------------------------------- | ------------------------------ |
| `list()`                                        | Device inventory               |
| `boot()`, `shutdown()`, `erase()`               | Device lifecycle               |
| `install()`, `uninstall()`, `launch()`          | App lifecycle                  |
| `openUrl()`                                     | Universal links and deep links |
| `tap()`, `tapElement()`, `swipe()`, `gesture()` | UI input                       |
| `typeText()`, `key()`, `keySequence()`          | Text and keyboard input        |
| `button()`, `home()`, `back()`, `appSwitcher()` | System controls                |
| `tree()`, `query()`, `waitFor()`, `assert()`    | UI state checks                |
| `waitForNot()`, `assertNot()`                   | Negative UI state checks       |
| `scrollUntilVisible()`                          | Scroll until a selector exists |
| `screenshot()`, `record()`, `logs()`            | Evidence capture               |
| `batch()`                                       | Multi-step actions             |

Selectors can match `text`, `id`, `label`, `value`, `type`, `index`, `enabled`, `checked`, `focused`, or `selected`. Set `regex: true` to treat string selector fields as regular expressions.

## Maestro-Compatible YAML

The CLI includes a compatibility runner for common Maestro YAML flows:

```sh
simdeck use <udid>
simdeck maestro test flow.yaml --artifacts-dir artifacts/maestro
```

Supported commands include `launchApp`, `openLink`, `tapOn`, `inputText`, `eraseText`, `pressKey`, `assertVisible`, `assertNotVisible`, `scrollUntilVisible`, `swipe`, `takeScreenshot`, and `waitForAnimationToEnd`. Unsupported Maestro commands fail clearly so the flow can be adjusted or the compatibility layer can be expanded.

## Repository Tests

Normal unit and client tests:

```sh
npm run test
```

iOS integration test:

```sh
npm run build:cli
npm run build:client
npm run test:integration:fixture
npm run test:integration:cli
```

Verbose iOS run:

```sh
npm run test:integration:cli:verbose
```

Android integration test:

```sh
npm run build:cli
npm run build:simdeck-test
npm run test:integration:android
```

Android tests require the Android SDK and a running or bootable AVD.

## Agent Control Benchmarks

Compare SimDeck against agent-device and Argent on a booted iOS simulator:

```sh
npm run bench:agent-control -- --reps 3
```

Pass `--udid <udid>` to pin a simulator and `--out-dir <path>` to choose where
the JSON and Markdown reports are written. The benchmark measures cold tool
startup plus hot command latency for common agent actions: listing devices,
launching Settings, opening a URL, describing the AX tree, waiting, tapping,
back navigation, swiping, screenshots, home, and a short tap/back batch flow.
Setup/reset work is excluded from action timings.

## Helpful Environment Variables

| Variable                                        | Purpose                                              |
| ----------------------------------------------- | ---------------------------------------------------- |
| `SIMDECK_INTEGRATION_VERBOSE=1`                 | Print commands, outputs, and timings                 |
| `SIMDECK_INTEGRATION_SHOW_SIMULATOR=1`          | Open Simulator.app during iOS tests                  |
| `SIMDECK_INTEGRATION_KEEP_SIMULATOR=1`          | Keep the temporary iOS simulator                     |
| `SIMDECK_INTEGRATION_TRACE_HTTP=1`              | Print HTTP request logs                              |
| `SIMDECK_INTEGRATION_ANDROID_AVD=<name>`        | Pick an Android AVD                                  |
| `SIMDECK_INTEGRATION_BOOT_ANDROID=1`            | Let SimDeck boot the Android emulator                |
| `SIMDECK_INTEGRATION_REQUIRE_RUNNING_ANDROID=1` | Fail instead of skipping when Android is unavailable |

## Stress Test A Running Daemon

```sh
npm run test:stress -- --server-url http://127.0.0.1:4310 --iterations 1000 --concurrency 12
```

Include simulator refresh traffic:

```sh
npm run test:stress -- --udid <udid> --iterations 2000 --concurrency 16
```

## Stress Test Daemon Cleanup

```sh
npm run build:cli
npm run test:stress:daemon -- --iterations 30 --concurrency 3
```

This starts isolated temporary project daemons, hits health and metrics, stops
them through the CLI, and verifies the process group, listener port, and daemon
status are cleaned up. Use `--binary /path/to/simdeck` to test an installed or
packaged binary.
