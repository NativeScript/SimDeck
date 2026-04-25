# Accessibility (AXe)

SimDeck's universal fallback inspector uses [AXe](https://github.com/cameroncooke/AXe), an open-source CLI that wraps the iOS Simulator accessibility APIs. It works against any simulator app — no SDK linking required — but only reports what the system accessibility stack exposes.

## Install AXe

Follow the AXe project's install instructions. The SimDeck server expects an `axe` binary on `PATH`:

```sh
which axe
axe --help
```

If `axe` isn't installed, `GET /api/simulators/{udid}/accessibility-tree` falls back to whichever in-app inspector is reachable. With no in-app inspector connected either, the request fails with a clear error.

## What it does

The server shells out to:

```sh
axe describe-ui --udid <udid>
```

…and parses the JSON response. The result is normalised into the same `roots[]` shape as the in-app inspectors, with `"source": "axe"`.

For point queries:

```sh
axe describe-ui --udid <udid> --point <x>,<y>
```

…which the browser client uses for the "describe element under cursor" affordance.

## Coverage

AXe reports anything UIKit publishes through the accessibility tree:

- `AXLabel`, `AXIdentifier`, `AXValue`, `help` (hint).
- `bounds` and `frameInScreen` in UIKit screen points.
- Role, sub-role, and the small set of custom accessibility actions a view exposes.

It does **not** see:

- SwiftUI value-tree internals.
- NativeScript logical tree nodes.
- UIView properties that aren't part of the accessibility surface.

For those, you need to link the [Swift in-app agent](/inspector/swift) or use the [NativeScript runtime inspector](/inspector/nativescript).

## When AXe is the right call

- You're inspecting an app you do not control (a system app, a third-party binary).
- You only need to find an element by label or ID and tap it.
- You're building accessibility QA workflows where the accessibility surface is the actual surface you care about.

## Limitations and gotchas

- **AXe is a separate process.** Each call spawns `axe` with an 8 second timeout. Repeated calls incur per-process startup cost.
- **Foreground app only.** Like the iOS accessibility stack, AXe sees the foreground app at the time of the call. If you switch apps mid-call, the snapshot may straddle two processes.
- **Coordinates are in UIKit points.** Multiply by `displayScale` (from the inspector metadata or the simulator's device profile) when correlating with pixel-space frames.
- **Errors surface as plain text.** Failures are returned as JSON `{"error":{"message":"..."}}` from the SimDeck endpoint with HTTP `500`. The original AXe stderr is included verbatim.

## Combining with in-app inspectors

When both AXe and an in-app inspector are available, the response includes both in `availableSources`. The browser client offers a source selector so you can compare the trees side by side. This is especially useful when you suspect a NativeScript element is not making it into the accessibility tree the way you expect.
