#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

run("node", ["scripts/codex-setup.mjs", "--skip-npm"]);
run("npm", ["run", "build:cli"]);
run("npm", ["run", "build:client"]);
run("node", ["scripts/codex-worktree-cache.mjs", "save", "--best-effort"]);

const daemonArgs = ["daemon", "restart"];
pushOptionalEnv(daemonArgs, "--port", "SIMDECK_DAEMON_PORT");
pushOptionalEnv(daemonArgs, "--bind", "SIMDECK_DAEMON_BIND");
pushOptionalEnv(daemonArgs, "--advertise-host", "SIMDECK_ADVERTISE_HOST");
pushOptionalEnv(daemonArgs, "--video-codec", "SIMDECK_VIDEO_CODEC");
pushOptionalEnv(daemonArgs, "--stream-quality", "SIMDECK_STREAM_QUALITY");
pushOptionalEnv(daemonArgs, "--local-stream-fps", "SIMDECK_LOCAL_STREAM_FPS");
if (truthy(process.env.SIMDECK_LOW_LATENCY)) {
  daemonArgs.push("--low-latency");
}

run("./build/simdeck", daemonArgs);

function run(command, args) {
  console.log(`\n$ ${[command, ...args].join(" ")}`);
  const result = spawnSync(command, args, {
    cwd: ROOT,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function pushOptionalEnv(args, flag, envName) {
  const value = process.env[envName]?.trim();
  if (value) {
    args.push(flag, value);
  }
}

function truthy(value) {
  return value === "1" || value === "true" || value === "yes";
}
