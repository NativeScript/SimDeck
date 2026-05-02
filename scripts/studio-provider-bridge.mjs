#!/usr/bin/env node

import crypto from "node:crypto";
import os from "node:os";
import { pathToFileURL } from "node:url";

const cloudUrl = (
  process.env.SIMDECK_CLOUD_URL || "https://simdeck.djdev.me"
).replace(/\/$/, "");
let previewId = process.env.PREVIEW_ID || "";
let providerToken = process.env.PROVIDER_TOKEN || "";
let publicUrl = process.env.SIMDECK_STUDIO_URL || "";
const localUrl = (
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
const simulatorListCacheTtlMs = Math.max(
  0,
  Number(process.env.SIMDECK_PROVIDER_SIMULATORS_CACHE_MS || 5000),
);
const providerId =
  process.env.SIMDECK_STUDIO_PROVIDER_ID || stableLocalProviderId();

let stopped = false;
let lastRegisterAt = 0;
let registered = false;
const activeRequests = new Set();
const responseCache = new Map();
const inFlightCache = new Map();

if (isMainModule()) {
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.once(signal, () => {
      stopped = true;
    });
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
    if (registered) {
      await markProviderExpired();
    }
  }
}

async function registerProvider() {
  try {
    const metadata = await localProviderMetadata();
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
  } catch (error) {
    console.error(
      `[simdeck-provider-bridge] provider registration failed: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
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
  try {
    await fetchJson(`${cloudUrl}/api/actions/providers/register`, {
      previewId,
      providerToken,
      baseUrl: publicUrl,
      status: "expired",
    });
  } catch (error) {
    console.error(
      `[simdeck-provider-bridge] provider expiration failed: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

async function localProviderMetadata() {
  let health = null;
  try {
    health = await localJson("/api/health");
  } catch {
    health = null;
  }

  try {
    const simulators = await localJson("/api/simulators");
    const selected =
      simulators.simulators?.find((simulator) => simulator.isBooted) ??
      simulators.simulators?.[0] ??
      null;
    return { health, ok: true, simulator: selected };
  } catch {
    if (health) {
      return { health, ok: true, simulator: null };
    }
    return { health: null, ok: false, simulator: null };
  }
}

async function handleRequest(request) {
  try {
    const responsePayload = await cachedProxyResponse(request);
    await complete({
      requestId: request.id,
      ...responsePayload,
    });
  } catch (error) {
    await complete({
      requestId: request.id,
      error: error instanceof Error ? error.message : String(error),
    });
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

async function proxyLocalRequest(request) {
  const target = new URL(request.path, `${localUrl}/`);
  if (!target.searchParams.has("simdeckToken")) {
    target.searchParams.set("simdeckToken", localToken);
  }
  const headers = new Headers(request.headers || {});
  headers.set("x-simdeck-token", localToken);
  headers.delete("host");
  headers.delete("content-length");
  const response = await fetch(target, {
    body: request.bodyBase64
      ? Buffer.from(request.bodyBase64, "base64")
      : undefined,
    headers,
    method: request.method,
    signal: AbortSignal.timeout(proxyTimeoutMs),
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
  const responseBodyBase64 = Buffer.from(await response.arrayBuffer()).toString(
    "base64",
  );
  return {
    responseStatus: response.status,
    responseHeaders,
    responseBodyBase64,
  };
}

async function localJson(path) {
  const target = new URL(path, `${localUrl}/`);
  target.searchParams.set("simdeckToken", localToken);
  const response = await fetch(target, {
    headers: { "x-simdeck-token": localToken },
    signal: AbortSignal.timeout(Math.min(proxyTimeoutMs, 5000)),
  });
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
  const response = await fetch(url, {
    body: JSON.stringify(body),
    headers: { "content-type": "application/json" },
    method: "POST",
  });
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
