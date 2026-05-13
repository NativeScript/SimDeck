# Inspectors

SimDeck can show what an app is rendering, not just the pixels on screen.

Use the built-in accessibility fallback for any app. Add an in-app inspector when you want framework-level trees, source locations, and debug actions.

## Sources

| Source        | Best for                                      | Setup                                     |
| ------------- | --------------------------------------------- | ----------------------------------------- |
| Accessibility | Any app                                       | None                                      |
| Swift agent   | UIKit and SwiftUI apps you control            | Add the Swift package in debug builds     |
| NativeScript  | NativeScript apps                             | Install `@nativescript/simdeck-inspector` |
| React Native  | React Native apps                             | Import `react-native-simdeck/auto`        |
| Flutter       | Flutter apps                                  | Start `simdeck_flutter_inspector`         |
| DevTools      | WebKit, Metro, Chrome Inspector, app runtimes | Use the browser DevTools panel            |

## Use From The CLI

```sh
simdeck describe <udid>
simdeck describe <udid> --format agent --max-depth 3
simdeck describe <udid> --source native-ax
simdeck describe <udid> --source react-native
```

`auto` source selection uses the best available source and falls back to accessibility.

## Use From The Browser

Open the SimDeck UI and select a device. The inspector pane shows the active tree source and any fallback reason. When multiple sources are available, switch sources from the inspector controls.

The DevTools panel can open:

- Safari and inspectable `WKWebView` targets.
- React Native Metro targets.
- Local Chrome Inspector targets.
- Connected app runtime inspector targets.

For app-owned `WKWebView` on iOS 16.4 or newer, set `isInspectable = true`.

## Choosing A Source

- Use **Accessibility** when you cannot modify the app or when accessibility is the thing you are testing.
- Use **Swift** for UIKit properties, SwiftUI publishing, or debug view edits.
- Use **NativeScript**, **React Native**, or **Flutter** when you want framework names and source locations.
- Use **DevTools** for web content, Metro debugging, console evaluation, or familiar browser inspection.

## Troubleshooting

If SimDeck falls back to accessibility:

1. Make sure the app is foregrounded.
2. Confirm the inspector starts in debug builds.
3. Confirm the app points at the active SimDeck port.
4. Force the source from the CLI to see the fallback reason:

   ```sh
   simdeck describe <udid> --source react-native
   ```

See [Troubleshooting](/guide/troubleshooting#inspector-looks-wrong).
