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

| Method         | Purpose                                                           |
| -------------- | ----------------------------------------------------------------- |
| `list()`       | Fetch simulator inventory from `GET /api/simulators`.             |
| `launch()`     | Launch an installed bundle ID.                                    |
| `openUrl()`    | Open a URL or deep link.                                          |
| `tap()`        | Tap normalized screen coordinates.                                |
| `key()`        | Send one HID key code.                                            |
| `button()`     | Press a hardware button.                                          |
| `tree()`       | Fetch an accessibility hierarchy.                                 |
| `query()`      | Return compact matches for a selector.                            |
| `waitFor()`    | Poll until a selector appears.                                    |
| `assert()`     | Assert a selector is present.                                     |
| `batch()`      | Run multiple REST actions through `/api/simulators/{udid}/batch`. |
| `screenshot()` | Return a PNG buffer.                                              |
| `close()`      | Stop the daemon if this session started it.                       |

Selectors can match `id`, `label`, `value`, or `type`. Query options accept `source`, `maxDepth`, and `includeHidden`.

## Repository Integration Suite

The repo includes a macOS-only integration runner that creates a temporary simulator, builds and installs a small SwiftUI fixture app, then sweeps the CLI and REST control surface.

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

| Variable                               | Purpose                                               |
| -------------------------------------- | ----------------------------------------------------- |
| `SIMDECK_INTEGRATION_VERBOSE=1`        | Print commands, outputs, timings, and UI checkpoints. |
| `SIMDECK_INTEGRATION_TRACE_HTTP=1`     | Print raw HTTP request logs.                          |
| `SIMDECK_INTEGRATION_SHOW_SIMULATOR=1` | Open Simulator.app during the run.                    |
| `SIMDECK_INTEGRATION_KEEP_SIMULATOR=1` | Leave the temporary simulator after exit.             |

The integration suite is separate from `npm run test` because it boots and drives a real iOS simulator.
The SwiftUI fixture app is cached under `.cache/simdeck/fixture` using a hash
of its generated source, plist, simulator SDK, Swift compiler, and host
architecture.

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
