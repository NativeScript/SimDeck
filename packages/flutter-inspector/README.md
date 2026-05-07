# Flutter Inspector

`simdeck_flutter_inspector` is a debug-only Flutter iOS runtime inspector for SimDeck. It connects to the SimDeck server over WebSocket and publishes the live Flutter widget hierarchy with render bounds, diagnostics properties, semantics actions, and source locations when Flutter widget creation tracking is enabled.

## Install

Add the package to your Flutter app:

```sh
flutter pub add simdeck_flutter_inspector
```

## Start

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

`port` is the SimDeck server port. The Flutter app connects outbound to:

```text
ws://127.0.0.1:4310/api/inspector/connect
```

## What It Exposes

- Flutter widget hierarchy rooted at `WidgetsBinding.instance.rootElement`.
- RenderObject screen frames in logical screen points.
- Widget diagnostics properties and state type metadata.
- Semantics labels, values, hints, identifiers, flags, roles, and actions.
- Source locations from Flutter's widget creation tracking in debug builds.
- `View.hitTest`, `View.describeAtPoint`, `View.getProperties`, `View.listActions`, and `View.perform`.

`View.perform` supports best-effort `tap`, `longPress`, `focus`, `resignFirstResponder`, `setText`, `scrollBy`, `scrollTo`, `increase`, and `decrease` actions when the selected widget exposes the matching Flutter semantics, text, focus, or scroll API.

Flutter widgets are immutable, so `View.setProperty` intentionally returns an unsupported-method error. Use `View.perform` for runtime-safe text and interaction changes.
