# VS Code Extension

A bundled VS Code extension opens the Simdeck browser client inside an editor panel and auto-starts the local server when needed. The extension lives in `packages/vscode-extension/` and ships pre-bundled with the npm package.

## Install

The fastest path is to package the extension from this checkout and install it locally with the VS Code CLI:

```sh
npm run package:vscode-extension
npm run install:vscode-extension
```

This:

1. Builds a `.vsix` at `build/vscode/xcode-canvas-web-vscode.vsix`.
2. Installs it via `code --install-extension build/vscode/xcode-canvas-web-vscode.vsix --force`.

If the `code` command isn't on your `PATH`, install it from VS Code via **Shell Command: Install 'code' command in PATH** in the command palette.

## Use it

After installing, open the command palette and run:

```text
Xcode Canvas Web: Open Simulator View
```

The extension opens a webview panel pointed at the configured server URL. If the server isn't running, the extension auto-launches it (see settings below).

Two more commands round out the surface:

- **Xcode Canvas Web: Stop Managed Server** — stops the server the extension started.
- **Xcode Canvas Web: Show Output** — opens the extension's output channel for debugging.

## Settings

All settings live under the `xcodeCanvasWeb.*` namespace in VS Code settings:

| Setting                          | Default                 | Notes                                                                                                   |
| -------------------------------- | ----------------------- | ------------------------------------------------------------------------------------------------------- |
| `xcodeCanvasWeb.serverUrl`       | `http://127.0.0.1:4310` | URL the webview loads. Change this to point at a remote Simdeck instance.                               |
| `xcodeCanvasWeb.cliPath`         | _empty_                 | Optional explicit path to the `xcode-canvas-web` CLI. Empty means: workspace `build/`, then `PATH`.     |
| `xcodeCanvasWeb.port`            | `4310`                  | Port used when auto-starting the server.                                                                |
| `xcodeCanvasWeb.bindAddress`     | `127.0.0.1`             | Bind address used when auto-starting the server.                                                        |
| `xcodeCanvasWeb.autoStartServer` | `true`                  | If `true`, the extension starts the local server when opening the simulator view if it isn't reachable. |

## Resolving the CLI

When the extension needs to start the server it looks in this order:

1. The explicit `xcodeCanvasWeb.cliPath` setting.
2. The workspace's `build/xcode-canvas-web` (handy when developing on this repo).
3. `xcode-canvas-web` from `PATH`.

If none of those resolve, the extension surfaces an error in the output channel pointing you at this documentation.

## Talking to a remote server

Set `xcodeCanvasWeb.serverUrl` to any reachable Simdeck endpoint. The extension is purely a webview shell — it doesn't validate the URL, doesn't open WebTransport itself, and doesn't ship its own version of the React client. Whatever the server responds with is what you get.

For [LAN-reachable servers](/guide/lan-access), point the extension at `http://<advertise-host>:<port>` and disable `autoStartServer` so the extension doesn't try to spawn a local CLI.

## Troubleshooting

- **The webview shows a blank panel.** Open the output channel; the extension logs whatever HTTP error it saw when probing `/api/health`.
- **Auto-start doesn't work.** Ensure the resolved `cliPath` exists and is executable. The extension shows the resolved path in the output channel.
- **Stale managed server stays running.** Use **Xcode Canvas Web: Stop Managed Server** to terminate the server the extension spawned, then reopen the simulator view.
