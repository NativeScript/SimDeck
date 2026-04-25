#!/usr/bin/env node

import { existsSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packagePath = path.join(root, "build", "vscode", "simdeck-vscode.vsix");

if (!existsSync(packagePath)) {
  const packageResult = spawnSync(
    process.execPath,
    [path.join(root, "scripts", "package-vscode-extension.mjs")],
    {
      cwd: root,
      stdio: "inherit",
    },
  );

  if (packageResult.error) {
    console.error(packageResult.error.message);
    process.exit(1);
  }

  if ((packageResult.status ?? 1) !== 0) {
    process.exit(packageResult.status ?? 1);
  }
}

const cliCandidates = ["code", "code-insiders"];
const installer = cliCandidates.find((candidate) => {
  const probe = spawnSync(candidate, ["--version"], { stdio: "ignore" });
  return probe.status === 0;
});

if (!installer) {
  console.error(
    "Could not find the VS Code CLI. Install the `code` shell command and try again.",
  );
  process.exit(1);
}

const installResult = spawnSync(
  installer,
  ["--install-extension", packagePath, "--force"],
  {
    cwd: root,
    stdio: "inherit",
  },
);

if (installResult.error) {
  console.error(installResult.error.message);
  process.exit(1);
}

process.exit(installResult.status ?? 1);
