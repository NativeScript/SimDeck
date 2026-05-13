# Flutter Inspector

`simdeck_flutter_inspector` publishes a Flutter widget tree to SimDeck in debug builds.

Use it when accessibility does not show enough widget, render, semantics, or source-location data.

## Install

```sh
flutter pub add simdeck_flutter_inspector
```

## Start It

```dart
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:simdeck_flutter_inspector/simdeck_flutter_inspector.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();

  if (kDebugMode) {
    startSimDeckFlutterInspector(port: 4310);
  }

  runApp(const App());
}
```

`port` is the SimDeck server port.

## Inspect

```sh
simdeck describe <udid> --source flutter --format agent
```

Nodes may include:

- Widget, element, and state type.
- Text, keys, tooltips, labels, values, and semantics roles.
- RenderObject bounds in logical screen points.
- Semantics actions.
- Source locations from Flutter widget creation tracking.

Flutter enables widget creation tracking for normal debug runs.

## Debug Actions

The runtime supports best-effort actions such as:

- `tap`
- `longPress`
- `focus`
- `setText`
- `scrollBy`
- `scrollTo`
- semantics `increase` and `decrease`

Flutter widgets are immutable, so persistent visual changes should still be made in app source.

## Troubleshooting

- Use a debug build.
- Bring the app to the foreground.
- Confirm the app can reach the SimDeck server port.
- Force `--source flutter` to read the fallback reason.
