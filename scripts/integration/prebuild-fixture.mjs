#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { buildCachedFixtureApp } from "./fixture.mjs";

const root = path.resolve(new URL("../..", import.meta.url).pathname);
const tempRoot = fs.mkdtempSync(
  path.join(os.tmpdir(), "simdeck-fixture-prebuild-"),
);

try {
  const fixture = buildCachedFixtureApp({
    root,
    tempRoot,
    bundleId: "dev.nativescript.simdeck.integration.fixture",
    urlScheme: "simdeck-fixture",
    log: (message) => console.log(`[fixture] ${message}`),
  });
  console.log(`Prepared ${fixture.appPath}`);
} finally {
  fs.rmSync(tempRoot, { recursive: true, force: true });
}
