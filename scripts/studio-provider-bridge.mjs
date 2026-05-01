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
const providerId =
  process.env.SIMDECK_STUDIO_PROVIDER_ID || stableLocalProviderId();

let stopped = false;
let lastRegisterAt = 0;
let registered = false;

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

    console.log(`[simdeck-provider-bridge] ${publicUrl}`);

    await registerProvider();

    while (!stopped) {
      try {
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
        await handleRequest(next.request);
      } catch (error) {
        console.error(
          `[simdeck-provider-bridge] ${error instanceof Error ? error.message : String(error)}`,
        );
        await sleep(1000);
      }
    }
  } finally {
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
  try {
    const simulators = await localJson("/api/simulators");
    const selected =
      simulators.simulators?.find((simulator) => simulator.isBooted) ??
      simulators.simulators?.[0] ??
      null;
    return { ok: true, simulator: selected };
  } catch {
    try {
      await localJson("/api/health");
      return { ok: true, simulator: null };
    } catch {
      return { ok: false, simulator: null };
    }
  }
}

async function handleRequest(request) {
  try {
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
    const responseBodyBase64 = Buffer.from(
      await response.arrayBuffer(),
    ).toString("base64");
    await complete({
      requestId: request.id,
      responseStatus: response.status,
      responseHeaders,
      responseBodyBase64,
    });
  } catch (error) {
    await complete({
      requestId: request.id,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

async function localJson(path) {
  const target = new URL(path, `${localUrl}/`);
  target.searchParams.set("simdeckToken", localToken);
  const response = await fetch(target, {
    headers: { "x-simdeck-token": localToken },
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
