# Testing

SimDeck supports two testing workflows:

- App-level JS/TS automation through `simdeck/test`.
- Repository integration tests that drive real simulators and emulators.

## App Tests With `simdeck/test`

```ts
import { connect } from "simdeck/test";

const sim = await connect();

try {
  const { simulators } = await sim.list();
  const udid = simulators.find((device) => device.isBooted)?.udid;

  await sim.launch(udid, "com.example.App");
  await sim.tap(udid, 0.5, 0.5);
  await sim.waitFor(udid, { label: "Continue" });
  await sim.screenshot(udid);
} finally {
  sim.close();
}
```

`connect()` starts the project daemon if needed, reuses a healthy daemon, and only stops daemons it started itself.

## Useful Test Methods

| Method                                          | Purpose                        |
| ----------------------------------------------- | ------------------------------ |
| `list()`                                        | Device inventory               |
| `boot()`, `shutdown()`, `erase()`               | Device lifecycle               |
| `install()`, `uninstall()`, `launch()`          | App lifecycle                  |
| `openUrl()`                                     | Universal links and deep links |
| `tap()`, `tapElement()`, `swipe()`, `gesture()` | UI input                       |
| `typeText()`, `key()`, `keySequence()`          | Text and keyboard input        |
| `button()`, `home()`, `appSwitcher()`           | System controls                |
| `tree()`, `query()`, `waitFor()`, `assert()`    | UI state checks                |
| `screenshot()`, `logs()`                        | Evidence capture               |
| `batch()`                                       | Multi-step actions             |

Selectors can match `id`, `label`, `value`, or `type`.

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
