#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { readdirSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const skipNpm = args.includes("--skip-npm");
const skipCache = args.includes("--skip-cache");

if (!skipCache) {
  run("node", ["scripts/codex-worktree-cache.mjs", "hydrate"]);
}

ensureNativeBuildDependencies();

if (!skipNpm) {
  ensureNodeModules(".", "root");
  ensureNodeModules("client", "client");
}

function ensureNativeBuildDependencies() {
  if (process.platform !== "darwin") {
    return;
  }

  if (!commandSucceeds("pkg-config", ["--version"])) {
    installBrewPackage("pkgconf", "pkg-config");
  }

  if (!commandSucceeds("pkg-config", ["--exists", "x264"])) {
    installBrewPackage("x264", "x264 pkg-config metadata");
  }
}

function installBrewPackage(formula, label) {
  if (truthy(process.env.SIMDECK_CODEX_SKIP_BREW)) {
    throw new Error(
      `Missing ${label}. Install it with \`brew install ${formula}\` or unset SIMDECK_CODEX_SKIP_BREW.`,
    );
  }
  if (!commandSucceeds("brew", ["--version"])) {
    throw new Error(
      `Missing ${label}, and Homebrew is not available to install ${formula}.`,
    );
  }
  run("brew", ["install", formula]);
}

function ensureNodeModules(prefix, label) {
  const modulesPath =
    prefix === "."
      ? resolve(ROOT, "node_modules")
      : resolve(ROOT, prefix, "node_modules");
  if (existsAndHasContent(modulesPath)) {
    console.log(
      `[setup] skip ${label} npm install; node_modules already exists`,
    );
    return;
  }

  const args = prefix === "." ? ["ci"] : ["ci", "--prefix", prefix];
  run("npm", args);
}

function existsAndHasContent(path) {
  try {
    const stats = statSync(path);
    if (stats.isDirectory()) {
      return readdirSync(path).length > 0;
    }
    return stats.size > 0;
  } catch {
    return false;
  }
}

function commandSucceeds(command, args) {
  const result = spawnSync(command, args, {
    cwd: ROOT,
    stdio: "ignore",
    env: process.env,
  });
  return result.status === 0;
}

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

function truthy(value) {
  return value === "1" || value === "true" || value === "yes";
}
