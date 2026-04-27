const vscode = require("vscode");
const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const path = require("node:path");
const { spawn } = require("node:child_process");

let outputChannel;
let simulatorPanel;

function activate(context) {
  outputChannel = vscode.window.createOutputChannel("SimDeck");

  context.subscriptions.push(
    outputChannel,
    vscode.commands.registerCommand("simdeck.openSimulatorView", async () => {
      try {
        const serverUrl = await resolveSimulatorUrl(context);
        openSimulatorPanel(serverUrl);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        outputChannel.appendLine(message);
        outputChannel.show(true);
        void vscode.window.showErrorMessage(message);
      }
    }),
    vscode.commands.registerCommand("simdeck.stopServer", async () => {
      await stopProjectDaemon(context);
      await vscode.window.showInformationMessage(
        "Stopped the SimDeck project daemon.",
      );
    }),
    vscode.commands.registerCommand("simdeck.showOutput", () => {
      outputChannel.show(true);
    }),
  );
}

function deactivate() {}

function getServerUrl() {
  const config = vscode.workspace.getConfiguration("simdeck");
  return config.get("serverUrl", "http://127.0.0.1:4310");
}

async function resolveSimulatorUrl(context) {
  const serverUrl = getServerUrl();
  if (await isServerHealthy(serverUrl)) {
    return serverUrl;
  }

  const config = vscode.workspace.getConfiguration("simdeck");
  const autoStart = getAutoStartDaemon(config);
  if (!autoStart) {
    throw new Error(
      `SimDeck is not reachable at ${serverUrl}. Enable auto-start or launch the daemon manually.`,
    );
  }

  return await startProjectDaemon(context);
}

function getAutoStartDaemon(config) {
  const daemonSetting = config.inspect("autoStartDaemon");
  if (
    daemonSetting?.workspaceValue !== undefined ||
    daemonSetting?.workspaceFolderValue !== undefined ||
    daemonSetting?.globalValue !== undefined
  ) {
    return config.get("autoStartDaemon", true);
  }
  return config.get("autoStartServer", true);
}

async function startProjectDaemon(context) {
  const config = vscode.workspace.getConfiguration("simdeck");
  const cliPath = resolveCliPath(context, config.get("cliPath", ""));
  const port = String(config.get("port", 4310));
  const bindAddress = config.get("bindAddress", "127.0.0.1");
  const args = ["ui", "--port", port, "--bind", bindAddress];

  outputChannel.appendLine(`Starting SimDeck project daemon using ${cliPath}`);
  const result = await runCli(context, cliPath, args);
  outputChannel.append(result.stderr);

  const deadline = Date.now() + 15000;
  const metadata = parseJsonOutput(result.stdout, "simdeck ui");
  const daemonUrl = metadata.url;
  if (typeof daemonUrl !== "string" || daemonUrl.length === 0) {
    throw new Error("simdeck ui did not return a daemon URL.");
  }

  while (Date.now() < deadline) {
    if (await isServerHealthy(daemonUrl)) {
      outputChannel.appendLine(`SimDeck daemon ready at ${daemonUrl}`);
      return daemonUrl;
    }
    await delay(250);
  }

  throw new Error(`Timed out waiting for SimDeck at ${daemonUrl}.`);
}

async function stopProjectDaemon(context) {
  const config = vscode.workspace.getConfiguration("simdeck");
  const cliPath = resolveCliPath(context, config.get("cliPath", ""));
  outputChannel.appendLine(`Stopping SimDeck project daemon using ${cliPath}`);
  const result = await runCli(context, cliPath, ["daemon", "stop"]);
  outputChannel.append(result.stderr);
}

function runCli(context, cliPath, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(cliPath, args, {
      cwd: resolveWorkingDirectory(context),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk) => {
      const text = chunk.toString();
      stdout += text;
      outputChannel.append(text);
    });

    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });

    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve({ stdout, stderr });
        return;
      }
      const reason = signal ? `signal ${signal}` : `code ${code}`;
      reject(
        new Error(
          `simdeck ${args.join(" ")} exited with ${reason}.${stderr ? `\n${stderr}` : ""}`,
        ),
      );
    });
  });
}

function parseJsonOutput(stdout, commandName) {
  const trimmed = stdout.trim();
  if (!trimmed) {
    throw new Error(`${commandName} did not print JSON output.`);
  }

  try {
    return JSON.parse(trimmed);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`Unable to parse ${commandName} JSON output: ${message}`);
  }
}

function openSimulatorPanel(serverUrl) {
  if (simulatorPanel) {
    simulatorPanel.dispose();
    simulatorPanel = undefined;
  }

  simulatorPanel = vscode.window.createWebviewPanel(
    "simdeck.simulator",
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
    <iframe src="${escapedUrl}" title="SimDeck Simulator"></iframe>
  </body>
</html>`;
}

function resolveCliPath(context, configuredPath) {
  if (configuredPath) {
    return configuredPath;
  }

  const workspaceFolders = vscode.workspace.workspaceFolders ?? [];
  for (const folder of workspaceFolders) {
    const candidate = path.join(folder.uri.fsPath, "build", "simdeck");
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  const extensionWorkspaceCandidate = path.resolve(
    context.extensionPath,
    "..",
    "..",
    "build",
    "simdeck",
  );
  if (fs.existsSync(extensionWorkspaceCandidate)) {
    return extensionWorkspaceCandidate;
  }

  return "simdeck";
}

function resolveWorkingDirectory(context) {
  const workspaceFolders = vscode.workspace.workspaceFolders ?? [];
  if (workspaceFolders[0]) {
    return workspaceFolders[0].uri.fsPath;
  }
  return context.extensionPath;
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
