# Browser Client

The browser client is the UI served by `simdeck` at `/`. It is the same surface used in normal browser sessions and inside the VS Code extension.

## Open It

```sh
simdeck
```

Then open the printed local URL.

Detached flow:

```sh
simdeck ui --open
```

LAN flow:

```sh
simdeck ui --bind 0.0.0.0 --advertise-host 192.168.1.50 --open
```

## What The UI Provides

- Device list with boot and selection controls.
- Live video with stream quality controls.
- Pointer, keyboard, and hardware-button input.
- Rotation, home, app switcher, dark-mode toggle, and refresh actions.
- Accessibility and framework inspector panes.
- DevTools panel for supported WebKit, Metro, Chrome, and runtime targets.
- Stream diagnostics.

## Stream URL Options

Force a stream transport while debugging:

```text
http://127.0.0.1:4310?stream=webrtc
http://127.0.0.1:4310?stream=h264
```

Use the default URL for normal operation.

## Serve A Custom Client

Point the daemon at another static bundle:

```sh
simdeck ui --client-root /path/to/dist --open
```

Your client should use the [REST API](/api/rest), WebRTC offer endpoint, and control WebSocket documented in the API reference.

## Develop The Built-In Client

```sh
npm run dev
```

This starts the local SimDeck server and the Vite dev server. Client checks:

```sh
npm run --prefix client typecheck
npm run --prefix client test
```
