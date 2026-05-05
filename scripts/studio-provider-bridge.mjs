#!/usr/bin/env node

import crypto from "node:crypto";
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

const cloudUrl = (
  process.env.SIMDECK_CLOUD_URL || "https://simdeck.djdev.me"
).replace(/\/$/, "");
let previewId = process.env.PREVIEW_ID || "";
let providerToken = process.env.PROVIDER_TOKEN || "";
let publicUrl = process.env.SIMDECK_STUDIO_URL || "";
let localUrl = (
  process.env.SIMDECK_LOCAL_URL || "http://127.0.0.1:4310"
).replace(/\/$/, "");
let localToken = process.env.SIMDECK_LOCAL_TOKEN || providerToken;
const registerIntervalMs = Number(
  process.env.SIMDECK_PROVIDER_REGISTER_INTERVAL_MS || 15000,
);
const maxConcurrentRequests = Math.max(
  1,
  Number(process.env.SIMDECK_PROVIDER_MAX_CONCURRENT_REQUESTS || 8),
);
const proxyTimeoutMs = Math.max(
  1000,
  Number(process.env.SIMDECK_PROVIDER_PROXY_TIMEOUT_MS || 25000),
);
const cloudRequestTimeoutMs = Math.max(
  5000,
  Number(process.env.SIMDECK_PROVIDER_CLOUD_TIMEOUT_MS || 30000),
);
const simulatorListCacheTtlMs = Math.max(
  0,
  Number(process.env.SIMDECK_PROVIDER_SIMULATORS_CACHE_MS || 5000),
);
const localUnavailableLogIntervalMs = Math.max(
  5000,
  Number(
    process.env.SIMDECK_PROVIDER_LOCAL_UNAVAILABLE_LOG_INTERVAL_MS || 30000,
  ),
);
const localUnavailableRestartMs = Math.max(
  15000,
  Number(process.env.SIMDECK_PROVIDER_LOCAL_UNAVAILABLE_RESTART_MS || 45000),
);
const providerId =
  process.env.SIMDECK_STUDIO_PROVIDER_ID || stableLocalProviderId();
const parentPid = Number(process.env.SIMDECK_PROVIDER_PARENT_PID || 0);
let localDaemonPid = Number(process.env.SIMDECK_LOCAL_DAEMON_PID || 0);
let localDaemonLog = process.env.SIMDECK_LOCAL_DAEMON_LOG || "";
const localDaemonCommand = process.env.SIMDECK_LOCAL_DAEMON_COMMAND || "";
const localDaemonRestartArgs = parseJsonArrayEnv(
  "SIMDECK_LOCAL_DAEMON_RESTART_ARGS_JSON",
);
const localDaemonStatusArgs = parseJsonArrayEnv(
  "SIMDECK_LOCAL_DAEMON_STATUS_ARGS_JSON",
) ?? ["daemon", "status"];

let stopped = false;
let lastRegisterAt = 0;
let localUnavailableSince = 0;
let lastLocalUnavailableLogAt = 0;
let lastLocalRestartAt = 0;
let localRestartInFlight = null;
let registered = false;
let providerMarkedTerminal = false;
const activeRequests = new Set();
const responseCache = new Map();
const inFlightCache = new Map();

if (isMainModule()) {
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.once(signal, () => {
      stopped = true;
    });
  }
  if (Number.isInteger(parentPid) && parentPid > 0) {
    setInterval(() => {
      try {
        process.kill(parentPid, 0);
      } catch {
        stopped = true;
      }
    }, 1000).unref();
  }

  try {
    if (!previewId || !providerToken) {
      const session = await createLocalProviderSession();
      previewId = session.sessionId;
      providerToken = session.providerToken;
      publicUrl = session.url;
    }

    if (!publicUrl) {
      publicUrl = `${cloudUrl}/simulator/${encodeURIComponent(previewId)}`;
    }
    publicUrl = normalizeStudioPublicUrl(publicUrl);
    if (!localToken) {
      localToken = providerToken;
    }

    await registerProvider();
    console.log(`[simdeck-provider-bridge] ${publicUrl}`);

    while (!stopped) {
      try {
        if (activeRequests.size >= maxConcurrentRequests) {
          await Promise.race([...activeRequests, sleep(50)]);
          continue;
        }
        if (Date.now() - lastRegisterAt > registerIntervalMs) {
          await registerProvider();
        }
        const next = await fetchJson(
          `${cloudUrl}/api/actions/providers/rpc/next`,
          {
            previewId,
            providerToken,
          },
        );
        if (!next || !next.request) {
          await sleep(250);
          continue;
        }
        runProviderRequest(next.request);
      } catch (error) {
        console.error(
          `[simdeck-provider-bridge] ${error instanceof Error ? error.message : String(error)}`,
        );
        await sleep(1000);
      }
    }
  } finally {
    if (activeRequests.size > 0) {
      await Promise.allSettled(activeRequests);
    }
    if (registered && !providerMarkedTerminal) {
      await markProviderExpired();
    }
  }
}

async function registerProvider() {
  try {
    let metadata = await localProviderMetadata();
    updateLocalAvailability(metadata);
    await maybeRestartLocalDaemon(metadata);
    if (!metadata.ok && !localUnavailableSince) {
      metadata = await localProviderMetadata();
      updateLocalAvailability(metadata);
    }
    await fetchJson(`${cloudUrl}/api/actions/providers/register`, {
      previewId,
      providerToken,
      baseUrl: publicUrl,
      status: metadata.ok ? "ready" : "provider-online",
      simulatorUdid: metadata.simulator?.udid,
      simulatorName: metadata.simulator?.name,
      runtimeName: metadata.simulator?.runtimeName,
      videoCodec: metadata.health?.videoCodec,
      realtimeStream: metadata.health?.realtimeStream,
      streamQuality: metadata.health?.streamQuality,
    });
    registered = true;
    lastRegisterAt = Date.now();
    if (shouldStopForLocalMetadata(metadata, localDaemonProcessExited())) {
      providerMarkedTerminal = true;
      stopped = true;
      await markProviderFailed(
        metadata.failureReason ||
          "Local SimDeck daemon supervisor process exited.",
      );
    }
  } catch (error) {
    console.error(
      `[simdeck-provider-bridge] provider registration failed: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

export function shouldStopForLocalMetadata(metadata, daemonProcessExited) {
  return !metadata.ok && daemonProcessExited;
}

function localDaemonProcessExited() {
  if (
    Number.isInteger(localDaemonPid) &&
    localDaemonPid > 0 &&
    !processIsRunning(localDaemonPid)
  ) {
    console.error(
      `[simdeck-provider-bridge] local SimDeck daemon process ${localDaemonPid} is no longer running.`,
    );
    printRecentDaemonLog();
    return true;
  }
  return false;
}

function updateLocalAvailability(metadata) {
  if (metadata.ok) {
    localUnavailableSince = 0;
    return;
  }
  localUnavailableSince ||= Date.now();
  const elapsed = Date.now() - localUnavailableSince;
  if (Date.now() - lastLocalUnavailableLogAt >= localUnavailableLogIntervalMs) {
    lastLocalUnavailableLogAt = Date.now();
    console.error(
      `[simdeck-provider-bridge] local SimDeck HTTP unavailable for ${elapsed}ms while daemon supervisor is still running; keeping Studio bridge alive.`,
    );
    if (metadata.failureReason) {
      console.error(`[simdeck-provider-bridge] ${metadata.failureReason}`);
    }
    printRecentDaemonLog();
  }
}

function processIsRunning(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function printRecentDaemonLog() {
  const lines = recentDaemonLogLines();
  if (!lines) {
    return;
  }
  console.error("[simdeck-provider-bridge] recent daemon log:");
  console.error(lines);
}

function recentDaemonLogLines() {
  if (!localDaemonLog) {
    return "";
  }
  try {
    const data = fs.readFileSync(localDaemonLog, "utf8");
    return data.split(/\r?\n/).filter(Boolean).slice(-20).join("\n");
  } catch {
    return "";
  }
}

async function maybeRestartLocalDaemon(metadata) {
  if (metadata.ok || stopped || !localUnavailableSince) {
    return;
  }
  if (!localDaemonCommand || !localDaemonRestartArgs) {
    return;
  }
  const elapsed = Date.now() - localUnavailableSince;
  if (elapsed < localUnavailableRestartMs) {
    return;
  }
  if (localRestartInFlight) {
    await localRestartInFlight;
    return;
  }
  if (Date.now() - lastLocalRestartAt < localUnavailableRestartMs) {
    return;
  }

  lastLocalRestartAt = Date.now();
  localRestartInFlight = restartLocalDaemon()
    .catch((error) => {
      console.error(
        `[simdeck-provider-bridge] local SimDeck daemon restart failed: ${describeError(error)}`,
      );
    })
    .finally(() => {
      localRestartInFlight = null;
    });
  await localRestartInFlight;
}

async function restartLocalDaemon() {
  console.error(
    `[simdeck-provider-bridge] local SimDeck HTTP has been unavailable for ${Date.now() - localUnavailableSince}ms; restarting local daemon.`,
  );
  printRecentDaemonLog();
  await execFileAsync(localDaemonCommand, localDaemonRestartArgs, {
    timeout: 90_000,
    windowsHide: true,
  });
  const { stdout } = await execFileAsync(
    localDaemonCommand,
    localDaemonStatusArgs,
    {
      timeout: 15_000,
      windowsHide: true,
    },
  );
  const status = JSON.parse(stdout);
  const daemon = status.daemon ?? status;
  if (daemon.httpUrl) {
    localUrl = String(daemon.httpUrl).replace(/\/$/, "");
  }
  if (daemon.accessToken) {
    localToken = String(daemon.accessToken);
  }
  if (daemon.pid) {
    localDaemonPid = Number(daemon.pid);
  }
  if (daemon.logPath) {
    localDaemonLog = String(daemon.logPath);
  }
  responseCache.clear();
  inFlightCache.clear();
  localUnavailableSince = 0;
  lastLocalUnavailableLogAt = 0;
  console.error(
    `[simdeck-provider-bridge] local SimDeck daemon restarted at ${localUrl}.`,
  );
}

async function createLocalProviderSession() {
  const response = await fetchJson(`${cloudUrl}/api/local-provider-sessions`, {
    providerId,
    simulatorName: process.env.SIMDECK_STUDIO_SIMULATOR_NAME,
    runtimeName: process.env.SIMDECK_STUDIO_RUNTIME_NAME,
  });
  if (!response?.sessionId || !response?.providerToken || !response?.url) {
    throw new Error("Studio did not return a local provider session.");
  }
  return response;
}

async function markProviderExpired() {
  await markProviderStatus("expired");
}

async function markProviderFailed(reason) {
  if (reason) {
    console.error(`[simdeck-provider-bridge] ${reason}`);
  }
  await markProviderStatus("failed");
}

async function markProviderStatus(status) {
  try {
    await fetchJson(`${cloudUrl}/api/actions/providers/register`, {
      previewId,
      providerToken,
      baseUrl: publicUrl,
      status,
    });
  } catch (error) {
    console.error(
      `[simdeck-provider-bridge] provider ${status} update failed: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

async function localProviderMetadata() {
  let health = null;
  let healthError = null;
  try {
    health = await localJson("/api/health");
  } catch (error) {
    healthError = error;
    health = null;
  }

  try {
    const simulators = await localJson("/api/simulators");
    const selected =
      simulators.simulators?.find((simulator) => simulator.isBooted) ??
      simulators.simulators?.[0] ??
      null;
    return { health, ok: true, simulator: selected };
  } catch (error) {
    if (health) {
      return { health, ok: true, simulator: null };
    }
    return {
      failureReason: localProviderFailureReason(healthError, error),
      health: null,
      ok: false,
      simulator: null,
    };
  }
}

function localProviderFailureReason(healthError, simulatorError) {
  const healthMessage = describeError(healthError);
  const simulatorMessage = describeError(simulatorError);
  return [healthMessage, simulatorMessage].filter(Boolean).join("; ");
}

function describeError(error) {
  if (!error) {
    return "";
  }
  if (!(error instanceof Error)) {
    return String(error);
  }
  const cause = error.cause;
  if (cause instanceof Error && cause.message) {
    return `${error.message}: ${cause.message}`;
  }
  return error.message;
}

async function handleRequest(request) {
  let responsePayload;
  if (isWebSocketUpgradeRequest(request)) {
    await complete({
      requestId: request.id,
      responseBodyBase64: Buffer.from(
        "Studio provider RPC does not tunnel WebSocket upgrade requests.",
      ).toString("base64"),
      responseHeaders: { "content-type": "text/plain; charset=utf-8" },
      responseStatus: 426,
    });
    return;
  }
  try {
    responsePayload = await cachedProxyResponse(request);
  } catch (error) {
    console.error(
      `[simdeck-provider-bridge] request ${request.id} ${request.method} ${request.path} failed: ${describeError(error)}`,
    );
    await handleLocalProxyFailure(error);
    if (request.method !== "GET") {
      await complete({
        requestId: request.id,
        error: describeError(error),
      });
      return;
    }
    try {
      responsePayload = await proxyLocalRequest(request);
    } catch (retryError) {
      console.error(
        `[simdeck-provider-bridge] request ${request.id} ${request.method} ${request.path} retry failed: ${describeError(retryError)}`,
      );
      await complete({
        requestId: request.id,
        error: describeError(retryError),
      });
      return;
    }
  }

  try {
    await complete({
      requestId: request.id,
      ...responsePayload,
    });
  } catch (error) {
    console.error(
      `[simdeck-provider-bridge] request ${request.id} ${request.method} ${request.path} completion failed: ${describeError(error)}`,
    );
    throw error;
  }
}

function runProviderRequest(request) {
  const task = handleRequest(request).catch((error) => {
    console.error(
      `[simdeck-provider-bridge] request ${request.id} failed: ${error instanceof Error ? error.message : String(error)}`,
    );
  });
  activeRequests.add(task);
  task.finally(() => {
    activeRequests.delete(task);
  });
}

async function cachedProxyResponse(request) {
  const cacheKey = cacheKeyForRequest(request);
  if (!cacheKey || simulatorListCacheTtlMs <= 0) {
    return proxyLocalRequest(request);
  }

  const cached = responseCache.get(cacheKey);
  if (cached && Date.now() - cached.updatedAt <= simulatorListCacheTtlMs) {
    return cached.payload;
  }

  const pending = inFlightCache.get(cacheKey);
  if (pending) {
    return pending;
  }

  const pendingRequest = proxyLocalRequest(request)
    .then((payload) => {
      if (payload.responseStatus >= 200 && payload.responseStatus < 300) {
        responseCache.set(cacheKey, { payload, updatedAt: Date.now() });
      }
      return payload;
    })
    .finally(() => {
      inFlightCache.delete(cacheKey);
    });
  inFlightCache.set(cacheKey, pendingRequest);
  return pendingRequest;
}

function cacheKeyForRequest(request) {
  if (request.method !== "GET") {
    return "";
  }
  const target = new URL(request.path, `${localUrl}/`);
  target.searchParams.delete("simdeckToken");
  if (target.pathname !== "/api/simulators") {
    return "";
  }
  return `${target.pathname}?${target.searchParams.toString()}`;
}

export function isWebSocketUpgradeRequest(request) {
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

async function proxyLocalRequest(request) {
  const target = new URL(request.path, `${localUrl}/`);
  if (!target.searchParams.has("simdeckToken")) {
    target.searchParams.set("simdeckToken", localToken);
  }
  const headers = new Headers(request.headers || {});
  headers.set("x-simdeck-token", localToken);
  headers.delete("host");
  headers.delete("content-length");
  let response;
  try {
    response = await fetch(target, {
      body: request.bodyBase64
        ? Buffer.from(request.bodyBase64, "base64")
        : undefined,
      headers,
      method: request.method,
      signal: AbortSignal.timeout(proxyTimeoutMs),
    });
  } catch (error) {
    throw new Error(
      `Local SimDeck request ${target.origin}${target.pathname} failed`,
      {
        cause: error,
      },
    );
  }
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
  const responseBodyBase64 = Buffer.from(await response.arrayBuffer()).toString(
    "base64",
  );
  return {
    responseStatus: response.status,
    responseHeaders,
    responseBodyBase64,
  };
}

async function handleLocalProxyFailure(error) {
  const message = describeError(error);
  if (!message.includes("Local SimDeck request")) {
    return;
  }
  updateLocalAvailability({
    failureReason: message,
    health: null,
    ok: false,
    simulator: null,
  });
  await maybeRestartLocalDaemon({
    failureReason: message,
    health: null,
    ok: false,
    simulator: null,
  });
}

async function localJson(path) {
  const target = new URL(path, `${localUrl}/`);
  target.searchParams.set("simdeckToken", localToken);
  let response;
  try {
    response = await fetch(target, {
      headers: { "x-simdeck-token": localToken },
      signal: AbortSignal.timeout(Math.min(proxyTimeoutMs, 5000)),
    });
  } catch (error) {
    throw new Error(`Local SimDeck request ${target.origin}${path} failed`, {
      cause: error,
    });
  }
  if (!response.ok) {
    throw new Error(
      `${target.href} failed with ${response.status}: ${await response.text()}`,
    );
  }
  return response.json();
}

async function complete(payload) {
  await fetchJson(`${cloudUrl}/api/actions/providers/rpc/complete`, {
    previewId,
    providerToken,
    ...payload,
  });
}

async function fetchJson(url, body) {
  let response;
  try {
    response = await fetch(url, {
      body: JSON.stringify(body),
      headers: { "content-type": "application/json" },
      method: "POST",
      signal: AbortSignal.timeout(cloudRequestTimeoutMs),
    });
  } catch (error) {
    throw new Error(`Studio request ${url} failed`, { cause: error });
  }
  if (response.status === 204) {
    return null;
  }
  if (!response.ok) {
    throw new Error(
      `${url} failed with ${response.status}: ${await response.text()}`,
    );
  }
  return response.json();
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function normalizeStudioPublicUrl(value) {
  return normalizeStudioPublicUrlWithCloud(value, cloudUrl);
}

export function normalizeStudioPublicUrlWithCloud(value, baseCloudUrl) {
  const trimmed = String(value || "").trim();
  if (!trimmed) {
    return "";
  }

  const normalizedCloudUrl = baseCloudUrl.replace(/\/$/, "");
  const cloudOrigin = new URL(normalizedCloudUrl).origin;
  const collapsed = trimmed
    .replace(repeatedPrefixPattern(normalizedCloudUrl), normalizedCloudUrl)
    .replace(repeatedPrefixPattern(cloudOrigin), cloudOrigin);
  return new URL(collapsed, `${normalizedCloudUrl}/`).toString();
}

function repeatedPrefixPattern(prefix) {
  return new RegExp(`^(?:${escapeRegExp(prefix)}){2,}`);
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function parseJsonArrayEnv(name) {
  const raw = process.env[name];
  if (!raw) {
    return null;
  }
  try {
    const value = JSON.parse(raw);
    if (
      Array.isArray(value) &&
      value.every((item) => typeof item === "string")
    ) {
      return value;
    }
  } catch {
    return null;
  }
  return null;
}

function isMainModule() {
  return import.meta.url === pathToFileURL(process.argv[1] || "").href;
}

function stableLocalProviderId() {
  const fingerprint = [
    os.hostname(),
    localUrl,
    process.env.SIMDECK_STUDIO_SIMULATOR_UDID || "",
  ].join("\n");
  return `local-${crypto.createHash("sha256").update(fingerprint).digest("hex").slice(0, 24)}`;
}
