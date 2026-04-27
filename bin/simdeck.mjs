#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const binaryPath = path.join(packageRoot, "build", "simdeck-bin");

if (process.platform !== "darwin") {
  console.error("simdeck only supports macOS.");
  process.exit(1);
}

if (!existsSync(binaryPath)) {
  console.error(
    "simdeck native binary is missing. Reinstall the npm package or run `npm run build:cli` from a source checkout.",
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
