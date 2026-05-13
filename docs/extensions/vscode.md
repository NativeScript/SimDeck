# VS Code Extension

The VS Code extension opens the SimDeck browser UI in an editor panel and can start the project daemon for you.

## Install

From a source checkout:

```sh
npm run package:vscode
npm run install:vscode
```

This builds and installs `build/vscode/simdeck-vscode.vsix`.

If the `code` command is missing, run **Shell Command: Install 'code' command in PATH** from the VS Code command palette.

## Open SimDeck

Run this command from the command palette:

```text
SimDeck: Open Simulator View
```

The extension tries the configured server URL first. If it is not reachable and auto-start is enabled, it runs `simdeck ui` for the current workspace.

## Commands

| Command                        | Purpose                   |
| ------------------------------ | ------------------------- |
| `SimDeck: Open Simulator View` | Open the webview panel    |
| `SimDeck: Stop Project Daemon` | Run `simdeck daemon stop` |
| `SimDeck: Show Output`         | Open extension logs       |

## Settings

| Setting                   | Default                 | Purpose                      |
| ------------------------- | ----------------------- | ---------------------------- |
| `simdeck.serverUrl`       | `http://127.0.0.1:4310` | Preferred daemon URL         |
| `simdeck.cliPath`         | empty                   | Explicit path to the CLI     |
| `simdeck.port`            | `4310`                  | Port for auto-start          |
| `simdeck.bindAddress`     | `127.0.0.1`             | Bind address for auto-start  |
| `simdeck.autoStartDaemon` | `true`                  | Start the daemon when needed |

CLI resolution order:

1. `simdeck.cliPath`
2. Workspace `build/simdeck`
3. `simdeck` on `PATH`

## Remote Or LAN Server

Set `simdeck.serverUrl` to the remote SimDeck URL and disable auto-start:

```json
{
  "simdeck.serverUrl": "http://192.168.1.50:4310",
  "simdeck.autoStartDaemon": false
}
```

Pair in the webview if the server asks for the LAN code.

## Troubleshooting

| Symptom                  | Fix                                                                 |
| ------------------------ | ------------------------------------------------------------------- |
| Blank panel              | Open `SimDeck: Show Output` and check the daemon URL and CLI stderr |
| Auto-start fails         | Set `simdeck.cliPath` to the real CLI path                          |
| Old server keeps loading | Run `SimDeck: Stop Project Daemon`, then reopen                     |
