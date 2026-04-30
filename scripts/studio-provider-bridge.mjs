#!/usr/bin/env node

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

let stopped = false;
let lastRegisterAt = 0;
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.once(signal, () => {
    stopped = true;
  });
}

if (!previewId || !providerToken) {
  const session = await createLocalProviderSession();
  previewId = session.sessionId;
  providerToken = session.providerToken;
  publicUrl = session.url;
}

if (!publicUrl) {
  publicUrl = `${cloudUrl}/simulator/${encodeURIComponent(previewId)}`;
}
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
    const next = await fetchJson(`${cloudUrl}/api/actions/providers/rpc/next`, {
      previewId,
      providerToken,
    });
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
    lastRegisterAt = Date.now();
  } catch (error) {
    console.error(
      `[simdeck-provider-bridge] provider registration failed: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

async function createLocalProviderSession() {
  const response = await fetchJson(`${cloudUrl}/api/local-provider-sessions`, {
    simulatorName: process.env.SIMDECK_STUDIO_SIMULATOR_NAME,
    runtimeName: process.env.SIMDECK_STUDIO_RUNTIME_NAME,
  });
  if (!response?.sessionId || !response?.providerToken || !response?.url) {
    throw new Error("Studio did not return a local provider session.");
  }
  return response;
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
