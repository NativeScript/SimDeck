#!/usr/bin/env node

import { mkdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const extensionDir = path.join(root, "packages", "vscode-extension");
const outputDir = path.join(root, "build", "vscode");
const packagePath = path.join(outputDir, "simdeck-vscode.vsix");
const vscePath = path.join(root, "node_modules", "@vscode", "vsce", "vsce");

mkdirSync(outputDir, { recursive: true });

const result = spawnSync(
  process.execPath,
  [
    "--",
    vscePath,
    "package",
    "--out",
    packagePath,
    "--allow-missing-repository",
  ],
  {
    cwd: extensionDir,
    stdio: "inherit",
  },
);

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 1);
