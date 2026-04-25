# VS Code Extension

A bundled VS Code extension opens the SimDeck browser client inside an editor panel and auto-starts the local server when needed. The extension lives in `packages/vscode-extension/` and ships pre-bundled with the npm package.

## Install

The fastest path is to package the extension from this checkout and install it locally with the VS Code CLI:

```sh
npm run package:vscode-extension
npm run install:vscode-extension
```

This:

1. Builds a `.vsix` at `build/vscode/simdeck-vscode.vsix`.
2. Installs it via `code --install-extension build/vscode/simdeck-vscode.vsix --force`.

If the `code` command isn't on your `PATH`, install it from VS Code via **Shell Command: Install 'code' command in PATH** in the command palette.

## Use it

After installing, open the command palette and run:

```text
SimDeck: Open Simulator View
```

The extension opens a webview panel pointed at the configured server URL. If the server isn't running, the extension auto-launches it (see settings below).

Two more commands round out the surface:

- **SimDeck: Stop Managed Server** — stops the server the extension started.
- **SimDeck: Show Output** — opens the extension's output channel for debugging.

## Settings

All settings live under the `simdeck.*` namespace in VS Code settings:

| Setting                   | Default                 | Notes                                                                                                   |
| ------------------------- | ----------------------- | ------------------------------------------------------------------------------------------------------- |
| `simdeck.serverUrl`       | `http://127.0.0.1:4310` | URL the webview loads. Change this to point at a remote SimDeck instance.                               |
| `simdeck.cliPath`         | _empty_                 | Optional explicit path to the `simdeck` CLI. Empty means: workspace `build/`, then `PATH`.              |
| `simdeck.port`            | `4310`                  | Port used when auto-starting the server.                                                                |
| `simdeck.bindAddress`     | `127.0.0.1`             | Bind address used when auto-starting the server.                                                        |
| `simdeck.autoStartServer` | `true`                  | If `true`, the extension starts the local server when opening the simulator view if it isn't reachable. |

## Resolving the CLI

When the extension needs to start the server it looks in this order:

1. The explicit `simdeck.cliPath` setting.
2. The workspace's `build/simdeck` (handy when developing on this repo).
3. `simdeck` from `PATH`.

If none of those resolve, the extension surfaces an error in the output channel pointing you at this documentation.

## Talking to a remote server

Set `simdeck.serverUrl` to any reachable SimDeck endpoint. The extension is purely a webview shell — it doesn't validate the URL, doesn't open WebTransport itself, and doesn't ship its own version of the React client. Whatever the server responds with is what you get.

For [LAN-reachable servers](/guide/lan-access), point the extension at `http://<advertise-host>:<port>` and disable `autoStartServer` so the extension doesn't try to spawn a local CLI.

## Troubleshooting

- **The webview shows a blank panel.** Open the output channel; the extension logs whatever HTTP error it saw when probing `/api/health`.
- **Auto-start doesn't work.** Ensure the resolved `cliPath` exists and is executable. The extension shows the resolved path in the output channel.
- **Stale managed server stays running.** Use **SimDeck: Stop Managed Server** to terminate the server the extension spawned, then reopen the simulator view.
