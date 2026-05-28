#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const binary = resolve(
  root,
  process.env.SIMDECK_INTEGRATION_SERVICE_BINARY ?? join("build", "simdeck"),
);
const enabled =
  process.env.SIMDECK_INTEGRATION_LAUNCHAGENT === "1" ||
  process.env.CI === "true";

if (process.platform !== "darwin") {
  console.log("Skipping LaunchAgent restart integration test on non-macOS.");
  process.exit(0);
}

if (!enabled) {
  console.log(
    "Skipping LaunchAgent restart integration test. Set SIMDECK_INTEGRATION_LAUNCHAGENT=1 to run it.",
  );
  process.exit(0);
}

if (!existsSync(binary)) {
  throw new Error(
    `Missing SimDeck binary at ${binary}. Run npm run build:cli first.`,
  );
}

const tempRoot = await mkdtemp(join(tmpdir(), "simdeck-launchagent-it-"));
const home = join(tempRoot, "home");
const projectRoot = join(tempRoot, "project");
mkdirSync(home, { recursive: true });
mkdirSync(projectRoot, { recursive: true });

const servicePort = await findFreePort();
const clientRoot = join(root, "packages", "client", "dist");
const env = {
  ...process.env,
  HOME: home,
};

let blocker = null;

try {
  blocker = await listenIfAvailable("127.0.0.1", 4310);
  runJson(["service", "off"], { allowFailure: true });

  const onArgs = [
    "service",
    "on",
    "--port",
    String(servicePort),
    "--bind",
    "127.0.0.1",
    "--video-codec",
    "software",
    "--stream-quality",
    "tiny",
  ];
  if (existsSync(clientRoot)) {
    onArgs.push("--client-root", clientRoot);
  }

  const installed = runJson(onArgs);
  assertEqual(installed.ok, true, "service on should succeed");
  assertEqual(
    installed.port,
    servicePort,
    "service on should install requested port",
  );

  await waitForHealth(servicePort);

  const restarted = runJson(["service", "restart"]);
  assertEqual(restarted.ok, true, "service restart should succeed");
  assertEqual(
    restarted.port,
    servicePort,
    "service restart without --port should preserve the installed LaunchAgent port",
  );

  await waitForHealth(servicePort);
  const status = runJson(["service", "status"]);
  assertEqual(status.healthy, true, "service should be healthy after restart");
  assertEqual(
    status.service?.port,
    servicePort,
    "status should report the preserved LaunchAgent port",
  );

  console.log(
    JSON.stringify(
      {
        ok: true,
        binary,
        servicePort,
        defaultPortBlocked: Boolean(blocker),
      },
      null,
      2,
    ),
  );
} finally {
  try {
    runJson(["service", "off"], { allowFailure: true });
  } finally {
    if (blocker) {
      await new Promise((resolveClose) => blocker.close(resolveClose));
    }
    rmSync(tempRoot, { recursive: true, force: true });
  }
}

function runJson(args, options = {}) {
  const result = spawnSync(binary, args, {
    cwd: projectRoot,
    env,
    encoding: "utf8",
  });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  if (result.status !== 0) {
    if (options.allowFailure) {
      return null;
    }
    throw new Error(
      `${binary} ${args.join(" ")} failed with ${result.status}:\n${output}`,
    );
  }
  if (!output) {
    return null;
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(
      `${binary} ${args.join(" ")} did not print JSON: ${error.message}\n${output}`,
    );
  }
}

async function waitForHealth(port) {
  const deadline = Date.now() + 15_000;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (response.ok) {
        return;
      }
      lastError = new Error(`/api/health returned ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await sleep(50);
  }
  throw new Error(
    `Timed out waiting for SimDeck service on ${port}: ${lastError?.message ?? "unknown error"}`,
  );
}

async function findFreePort() {
  const server = await listenIfAvailable("127.0.0.1", 0);
  const address = server.address();
  await new Promise((resolveClose) => server.close(resolveClose));
  return address.port;
}

function listenIfAvailable(host, port) {
  return new Promise((resolveListen, rejectListen) => {
    const server = createServer();
    server.once("error", (error) => {
      if (error.code === "EADDRINUSE" && port !== 0) {
        resolveListen(null);
        return;
      }
      rejectListen(error);
    });
    server.listen(port, host, () => resolveListen(server));
  });
}

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${expected}, got ${actual}`);
  }
}
