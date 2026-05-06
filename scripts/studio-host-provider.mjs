#!/usr/bin/env node

import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const packageRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const defaultConfigPath = path.join(
  os.homedir(),
  ".simdeck",
  "studio-provider.json",
);
const defaultWorkRoot = path.join(os.homedir(), ".simdeck", "studio-provider");
const defaultLocalUrl = "http://127.0.0.1:4310";

const command = process.argv[2] || "";

if (isMainModule()) {
  try {
    if (command === "connect") {
      await connect(parseArgs(process.argv.slice(3)));
    } else if (command === "run") {
      await run(parseArgs(process.argv.slice(3)));
    } else if (command === "status") {
      await status(parseArgs(process.argv.slice(3)));
    } else {
      usage();
      process.exit(command ? 2 : 0);
    }
  } catch (error) {
    console.error(`[simdeck-provider] ${describeError(error)}`);
    process.exit(1);
  }
}

async function connect(args) {
  const studioUrl = requiredArg(args, "studio-url").replace(/\/$/, "");
  const hostId = requiredArg(args, "host-id");
  const hostToken = requiredArg(args, "host-token");
  const configPath = args["config"] || defaultConfigPath;
  const config = {
    createdAt: new Date().toISOString(),
    hostId,
    hostToken,
    studioUrl,
    workRoot: args["work-root"] || defaultWorkRoot,
  };
  await writeJsonFile(configPath, config, 0o600);
  console.log(`Saved SimDeck Studio provider config to ${configPath}`);
  console.log("Run `simdeck provider run` to start the provider.");
}

async function status(args) {
  const config = await loadConfig(args);
  const local = await localProviderMetadata(config).catch((error) => ({
    ok: false,
    error: describeError(error),
  }));
  console.log(JSON.stringify({ config: redactConfig(config), local }, null, 2));
}

async function run(args) {
  const config = await loadConfig(args);
  config.localUrl = (
    args["local-url"] ||
    config.localUrl ||
    defaultLocalUrl
  ).replace(/\/$/, "");
  config.localToken = args["local-token"] || config.localToken || "";
  config.maxCapacity = clampCapacity(
    Number(args["max-capacity"] || config.maxCapacity || 1),
  );
  config.workRoot = args["work-root"] || config.workRoot || defaultWorkRoot;
  config.simulatorTemplateName =
    args["simulator-template"] ||
    config.simulatorTemplateName ||
    "iPhone 17 Pro";
  config.pollIntervalMs = Number(args["poll-interval-ms"] || 750);
  config.heartbeatIntervalMs = Number(args["heartbeat-interval-ms"] || 15000);
  config.proxyTimeoutMs = Number(args["proxy-timeout-ms"] || 25000);
  config.videoCodec = args["video-codec"] || config.videoCodec || "software";
  config.streamQuality =
    args["stream-quality"] || config.streamQuality || "smooth";

  await fs.promises.mkdir(config.workRoot, { recursive: true });
  const state = {
    activeRequests: new Set(),
    activeSessions: new Map(),
    inFlightAllocations: 0,
    lastHeartbeatAt: 0,
    stopped: false,
  };
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.once(signal, () => {
      state.stopped = true;
    });
  }

  await ensureDaemon(config);
  await heartbeat(config, state, true);
  console.log(
    `[simdeck-provider] online as ${config.hostId} for ${config.studioUrl}`,
  );

  while (!state.stopped) {
    try {
      if (Date.now() - state.lastHeartbeatAt >= config.heartbeatIntervalMs) {
        await heartbeat(config, state, false);
      }
      await Promise.all([pollJob(config, state), pollRpc(config, state)]);
    } catch (error) {
      console.error(`[simdeck-provider] ${describeError(error)}`);
      await sleep(1000);
    }
    await sleep(config.pollIntervalMs);
  }

  await Promise.allSettled(state.activeRequests);
  await heartbeat(config, state, false, "draining");
}

async function ensureDaemon(config) {
  const status = await daemonStatus().catch(() => null);
  if (daemonLooksUsable(status, config)) {
    config.localUrl = status.httpUrl.replace(/\/$/, "");
    config.localToken = status.accessToken;
    return status;
  }
  const args = [
    "daemon",
    status ? "restart" : "start",
    "--port",
    String(new URL(config.localUrl).port || 4310),
    "--bind",
    "127.0.0.1",
    "--video-codec",
    config.videoCodec,
    "--stream-quality",
    config.streamQuality,
  ];
  await execFileAsync(simdeckBinary(), args, { timeout: 120000 });
  const next = await daemonStatus();
  config.localUrl = next.httpUrl.replace(/\/$/, "");
  config.localToken = next.accessToken;
  return next;
}

async function daemonStatus() {
  const { stdout } = await execFileAsync(
    simdeckBinary(),
    ["daemon", "status"],
    {
      timeout: 15000,
    },
  );
  const parsed = JSON.parse(stdout);
  return parsed.daemon || parsed;
}

function daemonLooksUsable(status, config) {
  if (!status?.httpUrl || !status?.accessToken) {
    return false;
  }
  return status.httpUrl.replace(/\/$/, "") === config.localUrl;
}

async function heartbeat(config, state, first, statusOverride) {
  const metadata = await localProviderMetadata(config).catch((error) => ({
    capabilities: { error: describeError(error) },
    ok: false,
  }));
  await studioJson(config, "/api/actions/provider-hosts/heartbeat", {
    activeSessionCount: state.activeSessions.size,
    capabilities: metadata.capabilities,
    hostId: config.hostId,
    hostToken: config.hostToken,
    maxCapacity: config.maxCapacity,
    simulatorTemplateName: config.simulatorTemplateName,
    status: statusOverride || (metadata.ok ? "online" : "draining"),
  });
  state.lastHeartbeatAt = Date.now();
  if (!metadata.ok && first) {
    throw new Error("Local SimDeck daemon is not healthy.");
  }
}

async function pollJob(config, state) {
  if (
    state.activeSessions.size + state.inFlightAllocations >=
    config.maxCapacity
  ) {
    return;
  }
  const response = await studioJson(
    config,
    "/api/actions/provider-hosts/jobs/next",
    {
      hostId: config.hostId,
      hostToken: config.hostToken,
    },
  );
  if (!response?.job) {
    return;
  }
  const task = handleJob(config, state, response.job).catch((error) => {
    console.error(
      `[simdeck-provider] job ${response.job.id} failed: ${describeError(error)}`,
    );
  });
  state.activeRequests.add(task);
  task.finally(() => state.activeRequests.delete(task));
}

async function handleJob(config, state, job) {
  if (job.type === "allocate") {
    if (
      state.activeSessions.size + state.inFlightAllocations >=
      config.maxCapacity
    ) {
      return;
    }
    state.inFlightAllocations += 1;
    try {
      await allocateSession(config, state, job);
    } finally {
      state.inFlightAllocations = Math.max(0, state.inFlightAllocations - 1);
    }
  } else if (job.type === "release") {
    await releaseSession(config, state, job);
  } else {
    await completeJob(config, job.id, {
      error: `Unsupported provider job type: ${job.type}`,
      status: "failed",
    });
  }
}

async function allocateSession(config, state, job) {
  const payload = job.payload || {};
  const templateName =
    payload.simulatorTemplateName ||
    config.simulatorTemplateName ||
    "iPhone 17 Pro";
  let udid = "";
  try {
    const template = await ensureTemplateSimulator(templateName);
    const sessionName = `SimDeck ${payload.sessionId || job.sessionId}`;
    udid = await cloneSimulator(template.udid, sessionName);
    state.activeSessions.set(job.sessionId, { udid });
    await bootSimulator(config, udid);
    if (payload.artifactId) {
      const appPath = await downloadAndExtractArtifact(config, job.sessionId);
      await localJson(
        config,
        `/api/simulators/${encodeURIComponent(udid)}/install`,
        {
          appPath,
        },
      );
    }
    if (payload.bundleId) {
      await localJson(
        config,
        `/api/simulators/${encodeURIComponent(udid)}/launch`,
        {
          bundleId: payload.bundleId,
        },
      );
    }
    const simulator = await simulatorByUdid(config, udid);
    await completeJob(config, job.id, {
      activeSessionCount: state.activeSessions.size,
      runtimeName: simulator?.runtimeName,
      simulatorName: simulator?.name || sessionName,
      simulatorUdid: udid,
      status: "completed",
    });
    console.log(`[simdeck-provider] allocated ${udid} for ${job.sessionId}`);
  } catch (error) {
    if (udid) {
      await deleteSimulator(udid).catch(() => {});
      state.activeSessions.delete(job.sessionId);
    }
    await completeJob(config, job.id, {
      activeSessionCount: state.activeSessions.size,
      error: describeError(error),
      status: "failed",
    });
  }
}

async function releaseSession(config, state, job) {
  const payload = job.payload || {};
  const udid =
    payload.simulatorUdid ||
    state.activeSessions.get(job.sessionId)?.udid ||
    "";
  if (udid) {
    await localJson(
      config,
      `/api/simulators/${encodeURIComponent(udid)}/shutdown`,
      {},
    ).catch(() => {});
    await deleteSimulator(udid);
  }
  state.activeSessions.delete(job.sessionId);
  await completeJob(config, job.id, {
    activeSessionCount: state.activeSessions.size,
    status: "completed",
  });
  console.log(`[simdeck-provider] released ${job.sessionId}`);
}

async function pollRpc(config, state) {
  const response = await studioJson(
    config,
    "/api/actions/provider-hosts/rpc/next",
    {
      hostId: config.hostId,
      hostToken: config.hostToken,
    },
  );
  if (!response?.request) {
    return;
  }
  const task = handleRpc(config, response.request).catch((error) => {
    console.error(
      `[simdeck-provider] rpc ${response.request.id} failed: ${describeError(error)}`,
    );
  });
  state.activeRequests.add(task);
  task.finally(() => state.activeRequests.delete(task));
}

async function handleRpc(config, request) {
  if (isWebSocketUpgradeRequest(request)) {
    await completeRpc(config, request.id, {
      responseBodyBase64: Buffer.from(
        "Studio provider RPC does not tunnel WebSocket upgrade requests.",
      ).toString("base64"),
      responseHeaders: { "content-type": "text/plain; charset=utf-8" },
      responseStatus: 426,
    });
    return;
  }
  try {
    await completeRpc(
      config,
      request.id,
      await proxyLocalRequest(config, request),
    );
  } catch (error) {
    await completeRpc(config, request.id, { error: describeError(error) });
  }
}

async function proxyLocalRequest(config, request) {
  const target = new URL(request.path, `${config.localUrl}/`);
  if (!target.searchParams.has("simdeckToken")) {
    target.searchParams.set("simdeckToken", config.localToken);
  }
  const headers = new Headers(request.headers || {});
  headers.set("x-simdeck-token", config.localToken);
  headers.delete("host");
  headers.delete("content-length");
  const response = await fetch(target, {
    body: request.bodyBase64
      ? Buffer.from(request.bodyBase64, "base64")
      : undefined,
    headers,
    method: request.method,
    signal: AbortSignal.timeout(config.proxyTimeoutMs),
  });
  const responseHeaders = {};
  for (const [name, value] of response.headers.entries()) {
    const lower = name.toLowerCase();
    if (
      lower === "connection" ||
      lower === "content-encoding" ||
      lower === "content-length" ||
      lower === "transfer-encoding"
    ) {
      continue;
    }
    responseHeaders[name] = value;
  }
  return {
    responseBodyBase64: Buffer.from(await response.arrayBuffer()).toString(
      "base64",
    ),
    responseHeaders,
    responseStatus: response.status,
  };
}

async function completeJob(config, jobId, body) {
  return studioJson(config, "/api/actions/provider-hosts/jobs/complete", {
    ...body,
    hostId: config.hostId,
    hostToken: config.hostToken,
    jobId,
  });
}

async function completeRpc(config, requestId, body) {
  return studioJson(config, "/api/actions/provider-hosts/rpc/complete", {
    ...body,
    hostId: config.hostId,
    hostToken: config.hostToken,
    requestId,
  });
}

async function localProviderMetadata(config) {
  const [health, simulators] = await Promise.all([
    localGet(config, "/api/health"),
    localGet(config, "/api/simulators"),
  ]);
  return {
    capabilities: {
      health,
      simulators:
        simulators.simulators?.map((simulator) => ({
          isBooted: simulator.isBooted,
          name: simulator.name,
          runtimeName: simulator.runtimeName,
          udid: simulator.udid,
        })) ?? [],
    },
    ok: Boolean(health?.ok),
  };
}

async function ensureTemplateSimulator(templateName) {
  const inventory = await simulatorInventory();
  const exact = inventory.devices.find(
    (device) => device.name === templateName && device.isAvailable !== false,
  );
  if (exact) {
    return exact;
  }
  const runtime =
    inventory.runtimes.find(
      (candidate) => candidate.isAvailable && candidate.platform === "iOS",
    ) || inventory.runtimes.find((candidate) => candidate.isAvailable);
  if (!runtime) {
    throw new Error("No available iOS simulator runtime was found.");
  }
  const deviceType =
    inventory.deviceTypes.find(
      (candidate) => candidate.name === templateName,
    ) ||
    inventory.deviceTypes.find((candidate) =>
      candidate.name.includes("iPhone 17 Pro"),
    ) ||
    inventory.deviceTypes.find((candidate) =>
      candidate.name.includes("iPhone"),
    );
  if (!deviceType) {
    throw new Error(`No simulator device type was found for ${templateName}.`);
  }
  const udid = (
    await execText("xcrun", [
      "simctl",
      "create",
      templateName,
      deviceType.identifier,
      runtime.identifier,
    ])
  ).trim();
  return {
    isAvailable: true,
    name: templateName,
    runtimeName: runtime.name,
    udid,
  };
}

async function simulatorInventory() {
  const [devicesJson, deviceTypesJson, runtimesJson] = await Promise.all([
    execJson("xcrun", ["simctl", "list", "-j", "devices"]),
    execJson("xcrun", ["simctl", "list", "-j", "devicetypes"]),
    execJson("xcrun", ["simctl", "list", "-j", "runtimes"]),
  ]);
  const devices = [];
  for (const [runtimeName, runtimeDevices] of Object.entries(
    devicesJson.devices || {},
  )) {
    for (const device of runtimeDevices || []) {
      devices.push({ ...device, runtimeName });
    }
  }
  return {
    deviceTypes: deviceTypesJson.devicetypes || [],
    devices,
    runtimes: runtimesJson.runtimes || [],
  };
}

async function cloneSimulator(templateUdid, name) {
  return (
    await execText("xcrun", ["simctl", "clone", templateUdid, name])
  ).trim();
}

async function deleteSimulator(udid) {
  await execFileAsync("xcrun", ["simctl", "delete", udid], { timeout: 60000 });
}

async function bootSimulator(config, udid) {
  await localJson(
    config,
    `/api/simulators/${encodeURIComponent(udid)}/boot`,
    {},
  );
  await execFileAsync("xcrun", ["simctl", "bootstatus", udid, "-b"], {
    timeout: 600000,
  });
}

async function simulatorByUdid(config, udid) {
  const list = await localGet(config, "/api/simulators");
  return list.simulators?.find((simulator) => simulator.udid === udid) || null;
}

async function downloadAndExtractArtifact(config, sessionId) {
  const sessionRoot = path.join(config.workRoot, "sessions", sessionId);
  await fs.promises.rm(sessionRoot, { force: true, recursive: true });
  await fs.promises.mkdir(sessionRoot, { recursive: true });
  const zipPath = path.join(sessionRoot, "artifact.zip");
  const response = await fetch(
    `${config.studioUrl}/api/actions/provider-hosts/sessions/${encodeURIComponent(sessionId)}/artifact`,
    {
      headers: {
        "x-simdeck-host-id": config.hostId,
        "x-simdeck-host-token": config.hostToken,
      },
    },
  );
  if (!response.ok) {
    throw new Error(`Artifact download failed: HTTP ${response.status}`);
  }
  await fs.promises.writeFile(
    zipPath,
    Buffer.from(await response.arrayBuffer()),
  );
  await execFileAsync("ditto", ["-x", "-k", zipPath, sessionRoot], {
    timeout: 120000,
  });
  const appPath = await findAppBundle(sessionRoot);
  if (!appPath) {
    throw new Error("Artifact did not contain an .app bundle.");
  }
  return appPath;
}

async function findAppBundle(root) {
  const entries = await fs.promises.readdir(root, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(root, entry.name);
    if (entry.isDirectory() && entry.name.endsWith(".app")) {
      return full;
    }
    if (entry.isDirectory()) {
      const nested = await findAppBundle(full);
      if (nested) {
        return nested;
      }
    }
  }
  return null;
}

async function localGet(config, path) {
  const target = new URL(path, `${config.localUrl}/`);
  target.searchParams.set("simdeckToken", config.localToken);
  const response = await fetch(target, {
    headers: { "x-simdeck-token": config.localToken },
  });
  if (!response.ok) {
    throw new Error(
      `Local SimDeck GET ${path} failed: HTTP ${response.status}`,
    );
  }
  return response.json();
}

async function localJson(config, path, body) {
  const target = new URL(path, `${config.localUrl}/`);
  target.searchParams.set("simdeckToken", config.localToken);
  const response = await fetch(target, {
    body: JSON.stringify(body),
    headers: {
      "content-type": "application/json",
      "x-simdeck-token": config.localToken,
    },
    method: "POST",
  });
  if (!response.ok) {
    throw new Error(
      `Local SimDeck POST ${path} failed: HTTP ${response.status}`,
    );
  }
  return response.json();
}

async function studioJson(config, path, body) {
  const response = await fetch(`${config.studioUrl}${path}`, {
    body: JSON.stringify(body),
    headers: { "content-type": "application/json" },
    method: "POST",
  });
  if (!response.ok) {
    const message = await response.text().catch(() => "");
    throw new Error(
      `Studio request ${path} failed: HTTP ${response.status}${message ? `: ${message}` : ""}`,
    );
  }
  return response.json();
}

async function loadConfig(args) {
  const configPath = args["config"] || defaultConfigPath;
  let config = {};
  try {
    config = JSON.parse(await fs.promises.readFile(configPath, "utf8"));
  } catch {
    config = {};
  }
  config.studioUrl = (args["studio-url"] || config.studioUrl || "").replace(
    /\/$/,
    "",
  );
  config.hostId = args["host-id"] || config.hostId || "";
  config.hostToken = args["host-token"] || config.hostToken || "";
  if (!config.studioUrl || !config.hostId || !config.hostToken) {
    throw new Error(
      "Missing provider config. Run `simdeck provider connect --studio-url ... --host-id ... --host-token ...` first.",
    );
  }
  return config;
}

function simdeckBinary() {
  if (process.env.SIMDECK_BINARY) {
    return process.env.SIMDECK_BINARY;
  }
  const sourceBinary = path.join(packageRoot, "build", "simdeck");
  if (fs.existsSync(sourceBinary)) {
    return sourceBinary;
  }
  return path.join(packageRoot, "build", "simdeck-bin");
}

async function execJson(command, args) {
  return JSON.parse(await execText(command, args));
}

async function execText(command, args) {
  const { stdout } = await execFileAsync(command, args, {
    maxBuffer: 16 * 1024 * 1024,
    timeout: 120000,
  });
  return stdout;
}

async function writeJsonFile(file, value, mode) {
  await fs.promises.mkdir(path.dirname(file), { recursive: true, mode: 0o700 });
  await fs.promises.writeFile(file, `${JSON.stringify(value, null, 2)}\n`, {
    mode,
  });
  await fs.promises.chmod(file, mode);
}

function clampCapacity(value) {
  if (!Number.isFinite(value)) {
    return 1;
  }
  return Math.min(16, Math.max(1, Math.floor(value)));
}

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const item = argv[index];
    if (!item.startsWith("--")) {
      continue;
    }
    const key = item.slice(2);
    const next = argv[index + 1];
    if (!next || next.startsWith("--")) {
      args[key] = "1";
      continue;
    }
    args[key] = next;
    index += 1;
  }
  return args;
}

function requiredArg(args, name) {
  const value = args[name];
  if (!value) {
    throw new Error(`--${name} is required.`);
  }
  return value;
}

function redactConfig(config) {
  return { ...config, hostToken: config.hostToken ? "[redacted]" : "" };
}

function isWebSocketUpgradeRequest(request) {
  const headers = new Headers(request.headers || {});
  return (
    headers.get("upgrade")?.toLowerCase() === "websocket" ||
    headers
      .get("connection")
      ?.toLowerCase()
      .split(",")
      .some((value) => value.trim() === "upgrade") === true
  );
}

function describeError(error) {
  if (error instanceof Error) {
    return error.cause instanceof Error
      ? `${error.message}: ${error.cause.message}`
      : error.message;
  }
  return String(error);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function usage() {
  console.log(`Usage:
  simdeck provider connect --studio-url URL --host-id ID --host-token TOKEN
  simdeck provider run [--config PATH] [--max-capacity N]
  simdeck provider status [--config PATH]`);
}

function isMainModule() {
  return process.argv[1] === fileURLToPath(import.meta.url);
}

export {
  cloneSimulator,
  ensureTemplateSimulator,
  isWebSocketUpgradeRequest,
  parseArgs,
  redactConfig,
};
