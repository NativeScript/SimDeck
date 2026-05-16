#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import {
  constants,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const ROOT = repoRoot();
const PROJECT_NAME = basename(ROOT);
const CACHE_ROOT =
  process.env.SIMDECK_CODEX_CACHE_ROOT ||
  join(homedir(), ".cache", "simdeck", "codex-worktree-cache");

const args = process.argv.slice(2);
const command = args[0] || "hydrate";
const force = args.includes("--force");
const bestEffort = args.includes("--best-effort") || command === "hydrate";

if (!["hydrate", "save", "status"].includes(command)) {
  console.error("Usage: codex-worktree-cache.mjs <hydrate|save|status> [--force]");
  process.exit(2);
}

const entries = buildEntries();

try {
  if (command === "hydrate") {
    hydrateEntries(entries);
  } else if (command === "save") {
    saveEntries(entries);
  } else {
    printStatus(entries);
  }
} catch (error) {
  if (!bestEffort) {
    throw error;
  }
  console.warn(`[cache] ${describeError(error)}`);
}

function hydrateEntries(entries) {
  const sourceRoots = cacheSourceRoots();
  for (const entry of entries) {
    const destination = join(ROOT, entry.destination);
    if (existsAndHasContent(destination) && !force) {
      log(`skip ${entry.label}; ${entry.destination} already exists`);
      continue;
    }

    const source = findHydrationSource(entry, sourceRoots);
    if (!source) {
      log(`miss ${entry.label}`);
      continue;
    }

    copyIntoPlace(source.path, destination);
    log(`hydrated ${entry.label} from ${source.description}`);
  }
}

function saveEntries(entries) {
  mkdirSync(CACHE_ROOT, { recursive: true });
  for (const entry of entries) {
    const source = join(ROOT, entry.destination);
    if (!existsAndHasContent(source)) {
      log(`skip ${entry.label}; ${entry.destination} is missing`);
      continue;
    }

    copyIntoPlace(source, entry.cachePath, { replace: true });
    log(`saved ${entry.label}`);
  }
}

function printStatus(entries) {
  console.log(`Cache root: ${CACHE_ROOT}`);
  for (const entry of entries) {
    const destination = join(ROOT, entry.destination);
    console.log(
      `${entry.label}: destination=${existsAndHasContent(destination) ? "present" : "missing"} cache=${
        existsAndHasContent(entry.cachePath) ? "present" : "missing"
      }`,
    );
  }
}

function buildEntries() {
  const rootLockHash = hashFiles(["package-lock.json"]);
  const clientLockHash = hashFiles(["client/package-lock.json"]);
  const cargoHash = hashFiles(["server/Cargo.toml", "server/Cargo.lock"]);
  const rustHost = rustHostTriple();

  return [
    {
      label: "root node_modules",
      destination: "node_modules",
      cachePath: join(CACHE_ROOT, "node", "root", rootLockHash, "node_modules"),
      sourceLockFiles: ["package-lock.json"],
    },
    {
      label: "client node_modules",
      destination: "client/node_modules",
      cachePath: join(CACHE_ROOT, "node", "client", clientLockHash, "node_modules"),
      sourceLockFiles: ["client/package-lock.json"],
    },
    {
      label: "Rust target",
      destination: "server/target",
      cachePath: join(CACHE_ROOT, "rust", rustHost, cargoHash, "target"),
      sourceLockFiles: ["server/Cargo.toml", "server/Cargo.lock"],
    },
  ];
}

function findHydrationSource(entry, sourceRoots) {
  if (existsAndHasContent(entry.cachePath)) {
    return { path: entry.cachePath, description: entry.cachePath };
  }

  for (const root of sourceRoots) {
    if (!locksMatch(root, entry.sourceLockFiles)) {
      continue;
    }
    const candidate = join(root, entry.destination);
    if (existsAndHasContent(candidate)) {
      return { path: candidate, description: root };
    }
  }

  return null;
}

function cacheSourceRoots() {
  const roots = [];
  const explicitSource = process.env.SIMDECK_CACHE_SOURCE;
  if (explicitSource) {
    roots.push(resolve(explicitSource));
  }

  const commonRoot = mainCheckoutRoot();
  if (commonRoot) {
    roots.push(commonRoot);
  }

  const codexWorktrees = join(homedir(), ".codex", "worktrees");
  if (existsSync(codexWorktrees)) {
    for (const id of readdirSync(codexWorktrees)) {
      const candidate = join(codexWorktrees, id, PROJECT_NAME);
      if (candidate !== ROOT && existsSync(candidate)) {
        roots.push(candidate);
      }
    }
  }

  return [...new Set(roots)]
    .filter((root) => root !== ROOT && existsSync(root))
    .sort((left, right) => mtimeMs(right) - mtimeMs(left));
}

function locksMatch(candidateRoot, lockFiles) {
  for (const lockFile of lockFiles) {
    const current = join(ROOT, lockFile);
    const candidate = join(candidateRoot, lockFile);
    if (!existsSync(current) || !existsSync(candidate)) {
      return false;
    }
    if (hashPath(current) !== hashPath(candidate)) {
      return false;
    }
  }
  return true;
}

function copyIntoPlace(source, destination, { replace = false } = {}) {
  if (replace) {
    mkdirSync(dirname(destination), { recursive: true });
  } else if (existsSync(destination)) {
    if (force) {
      rmSync(destination, { recursive: true, force: true });
    } else {
      return;
    }
  }

  const temporary = `${destination}.tmp-${process.pid}-${Date.now()}`;
  rmSync(temporary, { recursive: true, force: true });
  mkdirSync(dirname(temporary), { recursive: true });

  try {
    clonePath(source, temporary);
    if (replace) {
      rmSync(destination, { recursive: true, force: true });
    }
    renameSync(temporary, destination);
  } catch (error) {
    rmSync(temporary, { recursive: true, force: true });
    if (!bestEffort) {
      throw error;
    }
    console.warn(`[cache] failed to copy ${source}: ${describeError(error)}`);
  }
}

function clonePath(source, destination) {
  const stats = statSync(source);
  if (stats.isFile()) {
    try {
      copyFileSync(source, destination, constants.COPYFILE_FICLONE);
    } catch {
      copyFileSync(source, destination);
    }
    return;
  }

  try {
    cpSync(source, destination, {
      recursive: true,
      verbatimSymlinks: true,
      preserveTimestamps: true,
      mode: constants.COPYFILE_FICLONE,
    });
  } catch {
    cpSync(source, destination, {
      recursive: true,
      verbatimSymlinks: true,
      preserveTimestamps: true,
    });
  }
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

function repoRoot() {
  const result = spawnSync("git", ["rev-parse", "--show-toplevel"], {
    encoding: "utf8",
  });
  if (result.status === 0) {
    return result.stdout.trim();
  }
  return resolve(dirname(fileURLToPath(import.meta.url)), "..");
}

function mainCheckoutRoot() {
  const result = spawnSync(
    "git",
    ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    { cwd: ROOT, encoding: "utf8" },
  );
  if (result.status !== 0) {
    return null;
  }

  const gitCommonDir = result.stdout.trim();
  if (basename(gitCommonDir) !== ".git") {
    return null;
  }
  return dirname(gitCommonDir);
}

function hashFiles(paths) {
  const hash = createHash("sha256");
  for (const path of paths) {
    hash.update(path);
    hash.update("\0");
    hash.update(readFileSync(join(ROOT, path)));
    hash.update("\0");
  }
  return hash.digest("hex").slice(0, 16);
}

function hashPath(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function rustHostTriple() {
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  if (result.status !== 0) {
    return `${process.platform}-${process.arch}`;
  }
  const host = result.stdout
    .split("\n")
    .find((line) => line.startsWith("host: "))
    ?.slice("host: ".length)
    .trim();
  return host || `${process.platform}-${process.arch}`;
}

function mtimeMs(path) {
  try {
    return statSync(path).mtimeMs;
  } catch {
    return 0;
  }
}

function log(message) {
  console.log(`[cache] ${message}`);
}

function describeError(error) {
  return error instanceof Error ? error.message : String(error);
}
