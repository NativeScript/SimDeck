#!/usr/bin/env node

const cloudUrl = requiredEnv("SIMDECK_CLOUD_URL").replace(/\/$/, "");
const previewId = requiredEnv("PREVIEW_ID");
const providerToken = requiredEnv("PROVIDER_TOKEN");
const localUrl = (
  process.env.SIMDECK_LOCAL_URL || "http://127.0.0.1:4310"
).replace(/\/$/, "");
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
      baseUrl: `${cloudUrl}/simulator/${encodeURIComponent(previewId)}`,
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
      target.searchParams.set("simdeckToken", providerToken);
    }
    const headers = new Headers(request.headers || {});
    headers.set("x-simdeck-token", providerToken);
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
  target.searchParams.set("simdeckToken", providerToken);
  const response = await fetch(target, {
    headers: { "x-simdeck-token": providerToken },
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

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required.`);
  }
  return value;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
