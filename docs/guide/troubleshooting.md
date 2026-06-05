# Troubleshooting

Use this page when SimDeck does not start, cannot see a device, shows a bad stream, or falls back to the wrong inspector.

## First checks

```sh
simdeck --version
xcode-select -p
simdeck service status
simdeck list
```

If the background service may be stale:

```sh
simdeck service stop
simdeck
```

## Server will not start

### Port is already in use

```text
bind HTTP listener on 127.0.0.1:4310
```

Use another port:

```sh
simdeck -p 4320 --open
```

Or find the listener:

```sh
lsof -nP -iTCP:4310 -sTCP:LISTEN
```

If it is an old service:

```sh
simdeck service stop
```

### Native binary is missing

Reinstall from npm:

```sh
npm install -g simdeck@latest
```

From a source checkout:

```sh
npm run build:cli
```

### Source build fails

Check the common prerequisites:

```sh
xcode-select --install
rustc --version
node --version
```

Builds must run on macOS because SimDeck links macOS simulator frameworks.

## Device does not boot or list

### `simdeck list` hangs or returns stale data

Restart Apple's simulator service:

```sh
simdeck core-simulator restart
simdeck list
```

### Wrong Xcode is selected

```sh
sudo xcode-select -s /Applications/Xcode.app
simdeck list
```

Or run one command with an explicit developer directory:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer simdeck list
```

### Android emulator is missing

Confirm Android SDK tools are on `PATH`:

```sh
adb devices
emulator -list-avds
```

Android IDs in SimDeck use `android:<avd-name>`.

## Stream is black or stuck

### Timed out waiting for the first frame

Try software encoding:

```sh
simdeck service restart --video-codec software
```

For CI or virtualized Macs:

```sh
simdeck service restart --video-codec software --stream-quality ci-software
```

### Stream stutters or refreshes repeatedly

Lower the quality:

```sh
simdeck service restart --stream-quality low
```

Check metrics:

```sh
curl http://127.0.0.1:4310/api/metrics
```

If `frames_dropped_server` keeps climbing, the client or network cannot keep up. Move closer to the host, reduce quality, or switch to software encoding.

For Android emulator streams, SimDeck uses the emulator `-share-vid` shared display surface. If Android video never starts, confirm `adb devices` shows the emulator as `device`, that it has fully booted, and that externally launched emulators were started with `-share-vid`. SimDeck-owned Android boots add the flag automatically and default to `--android-gpu host`; try `simdeck service restart --android-gpu auto` or `--android-gpu swiftshader_indirect` only when host rendering is unstable.

### Browser cannot establish WebRTC

Use `?stream=webrtc` while testing to make transport selection explicit:

```text
http://127.0.0.1:4310?stream=webrtc
```

For routed remote sessions, configure TURN as described in [Video and streaming](/guide/video#remote-browsers).

## Inspector looks wrong

### `describe` returns accessibility instead of framework data

The fallback is expected when no in-app inspector is available. Check:

- The app with the inspector is foregrounded.
- The app was built in debug mode.
- The inspector package starts before the app UI boots.
- The app is pointing at the active SimDeck port.

Use a forced source to see the failure reason:

```sh
simdeck describe --source nativescript
simdeck describe --source react-native
simdeck describe --source flutter
simdeck describe --source uikit
```

### NativeScript inspector does not connect

- Call `startSimDeckInspector({ port: 4310 })` before bootstrap.
- For Angular, call it before `runNativeScriptAngularApp(...)`.
- Confirm the simulator app can reach `http://127.0.0.1:4310/api/health`.

### React Native source locations are missing

Use a development build. Production bundles usually strip React debug source metadata.

### Flutter source locations are missing

Run a debug build with widget creation tracking. Flutter enables this by default for normal debug runs.

## LAN browser cannot connect

Start SimDeck with a LAN bind and reachable advertised host:

```sh
simdeck --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
```

For native iOS pairing, prefer:

```sh
simdeck pair
```

Then check:

- The remote browser opens `http://192.168.1.50:4310`.
- macOS Firewall allows the port.
- The pairing code matches the current service.
- API scripts send the service token.

See [LAN access](/guide/lan-access).

## Logs to include in issues

Include:

- `simdeck --version`
- macOS version
- Xcode version
- The command you ran
- Service output, or `build/cli.log` when using `npm run dev`
- `simdeck service status` without sharing the token publicly
