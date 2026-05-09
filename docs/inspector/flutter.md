# Flutter Runtime Inspector

`simdeck_flutter_inspector` is a debug-only Flutter iOS plugin that publishes the live Flutter widget hierarchy to SimDeck. It uses the same [Inspector Protocol](/api/inspector-protocol) as the Swift, NativeScript, and React Native inspectors, and connects outbound to the SimDeck server over WebSocket.

The package source lives at `packages/flutter-inspector/` in this repo.

## Install

```sh
flutter pub add simdeck_flutter_inspector
```

## Start the inspector

Start the inspector during app startup in debug builds:

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

`port` is the SimDeck server port. Flutter apps do not need to choose a local inspector port.

## What it exposes

`View.getHierarchy` returns the Flutter widget tree. Each node may include:

- `type` and `displayName` — the widget runtime type.
- `title` — derived from semantics labels, common widget diagnostics, text, tooltip, value, or key.
- `frame` and `frameInScreen` — RenderObject bounds in logical screen points.
- `flutter` — widget, element, state type, key, and depth metadata.
- `semantics` — label, value, hint, identifier, role, flags, and supported semantics actions.
- `sourceLocation` — file, line, and column from Flutter widget creation tracking.

Source locations require debug builds with `--track-widget-creation`, which Flutter enables by default for debug runs.

## Debug actions

The Flutter runtime supports `View.getProperties`, `View.listActions`, and `View.perform`.

`View.perform` is best-effort and uses Flutter's own runtime APIs:

- `tap`, `longPress`, `increase`, and `decrease` dispatch matching semantics actions.
- `focus` and `resignFirstResponder` use Flutter focus scopes.
- `setText` updates the nearest `EditableText`.
- `scrollBy` and `scrollTo` drive the nearest `Scrollable`.

Flutter widgets are immutable, so `View.setProperty` returns an unsupported-method error. Runtime interactions should go through `View.perform`, while persistent visual changes should still be made in app source.
