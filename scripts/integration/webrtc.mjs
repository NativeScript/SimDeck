#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import {
  activationRecoveryReason,
  shouldRecycleSimulatorForFixtureLaunch,
} from "./activation-recovery.mjs";
import { buildCachedFixtureApp } from "./fixture.mjs";
import { selectIntegrationSimulator } from "./simulator-selection.mjs";

const root = path.resolve(new URL("../..", import.meta.url).pathname);
const simdeck = path.join(root, "build", "simdeck");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "simdeck-webrtc-it-"));
const serverPort = Number(
  process.env.SIMDECK_INTEGRATION_STREAM_PORT ?? "4520",
);
const serverUrl = `http://127.0.0.1:${serverPort}`;
const serverAccessToken = "integration";
const fixtureBundleId = "dev.nativescript.simdeck.integration.fixture";
const fixtureUrlScheme = "simdeck-fixture";
const fixtureAnimateUrl = "simdeck-fixture://animate";
const coreSimulatorCommandTimeoutMs = Number(
  process.env.SIMDECK_INTEGRATION_SIMCTL_TIMEOUT_MS ?? "300000",
);
const simdeckBootTimeoutMs = Number(
  process.env.SIMDECK_INTEGRATION_BOOT_TIMEOUT_MS ?? "300000",
);

let simulatorUDID = "";
let serverProcess = null;

process.on("SIGINT", () => {
  cleanup();
  process.exit(130);
});
process.on("SIGTERM", () => {
  cleanup();
  process.exit(143);
});
process.on("exit", cleanup);

main()
  .then(() => {
    cleanup();
    process.exit(0);
  })
  .catch((error) => {
    console.error(error?.stack ?? error);
    cleanup();
    process.exit(1);
  });

async function main() {
  if (process.platform !== "darwin") {
    throw new Error("SimDeck WebRTC integration tests require macOS.");
  }
  if (!fs.existsSync(simdeck)) {
    throw new Error(`Missing ${simdeck}. Run npm run build:cli first.`);
  }

  const { runtime, deviceType, sdkVersion } = selectIntegrationSimulator({
    runJson,
    runText,
    timeoutMs: coreSimulatorCommandTimeoutMs,
  });
  const simulatorName = `SimDeck WebRTC Integration ${Date.now()}`;
  simulatorUDID = runText("xcrun", [
    "simctl",
    "create",
    simulatorName,
    deviceType.identifier,
    runtime.identifier,
  ]).trim();
  console.log(
    `created ${simulatorUDID} (${deviceType.name}, ${runtime.version}; iphonesimulator SDK ${sdkVersion})`,
  );

  startServer();
  await waitForHealth();
  await retrySimdeckJson(["boot", simulatorUDID], "WebRTC boot simulator", {
    attempts: 3,
    delayMs: 3_000,
    timeoutMs: simdeckBootTimeoutMs,
  });
  runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
    timeoutMs: 600_000,
  });

  const fixture = buildCachedFixtureApp({
    root,
    tempRoot,
    bundleId: fixtureBundleId,
    urlScheme: fixtureUrlScheme,
  });
  await launchFixtureWithRecovery(fixture.appPath);

  const screenshotPath = path.join(tempRoot, "reference.png");
  simdeckJson(["screenshot", simulatorUDID, "--output", screenshotPath], {
    timeoutMs: 30_000,
  });
  const { width, height } = pngSize(screenshotPath);
  console.log(`reference screenshot ${width}x${height}`);

  const viewerUrl = new URL(serverUrl);
  viewerUrl.searchParams.set("device", simulatorUDID);
  viewerUrl.searchParams.set("simdeckToken", serverAccessToken);
  viewerUrl.searchParams.set("stream", "webrtc");

  runNodeScript(
    "scripts/e2e-webrtc-reliability.mjs",
    [
      viewerUrl.toString(),
      String(process.env.SIMDECK_E2E_WEBRTC_MS ?? "20000"),
    ],
    {
      SIMDECK_E2E_INTERACTIONS: "0",
      SIMDECK_E2E_MAX_DECODER_DROPS: "5",
      SIMDECK_E2E_MIN_DECODED_FPS:
        process.env.SIMDECK_E2E_MIN_DECODED_FPS ?? "55",
      SIMDECK_E2E_MIN_PRESENTED_FPS:
        process.env.SIMDECK_E2E_MIN_PRESENTED_FPS ?? "55",
      SIMDECK_E2E_MIN_RECEIVED_FPS:
        process.env.SIMDECK_E2E_MIN_RECEIVED_FPS ?? "55",
      SIMDECK_E2E_MIN_VIDEO_HEIGHT: String(height),
      SIMDECK_E2E_MIN_VIDEO_WIDTH: String(width),
      SIMDECK_E2E_REQUIRE_VISUAL: "0",
      SIMDECK_E2E_STREAM_READY_MS:
        process.env.SIMDECK_E2E_STREAM_READY_MS ?? "180000",
      SIMDECK_E2E_VISUAL_SAMPLE_INTERVAL_MS: "0",
      SIMDECK_E2E_WARMUP_MS: process.env.SIMDECK_E2E_WARMUP_MS ?? "2000",
    },
  );
}

function startServer() {
  killPortListeners(serverPort);
  serverProcess = spawn(
    simdeck,
    [
      "service",
      "run",
      "--metadata-path",
      path.join(tempRoot, "service.json"),
      "--port",
      String(serverPort),
      "--bind",
      "127.0.0.1",
      "--client-root",
      path.join(root, "packages", "client", "dist"),
      "--access-token",
      serverAccessToken,
      "--video-codec",
      "software",
      "--stream-quality",
      "full",
      "--local-stream-fps",
      "60",
    ],
    {
      cwd: tempRoot,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  serverProcess.stdout.on("data", (data) =>
    process.stdout.write(`[service] ${data}`),
  );
  serverProcess.stderr.on("data", (data) =>
    process.stderr.write(`[service] ${data}`),
  );
}

async function waitForHealth() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const health = await httpJson("/api/health");
      if (health.httpPort === serverPort) {
        return;
      }
    } catch {}
    await sleep(100);
  }
  throw new Error(`Timed out waiting for ${serverUrl}/api/health`);
}

async function launchFixtureWithRecovery(appPath, options = {}) {
  const recoveryCount = options.recoveryCount ?? 0;
  const maxRecoveries = options.maxRecoveries ?? 1;

  simdeckJson(["install", simulatorUDID, appPath], {
    timeoutMs: 60_000,
  });

  let launchError = null;
  try {
    simdeckJson(["launch", simulatorUDID, fixtureBundleId], {
      timeoutMs: 180_000,
    });
  } catch (error) {
    launchError = error;
  }

  let urlError = null;
  if (launchError === null) {
    try {
      await retrySimdeckJson(
        ["open-url", simulatorUDID, fixtureAnimateUrl],
        "WebRTC start fixture animation",
        {
          attempts: 3,
          delayMs: 5_000,
          timeoutMs: 180_000,
        },
      );
      return;
    } catch (error) {
      urlError = error;
    }
  }

  if (
    !shouldRecycleSimulatorForFixtureLaunch({
      launchError,
      urlError,
      recoveryCount,
      maxRecoveries,
    })
  ) {
    throw urlError ?? launchError;
  }

  console.warn(
    `WebRTC fixture activation hit ${activationRecoveryReason({
      launchError,
      urlError,
    })}; recycling simulator and retrying once.`,
  );
  await recycleSimulatorForFixtureLaunch();
  return launchFixtureWithRecovery(appPath, {
    recoveryCount: recoveryCount + 1,
    maxRecoveries,
  });
}

async function recycleSimulatorForFixtureLaunch() {
  try {
    simdeckJson(["shutdown", simulatorUDID], {
      timeoutMs: 180_000,
    });
  } catch (error) {
    console.warn(
      `WebRTC fixture recovery shutdown failed; continuing with boot: ${error?.message ?? error}`,
    );
  }
  await retrySimdeckJson(
    ["boot", simulatorUDID],
    "WebRTC fixture recovery boot",
    {
      attempts: 3,
      delayMs: 3_000,
      timeoutMs: simdeckBootTimeoutMs,
    },
  );
  runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
    timeoutMs: 600_000,
  });
}

function simdeckJson(args, options = {}) {
  return JSON.parse(runText(simdeck, args, options));
}

async function retrySimdeckJson(args, label, options = {}) {
  const attempts = options.attempts ?? 3;
  const delayMs = options.delayMs ?? 2_000;
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return simdeckJson(args, options);
    } catch (error) {
      lastError = error;
      if (attempt < attempts) {
        console.warn(
          `${label} attempt ${attempt}/${attempts} failed; retrying in ${delayMs}ms: ${error?.message ?? error}`,
        );
        await sleep(delayMs);
      }
    }
  }
  throw new Error(
    `${label} failed after ${attempts} attempts: ${lastError?.message ?? lastError}`,
  );
}

function runJson(command, args, options = {}) {
  return JSON.parse(runText(command, args, options));
}

function runText(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? root,
    encoding: "utf8",
    timeout: options.timeoutMs ?? 30_000,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with ${result.status ?? result.signal}\n${result.stdout}\n${result.stderr}\n${result.error?.message ?? ""}`,
    );
  }
  return result.stdout;
}

function runNodeScript(script, args, env) {
  const result = spawnSync(
    process.execPath,
    [path.join(root, script), ...args],
    {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, ...env },
      stdio: "inherit",
      timeout: 300_000,
    },
  );
  if (result.status !== 0) {
    throw new Error(`${script} failed with ${result.status}`);
  }
}

function httpJson(pathname) {
  return new Promise((resolve, reject) => {
    const request = http.get(`${serverUrl}${pathname}`, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => {
        body += chunk;
      });
      response.on("end", () => {
        if (response.statusCode && response.statusCode >= 400) {
          reject(
            new Error(`${pathname} returned ${response.statusCode}: ${body}`),
          );
          return;
        }
        try {
          resolve(JSON.parse(body));
        } catch (error) {
          reject(error);
        }
      });
    });
    request.on("error", reject);
    request.setTimeout(1000, () => {
      request.destroy(new Error("health request timed out"));
    });
  });
}

function pngSize(filePath) {
  const buffer = fs.readFileSync(filePath);
  if (
    buffer.length < 24 ||
    buffer.readUInt32BE(0) !== 0x89504e47 ||
    buffer.readUInt32BE(4) !== 0x0d0a1a0a
  ) {
    throw new Error(`${filePath} is not a PNG file`);
  }
  return {
    width: buffer.readUInt32BE(16),
    height: buffer.readUInt32BE(20),
  };
}

function killPortListeners(port) {
  const result = spawnSync("lsof", ["-ti", `tcp:${port}`], {
    encoding: "utf8",
  });
  for (const pid of result.stdout.split(/\s+/).filter(Boolean)) {
    spawnSync("kill", ["-TERM", pid]);
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function cleanup() {
  if (serverProcess && serverProcess.exitCode === null) {
    serverProcess.kill();
  }
  if (simulatorUDID) {
    spawnSync("xcrun", ["simctl", "shutdown", simulatorUDID], {
      stdio: "ignore",
    });
    spawnSync("xcrun", ["simctl", "delete", simulatorUDID], {
      stdio: "ignore",
    });
  }
  fs.rmSync(tempRoot, { recursive: true, force: true });
}
