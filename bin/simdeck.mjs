#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const RECOVERABLE_RESTART_EXIT_CODE = 75;

const packageRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const binaryPath = path.join(packageRoot, "build", "simdeck-bin");
const childArgs = process.argv.slice(2);
const isDaemonRun = childArgs[0] === "daemon" && childArgs[1] === "run";

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
  const env = isDaemonRun
    ? {
        ...process.env,
        SIMDECK_DAEMON_METADATA_PID: String(process.pid),
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
      isDaemonRun &&
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
