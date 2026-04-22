#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "darwin") {
  console.warn(
    "xcode-canvas-web only supports macOS. Skipping native CLI build.",
  );
  process.exit(0);
}

const packageRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const buildScript = path.join(packageRoot, "scripts", "build-cli.sh");
const result = spawnSync(buildScript, {
  cwd: packageRoot,
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 1);
