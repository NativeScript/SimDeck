# Testing

SimDeck supports two test layers: a small JS/TS client package for app tests, and a simulator-backed integration suite for the CLI and REST API.

## App Tests With `simdeck/test`

`simdeck/test` starts or reuses the project daemon and gives tests a typed API for simulator control:

```ts
import { connect } from "simdeck/test";

const sim = await connect();

try {
  const devices = await sim.list();
  await sim.launch("<udid>", "com.example.App");
  await sim.tap("<udid>", 0.5, 0.5);
  await sim.waitFor("<udid>", { label: "Continue" });
  const png = await sim.screenshot("<udid>");
} finally {
  sim.close();
}
```

`connect()` starts the daemon when needed, reuses it when healthy, and only stops daemons it started itself unless `keepDaemon` is set.

## Session API

The current session object exposes:

| Method                                 | Purpose                                                           |
| -------------------------------------- | ----------------------------------------------------------------- |
| `list()`                               | Fetch simulator inventory from `GET /api/simulators`.             |
| `boot()`, `shutdown()`, `erase()`      | Manage simulator or Android emulator lifecycle.                   |
| `install()`, `uninstall()`             | Install or remove an app.                                         |
| `launch()`                             | Launch an installed bundle ID or Android package.                 |
| `openUrl()`                            | Open a URL or deep link.                                          |
| `tap()`, `tapElement()`                | Tap normalized coordinates or a matching accessibility element.   |
| `touch()`, `swipe()`, `gesture()`      | Send normalized pointer gestures.                                 |
| `typeText()`, `key()`, `keySequence()` | Send text or HID keyboard input.                                  |
| `button()`                             | Press a hardware button.                                          |
| `home()`, `dismissKeyboard()`          | Trigger common system controls.                                   |
| `appSwitcher()`                        | Open the app switcher.                                            |
| `rotateLeft()`, `rotateRight()`        | Rotate the simulator display.                                     |
| `toggleAppearance()`                   | Toggle light/dark appearance.                                     |
| `pasteboardSet()`, `pasteboardGet()`   | Read or write pasteboard text.                                    |
| `chromeProfile()`                      | Fetch screen/chrome geometry.                                     |
| `logs()`                               | Fetch recent simulator or Android log entries.                    |
| `tree()`                               | Fetch an accessibility hierarchy.                                 |
| `query()`                              | Return compact matches for a selector.                            |
| `waitFor()`                            | Poll until a selector appears.                                    |
| `assert()`                             | Assert a selector is present.                                     |
| `batch()`                              | Run multiple REST actions through `/api/simulators/{udid}/batch`. |
| `screenshot()`                         | Return a PNG buffer.                                              |
| `close()`                              | Stop the daemon if this session started it.                       |

Selectors can match `id`, `label`, `value`, or `type`. Query options accept `source`, `maxDepth`, and `includeHidden`.

## Repository Integration Suite

The repo includes simulator-backed integration runners. The iOS runner is
macOS-only; it creates a temporary simulator, builds and installs a small UIKit
fixture app, then sweeps the CLI and REST control surface.

```sh
npm run build:cli
npm run build:client
npm run test:integration:fixture
npm run test:integration:cli
```

Verbose mode opens Simulator.app and prints each step with timing:

```sh
npm run test:integration:cli:verbose
```

Useful environment variables:

| Variable                                | Purpose                                               |
| --------------------------------------- | ----------------------------------------------------- |
| `SIMDECK_INTEGRATION_VERBOSE=1`         | Print commands, outputs, timings, and UI checkpoints. |
| `SIMDECK_INTEGRATION_TRACE_HTTP=1`      | Print raw HTTP request logs.                          |
| `SIMDECK_INTEGRATION_SHOW_SIMULATOR=1`  | Open Simulator.app during the run.                    |
| `SIMDECK_INTEGRATION_KEEP_SIMULATOR=1`  | Leave the temporary simulator after exit.             |
| `SIMDECK_INTEGRATION_SIMCTL_TIMEOUT_MS` | Override the cold CoreSimulator command timeout.      |
| `SIMDECK_INTEGRATION_IOS_RUNTIME`       | Force a runtime by version, name, or identifier.      |
| `SIMDECK_INTEGRATION_DEVICE_TYPE`       | Force an iPhone device type by name or identifier.    |

The integration suite is separate from `npm run test` because it boots and drives a real iOS simulator.
The UIKit fixture app is cached under `.cache/simdeck/fixture` using a hash
of its generated source, plist, simulator SDK, Clang version, and host
architecture.

By default, the integration runner selects the newest available iOS runtime that
does not exceed the active `iphonesimulator` SDK version, falling back to the
same major version when needed. This keeps CI off newer installed runtimes that
do not match the selected Xcode toolchain.

Android coverage is opt-in because it requires a locally installed Android SDK
and at least one existing AVD. It runs on macOS or Linux. On Linux, SimDeck
builds the daemon with a native iOS stub and leaves the Android bridge active.
The runner starts an isolated SimDeck daemon and sweeps the Android CLI and
`simdeck/test` surface for lifecycle, tree, screenshot, pasteboard behavior,
app launch, URL opening, touch/swipe/gesture, keyboard, system buttons,
rotation, appearance, logs, and batch controls:

```sh
npm run build:cli
npm run build:simdeck-test
npm run test:integration:android
```

Set `SIMDECK_INTEGRATION_ANDROID_AVD=<avd-name>` to pick a specific AVD. The
runner expects that emulator to already be booted, which is how the Linux CI job
uses `reactivecircus/android-emulator-runner`. If no AVD is configured, or a
local AVD exists but is not running, the Android runner prints a skip message
and exits successfully. Set `SIMDECK_INTEGRATION_REQUIRE_RUNNING_ANDROID=1` to
turn that skip into a failure. Set `SIMDECK_INTEGRATION_BOOT_ANDROID=1` to let
SimDeck cold-boot a local AVD before the suite. Set
`SIMDECK_INTEGRATION_KEEP_ANDROID=1` to leave an emulator booted when the runner
started it. Set `SIMDECK_INTEGRATION_STEP_TIMEOUT_MS` to override the per-step
timeout.

## Stress and Leak Checks

Use the stress runner against an already-running daemon when you want to shake out
high-usage reliability issues without adding minutes to every PR:

```sh
npm run test:stress -- --server-url http://127.0.0.1:4310 --iterations 1000 --concurrency 12
```

To include simulator-specific refresh traffic and RSS growth checks:

```sh
npm run test:stress -- --udid <udid> --iterations 2000 --concurrency 16 --max-rss-growth-mb 256
```

The runner repeatedly calls health, metrics, simulator listing, stream-quality,
and optional simulator refresh endpoints. It samples the daemon process RSS with
`ps` and fails if the peak or growth limits are exceeded.
