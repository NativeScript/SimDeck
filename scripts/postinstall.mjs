#!/usr/bin/env node

const isGlobalInstall =
  process.env.npm_config_global === "true" ||
  process.env.npm_config_location === "global" ||
  process.env.npm_config_global_style === "true";

const isCi =
  process.env.CI === "true" ||
  process.env.npm_config_loglevel === "silent" ||
  process.env.npm_config_loglevel === "error";

if (!isGlobalInstall || isCi) {
  process.exit(0);
}

const message = `
SimDeck is installed.

Open the simulator UI:
  simdeck

Open a specific simulator:
  simdeck "iPhone 17 Pro"

Detached daemon shortcuts:
  simdeck -d
  simdeck -k
  simdeck -r

Install the Codex skill:
  npx skills add NativeScript/SimDeck --skill simdeck -a codex -g

Recommended VS Code extension:
  nativescript.simdeck

Recommended for always-on agent/editor access:
  simdeck service on
  simdeck service off
`;

console.log(message.trimEnd());
