# Accessibility

Accessibility is SimDeck's universal inspector. It works with any simulator app and requires no app changes.

## What It Shows

- Accessibility label, identifier, value, hint, role, and actions.
- Bounds and frames in screen points.
- Enough structure for selector-based taps, waits, and assertions.

Use it from the CLI:

```sh
simdeck describe <udid> --source native-ax
simdeck tap <udid> --label "Continue" --wait-timeout-ms 5000
```

## What It Cannot Show

- Internal SwiftUI value trees.
- NativeScript, React Native, or Flutter component/widget structure.
- UIKit properties that are not exposed through accessibility.
- Source locations.

Use an in-app inspector when you need those details.

## Good Uses

- Testing labels, identifiers, and accessibility quality.
- Driving apps you do not control.
- Building stable selector-based automation.

## Coordinate Note

Frames are in screen points, not pixels. Use the simulator scale or inspector metadata if you need pixel coordinates.
