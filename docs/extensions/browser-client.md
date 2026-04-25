# Browser Client

The default UI SimDeck serves at `/` is a React app built with Vite. It lives at `client/` in this repo and is bundled into `client/dist/` for production. The Rust server serves the bundle as static assets.

You almost certainly don't need to know about the internals — the client just works. This page is for contributors and for anyone who wants to embed the same surface somewhere else.

## Tech stack

- **React 19** — view layer.
- **TypeScript** — strict typing for everything in `client/src/`.
- **Vite** — dev server and production build.
- **Vitest** — unit tests.

## Layout

```text
client/
├── index.html
├── vite.config.js
├── package.json
└── src/
    ├── main.tsx
    ├── app/
    │   ├── App.tsx
    │   └── AppShell.tsx
    ├── api/
    │   ├── client.ts
    │   ├── controls.ts
    │   ├── simulators.ts
    │   └── types.ts
    ├── features/
    │   ├── simulators/
    │   ├── viewport/
    │   ├── stream/
    │   ├── input/
    │   ├── accessibility/
    │   └── toolbar/
    ├── shared/
    ├── styles/
    └── workers/
```

| Folder                    | Responsibility                                                          |
| ------------------------- | ----------------------------------------------------------------------- |
| `api/`                    | Typed wrappers around the SimDeck REST API and shared TypeScript types. |
| `features/simulators/`    | Sidebar list of simulators plus boot/shutdown affordances.              |
| `features/viewport/`      | Frame canvas, chrome compositing, hit testing.                          |
| `features/stream/`        | WebTransport reader, decoder workers, frame queueing.                   |
| `features/input/`         | Touch / keyboard / hardware-button affordances.                         |
| `features/accessibility/` | Accessibility tree pane and source switcher.                            |
| `features/toolbar/`       | Top toolbar (rotate, home, app switcher, dark mode toggle, refresh).    |
| `workers/`                | Off-main-thread video decoder workers.                                  |

## Bootstrap flow

1. The browser fetches `index.html` from the Rust server.
2. `main.tsx` mounts the React tree at `#root`.
3. `AppShell` calls `GET /api/health` to learn the WebTransport URL template, certificate hash, and packet version.
4. The simulator sidebar fetches `GET /api/simulators` and renders the list.
5. Selecting a simulator opens a WebTransport session at `wss://.../wt/simulators/<udid>` pinned by the cert hash.
6. The decoder worker parses [packet headers](/api/packet-format), reassembles description + data, and pushes decoded frames to the renderer.
7. Touch and key events round-trip through `POST /api/simulators/<udid>/touch` and `/key`.

## Dev workflow

The repo's `npm run dev` script runs the server and Vite together:

```sh
npm run dev
```

This:

- Builds the Rust CLI if it isn't built.
- Stops any stale SimDeck server listening on `4310` or `4311`.
- Starts the Rust server in the background, logging to `build/cli.log`.
- Runs `vite` from `client/` against the local server, with hot-module reload.

Vite serves on `http://127.0.0.1:5173` and proxies API calls to the Rust server on `4310`.

## Tests and types

```sh
npm run --prefix client typecheck
npm run --prefix client test
```

`typecheck` runs `tsc --noEmit` against the strict client config. `test` runs the Vitest suite in `client/src/`.

## Replacing the client

The Rust server takes a `--client-root <path>` flag. You can ship a completely different UI by pointing it at a directory of static files:

```sh
xcode-canvas-web serve --port 4310 --client-root /path/to/your/dist
```

As long as your client speaks the documented [REST API](/api/rest), [WebTransport](/api/webtransport), and [Packet Format](/api/packet-format), it will work end to end.

## Embedding in another app

The browser client is designed to live inside any container that can host a webview. The bundled VS Code extension is one example; embedding it in an Electron app, a Tauri shell, or a custom dashboard works the same way:

1. Point the host at `http://<simdeck-host>:<port>/`.
2. Allow the host to talk to the same WebTransport endpoint exposed by the server.
3. Optionally, gate the host behind your own auth — SimDeck assumes a trusted local network.
