#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const binaryPath = path.join(packageRoot, "build", "xcode-canvas-web-bin");

if (!existsSync(binaryPath)) {
  console.error(
    "xcode-canvas-web is not built yet. Reinstall or rebuild the package so the native CLI is available.",
  );
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), {
  cwd: process.cwd(),
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 1);
