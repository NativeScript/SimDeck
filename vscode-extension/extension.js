const vscode = require("vscode");
const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const path = require("node:path");
const { spawn } = require("node:child_process");

let outputChannel;
let serverProcess;
let simulatorPanel;

function activate(context) {
  outputChannel = vscode.window.createOutputChannel("Xcode Canvas Web");

  context.subscriptions.push(
    outputChannel,
    vscode.commands.registerCommand(
      "xcodeCanvasWeb.openSimulatorView",
      async () => {
        try {
          const serverUrl = getServerUrl();
          await ensureServerRunning(context, serverUrl);
          openSimulatorPanel(serverUrl);
        } catch (error) {
          const message =
            error instanceof Error ? error.message : String(error);
          outputChannel.appendLine(message);
          outputChannel.show(true);
          void vscode.window.showErrorMessage(message);
        }
      },
    ),
    vscode.commands.registerCommand("xcodeCanvasWeb.stopServer", async () => {
      stopManagedServer();
      await vscode.window.showInformationMessage(
        "Stopped the managed Xcode Canvas Web server.",
      );
    }),
    vscode.commands.registerCommand("xcodeCanvasWeb.showOutput", () => {
      outputChannel.show(true);
    }),
    {
      dispose: () => {
        stopManagedServer();
      },
    },
  );
}

function deactivate() {
  stopManagedServer();
}

function getServerUrl() {
  const config = vscode.workspace.getConfiguration("xcodeCanvasWeb");
  return config.get("serverUrl", "http://127.0.0.1:4310");
}

async function ensureServerRunning(context, serverUrl) {
  if (await isServerHealthy(serverUrl)) {
    return;
  }

  const config = vscode.workspace.getConfiguration("xcodeCanvasWeb");
  const autoStart = config.get("autoStartServer", true);
  if (!autoStart) {
    throw new Error(
      `Xcode Canvas Web is not reachable at ${serverUrl}. Enable auto-start or launch the server manually.`,
    );
  }

  if (!serverProcess || serverProcess.exitCode !== null) {
    await startServer(context);
  }

  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) {
    if (await isServerHealthy(serverUrl)) {
      return;
    }
    await delay(250);
  }

  throw new Error(`Timed out waiting for Xcode Canvas Web at ${serverUrl}.`);
}

async function startServer(context) {
  const config = vscode.workspace.getConfiguration("xcodeCanvasWeb");
  const cliPath = resolveCliPath(context, config.get("cliPath", ""));
  const port = String(config.get("port", 4310));
  const bindAddress = config.get("bindAddress", "127.0.0.1");

  outputChannel.appendLine(`Starting Xcode Canvas Web using ${cliPath}`);

  serverProcess = spawn(
    cliPath,
    ["serve", "--port", port, "--bind", bindAddress],
    {
      cwd: resolveWorkingDirectory(context),
      stdio: ["ignore", "pipe", "pipe"],
    },
  );

  serverProcess.stdout.on("data", (chunk) => {
    outputChannel.append(chunk.toString());
  });

  serverProcess.stderr.on("data", (chunk) => {
    outputChannel.append(chunk.toString());
  });

  serverProcess.on("exit", (code, signal) => {
    outputChannel.appendLine(
      `Xcode Canvas Web server exited with ${signal ? `signal ${signal}` : `code ${code}`}.`,
    );
    serverProcess = undefined;
  });

  serverProcess.on("error", (error) => {
    outputChannel.appendLine(error.message);
  });
}

function openSimulatorPanel(serverUrl) {
  if (simulatorPanel) {
    simulatorPanel.dispose();
    simulatorPanel = undefined;
  }

  simulatorPanel = vscode.window.createWebviewPanel(
    "xcodeCanvasWeb.simulator",
    "Simulator View",
    vscode.ViewColumn.Beside,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
    },
  );

  simulatorPanel.webview.html = getWebviewHtml(serverUrl);
  simulatorPanel.onDidDispose(() => {
    simulatorPanel = undefined;
  });
}

function getWebviewHtml(serverUrl) {
  const origin = getOrigin(serverUrl);
  const escapedUrl = escapeHtml(serverUrl);
  const escapedOrigin = escapeHtml(origin);

  return `<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta
      http-equiv="Content-Security-Policy"
      content="default-src 'none'; style-src 'unsafe-inline'; frame-src ${escapedOrigin};"
    />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <style>
      html,
      body {
        margin: 0;
        padding: 0;
        width: 100%;
        height: 100%;
        overflow: hidden;
        background: #000;
      }

      iframe {
        position: fixed;
        inset: 0;
        width: 100vw;
        height: 100vh;
        border: 0;
        background: #000;
      }
    </style>
  </head>
  <body>
    <iframe src="${escapedUrl}" title="Xcode Canvas Web Simulator"></iframe>
  </body>
</html>`;
}

function resolveCliPath(context, configuredPath) {
  if (configuredPath) {
    return configuredPath;
  }

  const workspaceFolders = vscode.workspace.workspaceFolders ?? [];
  for (const folder of workspaceFolders) {
    const candidate = path.join(folder.uri.fsPath, "build", "xcode-canvas-web");
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  const extensionWorkspaceCandidate = path.resolve(
    context.extensionPath,
    "..",
    "..",
    "build",
    "xcode-canvas-web",
  );
  if (fs.existsSync(extensionWorkspaceCandidate)) {
    return extensionWorkspaceCandidate;
  }

  return "xcode-canvas-web";
}

function resolveWorkingDirectory(context) {
  const workspaceFolders = vscode.workspace.workspaceFolders ?? [];
  if (workspaceFolders[0]) {
    return workspaceFolders[0].uri.fsPath;
  }
  return context.extensionPath;
}

function stopManagedServer() {
  if (!serverProcess || serverProcess.exitCode !== null) {
    serverProcess = undefined;
    return;
  }

  serverProcess.kill("SIGTERM");
  serverProcess = undefined;
}

function isServerHealthy(serverUrl) {
  return new Promise((resolve) => {
    let target;
    try {
      target = new URL("/api/health", serverUrl);
    } catch {
      resolve(false);
      return;
    }

    const transport = target.protocol === "https:" ? https : http;
    const request = transport.get(
      target,
      {
        timeout: 1500,
      },
      (response) => {
        response.resume();
        resolve(
          Boolean(
            response.statusCode &&
            response.statusCode >= 200 &&
            response.statusCode < 300,
          ),
        );
      },
    );

    request.on("timeout", () => {
      request.destroy();
      resolve(false);
    });

    request.on("error", () => {
      resolve(false);
    });
  });
}

function getOrigin(value) {
  const url = new URL(value);
  return `${url.protocol}//${url.host}`;
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

module.exports = {
  activate,
  deactivate,
};
