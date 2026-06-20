#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const buildDir = path.join(rootDir, "build");
const manifestPath = path.join(rootDir, "packages", "server", "Cargo.toml");
const serverTargetDir = path.join(rootDir, "packages", "server", "target");
const target = process.env.SIMDECK_BUILD_TARGET?.trim();
const hostExe = process.platform === "win32" ? ".exe" : "";
const outputBin = path.join(
  buildDir,
  `simdeck-bin${targetExe(target) ?? hostExe}`,
);
const hostPlatformBin = target ? null : hostPlatformBinaryPath();

fs.mkdirSync(buildDir, { recursive: true });

if (target) {
  const installedTargets = run("rustup", ["target", "list", "--installed"], {
    encoding: "utf8",
  }).stdout;
  if (!installedTargets.split(/\r?\n/).includes(target)) {
    console.log(`Installing missing Rust target: ${target}`);
    run("rustup", ["target", "add", target]);
  }
}

const cargoArgs = ["build", "--release", "--manifest-path", manifestPath];
if (target) {
  cargoArgs.push("--target", target);
}
run("cargo", cargoArgs);

const serverBin = path.join(
  serverTargetDir,
  ...(target ? [target] : []),
  "release",
  `simdeck-server${targetExe(target) ?? hostExe}`,
);
const tmpOutputBin = `${outputBin}.tmp.${process.pid}`;
fs.copyFileSync(serverBin, tmpOutputBin);
if (process.platform !== "win32") {
  fs.chmodSync(tmpOutputBin, 0o755);
}
fs.renameSync(tmpOutputBin, outputBin);
if (hostPlatformBin && hostPlatformBin !== outputBin) {
  const tmpHostPlatformBin = `${hostPlatformBin}.tmp.${process.pid}`;
  fs.copyFileSync(outputBin, tmpHostPlatformBin);
  if (process.platform !== "win32") {
    fs.chmodSync(tmpHostPlatformBin, 0o755);
  }
  fs.renameSync(tmpHostPlatformBin, hostPlatformBin);
}

console.log(`Built ${outputBin}`);
run("file", [outputBin], { optional: true });
if (hostPlatformBin && hostPlatformBin !== outputBin) {
  console.log(`Built ${hostPlatformBin}`);
  run("file", [hostPlatformBin], { optional: true });
}

if (process.platform !== "win32") {
  const output = path.join(buildDir, "simdeck");
  fs.writeFileSync(
    output,
    `#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ "\${1:-}" == "service" ]] && [[ "\${2:-}" == "run" ]]; then
  while true; do
    set +e
    "$SCRIPT_DIR/${path.basename(outputBin)}" "$@"
    child_status=$?
    set -e
    if [[ "$child_status" == "75" ]]; then
      sleep 0.5
      continue
    fi
    exit "$child_status"
  done
fi

exec "$SCRIPT_DIR/${path.basename(outputBin)}" "$@"
`,
  );
  fs.chmodSync(output, 0o755);
  console.log(`Built ${output}`);
}

function targetExe(value) {
  if (!value) {
    return null;
  }
  return value.includes("windows") ? ".exe" : "";
}

function hostPlatformBinaryPath() {
  const binaryByHost = {
    "darwin-arm64": "simdeck-bin-darwin-arm64",
    "darwin-x64": "simdeck-bin-darwin-x64",
    "linux-arm64": "simdeck-bin-linux-arm64",
    "linux-x64": "simdeck-bin-linux-x64",
    "win32-x64": "simdeck-bin-win32-x64.exe",
  };
  const binary = binaryByHost[`${process.platform}-${process.arch}`];
  return binary ? path.join(buildDir, binary) : null;
}

function run(command, args, options = {}) {
  const spawnOptions = {
    cwd: rootDir,
    stdio: options.encoding ? ["ignore", "pipe", "inherit"] : "inherit",
  };
  if (options.encoding) {
    spawnOptions.encoding = options.encoding;
  }
  const result = spawnSync(command, args, spawnOptions);
  if (result.error) {
    if (options.optional) {
      return result;
    }
    throw result.error;
  }
  if (result.status !== 0 && !options.optional) {
    process.exit(result.status ?? 1);
  }
  return result;
}
