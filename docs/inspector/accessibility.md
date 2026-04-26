# Accessibility

SimDeck's universal fallback inspector uses the private iOS Simulator accessibility APIs. It works against any simulator app — no SDK linking required — but only reports what the system accessibility stack exposes.

## Coverage

It reports anything the app publishes through the accessibility tree:

- `AXLabel`, `AXIdentifier`, `AXValue`, `help` (hint).
- `bounds` and `frameInScreen` in UIKit screen points.
- Role, sub-role, and the small set of custom accessibility actions a view exposes.

It does **not** see:

- SwiftUI value-tree internals.
- NativeScript logical tree nodes.
- UIView properties that aren't part of the accessibility surface.

For those, you need to link the [Swift in-app agent](/inspector/swift) or use the [NativeScript runtime inspector](/inspector/nativescript).

## When AX is the right call

- You're inspecting an app you do not control (a system app, a third-party binary).
- You only need to find an element by label or ID and tap it.
- You're building accessibility QA workflows where the accessibility surface is the actual surface you care about.

## Limitations and gotchas

- **Foreground app only.** Like the iOS accessibility stack, AX snapshot sees the foreground app at the time of the call. If you switch apps mid-call, the snapshot may straddle two processes.
- **Coordinates are in UIKit points.** Multiply by `displayScale` (from the inspector metadata or the simulator's device profile) when correlating with pixel-space frames.

## Combining with in-app inspectors

When both AX and an in-app inspector are available, the response includes both in `availableSources`. The browser client offers a source selector so you can compare the trees side by side. This is especially useful when you suspect a NativeScript element is not making it into the accessibility tree the way you expect.
