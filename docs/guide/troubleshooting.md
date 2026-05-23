# Troubleshooting

Use this page when SimDeck does not start, cannot see a device, shows a bad stream, or falls back to the wrong inspector.

## First Checks

```sh
simdeck --version
xcode-select -p
simdeck daemon status
simdeck list
```

If a background daemon may be stale:

```sh
simdeck daemon stop
simdeck
```

## Server Will Not Start

### Port is already in use

```text
bind HTTP listener on 127.0.0.1:4311
```

Use another port:

```sh
simdeck ui --port 4320 --open
```

Or find the listener:

```sh
lsof -nP -iTCP:4311 -sTCP:LISTEN
```

If it is an old project daemon:

```sh
simdeck daemon stop
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

## Device Does Not Boot Or List

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

## Stream Is Black Or Stuck

### Timed out waiting for the first frame

Try software encoding:

```sh
simdeck daemon restart --video-codec software
```

For CI or virtualized Macs:

```sh
simdeck daemon restart --video-codec software --stream-quality ci-software
```

### Stream stutters or refreshes repeatedly

Lower the quality:

```sh
simdeck daemon restart --stream-quality low
```

Check metrics:

```sh
curl http://127.0.0.1:4310/api/metrics
```

If `frames_dropped_server` keeps climbing, the client or network cannot keep up. Move closer to the host, reduce quality, or switch to software encoding.

### Browser cannot establish WebRTC

Force the H.264 WebSocket fallback while testing:

```text
http://127.0.0.1:4310?stream=h264
```

For routed remote sessions, configure TURN as described in [Video & Streaming](/guide/video#remote-browsers).

## Inspector Looks Wrong

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

## LAN Browser Cannot Connect

Start SimDeck with a LAN bind and reachable advertised host:

```sh
simdeck ui --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
```

For native iOS pairing, prefer:

```sh
simdeck pair
```

Then check:

- The remote browser opens `http://192.168.1.50:4310`.
- macOS Firewall allows the port.
- The pairing code matches the current daemon or global service.
- API scripts send the daemon or service token.

See [LAN Access](/guide/lan-access).

## Logs To Include In Issues

Include:

- `simdeck --version`
- macOS version
- Xcode version
- The command you ran
- Foreground daemon output, or `build/cli.log` when using `npm run dev`
- `simdeck daemon status` without sharing the token publicly
