#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, rmSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
process.chdir(root);

function run(cmd, args, label) {
  const result = spawnSync(cmd, args, {
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (result.status !== 0) {
    console.error(`[${label}] failed with status ${result.status}`);
    process.exit(result.status ?? 1);
  }
}

rmSync("build", { recursive: true, force: true });

run("npx", ["tsc", "-p", "tsconfig.cjs.json"], "cjs");
run("npx", ["tsc", "-p", "tsconfig.esm.json"], "esm");
run("npx", ["tsc", "-p", "tsconfig.types.json"], "types");

copyFileSync("build/cjs/index.js", "index.js");
copyFileSync("build/esm/index.js", "index.mjs");
copyFileSync("build/types/index.d.ts", "index.d.ts");

rmSync("build", { recursive: true, force: true });

console.log(
  "✓ built @nativescript/simdeck-inspector (index.js, index.mjs, index.d.ts)",
);
