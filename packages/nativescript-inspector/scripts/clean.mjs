#!/usr/bin/env node
import { rmSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
process.chdir(root);

for (const target of ["build", "dist", "index.js", "index.mjs", "index.d.ts"]) {
  rmSync(target, { recursive: true, force: true });
}
