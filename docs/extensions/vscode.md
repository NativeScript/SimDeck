# VS Code Extension

A bundled VS Code extension opens the SimDeck browser client inside an editor panel and can start or reuse the project daemon when needed. The extension lives in `packages/vscode-extension/` and ships pre-bundled with the npm package.

## Install

The fastest path is to package the extension from this checkout and install it locally with the VS Code CLI:

```sh
npm run package:vscode-extension
npm run install:vscode-extension
```

Short aliases are available too:

```sh
npm run package:vscode
npm run install:vscode
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

The extension opens a webview panel pointed at a SimDeck daemon URL. If the configured URL is not reachable, the extension runs `simdeck ui`, reads the returned daemon URL, and loads that URL in the webview.

Two more commands round out the surface:

- **SimDeck: Stop Project Daemon** — runs `simdeck daemon stop` for the current workspace.
- **SimDeck: Show Output** — opens the extension's output channel for debugging.

## Settings

All settings live under the `simdeck.*` namespace in VS Code settings:

| Setting                   | Default                 | Notes                                                                                                                                   |
| ------------------------- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `simdeck.serverUrl`       | `http://127.0.0.1:4310` | Preferred URL to try first. If auto-start launches the daemon on a different port, the extension uses the URL returned by `simdeck ui`. |
| `simdeck.cliPath`         | _empty_                 | Optional explicit path to the `simdeck` CLI. Empty means: workspace `build/`, then `PATH`.                                              |
| `simdeck.port`            | `4310`                  | Preferred port passed to `simdeck ui` when auto-starting the project daemon.                                                            |
| `simdeck.bindAddress`     | `127.0.0.1`             | Bind address passed to `simdeck ui` when auto-starting the project daemon.                                                              |
| `simdeck.autoStartDaemon` | `true`                  | If `true`, the extension starts or reuses the project daemon when opening the simulator view if the preferred URL is not reachable.     |
| `simdeck.autoStartServer` | `true`                  | Deprecated compatibility alias for `simdeck.autoStartDaemon`.                                                                           |

## Resolving the CLI

When the extension needs to start or stop the project daemon it looks for the CLI in this order:

1. The explicit `simdeck.cliPath` setting.
2. The workspace's `build/simdeck` (handy when developing on this repo).
3. `simdeck` from `PATH`.

If none of those resolve, the extension surfaces an error in the output channel pointing you at this documentation.

## Talking to a remote server

Set `simdeck.serverUrl` to any reachable SimDeck endpoint. The extension is purely a webview shell — it doesn't open WebTransport itself and doesn't ship its own version of the React client. Whatever the daemon serves is what you get.

For [LAN-reachable daemons](/guide/lan-access), point the extension at `http://<advertise-host>:<port>` and disable `autoStartDaemon` so the extension does not start a local project daemon.

## Troubleshooting

- **The webview shows a blank panel.** Open the output channel; the extension logs the daemon URL returned by `simdeck ui` and any CLI stderr.
- **Auto-start doesn't work.** Ensure the resolved `cliPath` exists and is executable. The extension shows the resolved path in the output channel.
- **Stale daemon stays running.** Use **SimDeck: Stop Project Daemon** to run `simdeck daemon stop`, then reopen the simulator view.
