#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { createWriteStream, writeFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SERVER_BIN = resolve(ROOT, "build/xcode-canvas-web");
const LOG_PATH = resolve(ROOT, "build/cli.log");
const SERVER_PORT = "4310";

function findListeningPids(port) {
  try {
    const output = execFileSync("lsof", ["-t", `-iTCP:${port}`, "-sTCP:LISTEN"], {
      cwd: ROOT,
      encoding: "utf8"
    }).trim();
    return output
      .split(/\s+/)
      .map((value) => Number.parseInt(value, 10))
      .filter((value) => Number.isInteger(value));
  } catch {
    return [];
  }
}

function stopStaleCliProcesses() {
  const stalePids = new Set([...findListeningPids(4310), ...findListeningPids(4311)]);
  for (const pid of stalePids) {
    if (pid === process.pid) {
      continue;
    }

    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // Ignore races with already-exited processes.
    }
  }
}

stopStaleCliProcesses();
writeFileSync(LOG_PATH, "");
const logStream = createWriteStream(LOG_PATH);
console.log(`[server] serving on :${SERVER_PORT} — logs: build/cli.log`);

const cli = spawn(SERVER_BIN, ["serve", "--port", SERVER_PORT], {
  stdio: ["ignore", "pipe", "pipe"],
  cwd: ROOT
});

cli.stdout.pipe(logStream);
cli.stderr.pipe(logStream);

const vite = spawn("npx", ["vite", "--host", "127.0.0.1"], {
  stdio: "inherit",
  cwd: resolve(ROOT, "client")
});

function cleanup() {
  cli.kill();
  vite.kill();
  logStream.end();
}

process.on("SIGINT", cleanup);
process.on("SIGTERM", cleanup);

cli.on("error", (error) => {
  console.error(`[server] failed to start: ${error.message}`);
  vite.kill();
  process.exit(1);
});

cli.on("exit", (code, signal) => {
  const reason = code == null ? `signal ${signal ?? "unknown"}` : `code ${code}`;
  console.log(`[server] exited (${reason})`);
  vite.kill();
  process.exit(code ?? 1);
});

vite.on("exit", (code) => {
  console.log(`[vite] exited (${code})`);
  cli.kill();
  process.exit(code ?? 1);
});
