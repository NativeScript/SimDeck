#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const RECOVERABLE_RESTART_EXIT_CODE = 75;

const launcherDir = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = findPackageRoot(launcherDir);
const binaryPath = resolveBinaryPath(packageRoot);
const childArgs = process.argv.slice(2);
const isServiceRun = childArgs[0] === "service" && childArgs[1] === "run";

if (!binaryPath) {
  console.error("simdeck only supports macOS and Linux on arm64/x64.");
  process.exit(1);
}

if (!existsSync(binaryPath)) {
  console.error(
    "simdeck native binary is missing. Reinstall the npm package or run `npm run build:cli` from a source checkout.",
  );
  process.exit(1);
}

function findPackageRoot(startDir) {
  let current = path.resolve(startDir);
  while (true) {
    if (existsSync(path.join(current, "build", "simdeck-bin"))) {
      return current;
    }

    function resolveBinaryPath(rootDir) {
      const platform = process.platform;
      const arch = process.arch;
      const suffixByHost = {
        "darwin-arm64": "darwin-arm64",
        "darwin-x64": "darwin-x64",
        "linux-arm64": "linux-arm64",
        "linux-x64": "linux-x64",
      };

      const suffix = suffixByHost[`${platform}-${arch}`];
      if (!suffix) {
        return null;
      }

      const platformBinaryPath = path.join(rootDir, "build", `simdeck-bin-${suffix}`);
      if (existsSync(platformBinaryPath)) {
        return platformBinaryPath;
      }

      const fallbackBinaryPath = path.join(rootDir, "build", "simdeck-bin");
      return existsSync(fallbackBinaryPath) ? fallbackBinaryPath : platformBinaryPath;
    }
    const parent = path.dirname(current);
    if (parent === current) {
      return path.resolve(startDir, "../../..");
    }
    current = parent;
  }
}

let child;
let terminating = false;

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.once(signal, () => {
    terminating = true;
    if (child && !child.killed) {
      child.kill(signal);
    }
  });
}

function spawnChild() {
  const env = isServiceRun
    ? {
        ...process.env,
        SIMDECK_SERVICE_METADATA_PID: String(process.pid),
      }
    : process.env;

  child = spawn(binaryPath, childArgs, {
    cwd: process.cwd(),
    env,
    stdio: "inherit",
  });

  child.on("error", (error) => {
    console.error(error.message);
    process.exit(1);
  });

  child.on("exit", (code, signal) => {
    if (
      isServiceRun &&
      !terminating &&
      (code === RECOVERABLE_RESTART_EXIT_CODE || signal)
    ) {
      setTimeout(spawnChild, 500);
      return;
    }

    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 1);
  });
}

spawnChild();
