# Troubleshooting

Most SimDeck issues fall into one of three buckets: simulator boot, video stream, or accessibility/inspector. This page lists the symptoms and fixes for the ones we hit most often.

## Server won't start

### `bind HTTP listener on 127.0.0.1:4310`

Another process already owns the HTTP port. Pick a different one:

```sh
xcode-canvas-web serve --port 4320
```

Or find what's holding it:

```sh
lsof -nP -iTCP:4310 -sTCP:LISTEN
```

If the holder is an old SimDeck instance, the bundled `npm run dev` script auto-kills stale listeners on `4310` and `4311` for you.

### `xcode-canvas-web is not built yet`

The launcher script could not find the compiled binary. Reinstall the package or run the local build:

```sh
npm install -g xcode-canvas-web
# or, from a checkout:
./scripts/build-cli.sh
```

### Native build fails on install

The postinstall hook runs `cargo build --release` and Apple's Clang against the Objective-C bridge. The most common failures are:

- **Rust missing.** Install via [rustup](https://rustup.rs/), then reinstall.
- **Xcode command-line tools missing.** Run `xcode-select --install`.
- **Sandboxed CI without macOS frameworks.** Postinstall warns and exits cleanly on non-Darwin platforms; the binary just isn't installed.

## Simulator never boots

### `xcrun simctl` errors

The native bridge falls back to `xcrun simctl boot` when private CoreSimulator APIs are unavailable. Try the same command directly to surface the underlying error:

```sh
xcrun simctl boot <udid>
```

If `simctl` succeeds but SimDeck still fails, capture the server log and file an issue.

### CoreSimulator service unhealthy

If `simctl list` itself hangs or returns garbage, the macOS `com.apple.CoreSimulator.CoreSimulatorService` is wedged. Restart it:

```sh
killall -9 com.apple.CoreSimulator.CoreSimulatorService
```

CoreSimulator restarts on demand. Re-run `simctl list` to confirm before retrying.

### Multiple Xcode installs

When more than one Xcode is installed, `xcrun simctl` uses whichever Xcode is selected by `xcode-select`. Pick the one whose runtimes you care about:

```sh
sudo xcode-select -s /Applications/Xcode.app
```

## Stream is black or stuck

### "Timed out waiting for initial simulator keyframe"

The encoder did not produce a keyframe within 3 seconds. The most common causes:

- **VideoToolbox is busy.** macOS screen recording can starve the HEVC encoder. Switch to software H.264:

  ```sh
  xcode-canvas-web serve --port 4310 --video-codec h264-software
  ```

- **The Simulator window is minimised or off-screen.** The private display bridge captures from a headless context, so this is rare, but if you see it after waking from sleep, shut the simulator down and boot it again.
- **The simulator is mid-shutdown.** Wait for `xcode-canvas-web list` to report `isBooted: true`.

### Frequent stutter or "Refresh stream" loops

The transport hub forces a keyframe whenever a client falls behind. If `frames_dropped_server` on `/api/metrics` climbs steadily, the bottleneck is between the encoder and the decoder.

- Bring the client closer (LAN with low latency vs Wi-Fi mesh hops).
- Switch to `h264` instead of `hevc` if the client decoder is slow.
- Check `client_streams` in `/api/metrics`. If `decodedFps` is much lower than `packetFps`, the client decoder is the bottleneck.

## Inspector returns AXe instead of NativeScript / UIKit

The accessibility tree endpoint blends three inspector sources and falls back to AXe when none of the others are reachable. The response includes both a `source` field and a `fallbackReason` field that explains what happened.

Common reasons:

| `fallbackReason`                                                    | Fix                                                                                      |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `The in-app inspector process is not the foreground app.`           | Bring the inspector-enabled app to the foreground.                                       |
| `NativeScript hierarchy is not published by the app.`               | Make sure the app calls `startXcodeCanvasInspector(...)` before bootstrapping.           |
| `No connected NativeScript inspector ...`                           | The NativeScript inspector hasn't completed its WebSocket handshake yet. Reload the app. |
| `No in-app inspector found ... on ports 47370-47402`                | The Swift agent isn't listening; confirm the app links and starts the agent in DEBUG.    |
| `Unable to run \`axe describe-ui\`. Install AXe or ensure on PATH.` | Install AXe; the fallback can't run without it.                                          |

For more on the inspector matrix, see the [Inspector Overview](/inspector/).

## NativeScript inspector won't connect

- Confirm `startXcodeCanvasInspector({ port: 4310 })` runs in the simulator app's main thread before bootstrap.
- Confirm the simulator can reach the host: from inside the app, `fetch('http://127.0.0.1:4310/api/health')` should succeed.
- For Angular apps, make sure `startXcodeCanvasInspector(...)` runs **before** `runNativeScriptAngularApp(...)`.
- Watch the server log for messages such as `Registered NativeScript inspector for process …`. If you don't see one, the WebSocket never completed.

## Logs

When all else fails, capture the server log:

- Foreground server: redirect output to a file.
- Background service: read `~/Library/Logs/xcode-canvas-web.log` and `~/Library/Logs/xcode-canvas-web.err.log`.

Include both files when filing an issue, along with `xcode-canvas-web --version` (when implemented), the macOS version, and the Xcode version.
