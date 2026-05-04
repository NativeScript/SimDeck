#!/usr/bin/env node

import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const viewerUrl = new URL(
  process.env.SIMDECK_STRESS_VIEWER_URL ??
    process.argv[2] ??
    "http://127.0.0.1:4310",
);
const durationMs = positiveInt(
  process.env.SIMDECK_STRESS_WEBRTC_MS ?? process.argv[3],
  60_000,
);
const totalClients = positiveInt(process.env.SIMDECK_STRESS_CLIENTS, 10);
const steadyClients = Math.min(
  totalClients,
  positiveInt(process.env.SIMDECK_STRESS_STEADY_CLIENTS, 5),
);
const churnClients = Math.max(0, totalClients - steadyClients);
const churnSessionMs = positiveInt(
  process.env.SIMDECK_STRESS_CHURN_SESSION_MS,
  12_000,
);
const chromePortBase = positiveInt(
  process.env.SIMDECK_STRESS_CHROME_PORT_BASE,
  9400,
);
const maxPeerDisconnectedMs = positiveInt(
  process.env.SIMDECK_STRESS_MAX_PEER_DISCONNECTED_MS,
  3000,
);
const maxDecoderDrops = positiveInt(
  process.env.SIMDECK_STRESS_MAX_DECODER_DROPS,
  0,
);
const settleMs = positiveInt(process.env.SIMDECK_STRESS_SETTLE_MS, 15_000);
const childScript = new URL("./e2e-webrtc-reliability.mjs", import.meta.url);
const childScriptPath = fileURLToPath(childScript);
const outputDir = await mkdtemp(join(tmpdir(), "simdeck-webrtc-stress-"));

const startedAt = Date.now();
const initialMetrics = await fetchMetrics().catch((error) => ({
  error: String(error?.message ?? error),
}));
const workers = [];

for (let index = 0; index < steadyClients; index += 1) {
  workers.push(
    runViewer({
      label: `steady-${index + 1}`,
      durationMs,
      port: chromePortBase + index,
    }),
  );
}

for (let index = 0; index < churnClients; index += 1) {
  workers.push(
    runChurnViewer({
      index,
      label: `churn-${index + 1}`,
      port: chromePortBase + steadyClients + index,
    }),
  );
}

const results = await Promise.all(workers);
await sleep(settleMs);
const finalMetrics = await fetchMetrics().catch((error) => ({
  error: String(error?.message ?? error),
}));
const failures = results.flatMap((result) => result.failures);
const activeStreamLeak = Math.max(
  0,
  numeric(finalMetrics.active_streams) - numeric(initialMetrics.active_streams),
);
if (activeStreamLeak > 0) {
  failures.push(
    `active stream count did not return to baseline after ${settleMs}ms: initial=${numeric(
      initialMetrics.active_streams,
    )} final=${numeric(finalMetrics.active_streams)}`,
  );
}
const completedRuns = results.reduce(
  (sum, result) => sum + result.runs.length,
  0,
);
const successfulRuns = results.reduce(
  (sum, result) => sum + result.runs.filter((run) => run.ok).length,
  0,
);
const reconnects = results.reduce(
  (sum, result) =>
    sum +
    result.runs.reduce(
      (innerSum, run) => innerSum + numeric(run.summary?.reconnectDelta),
      0,
    ),
  0,
);
const decoderDrops = results.reduce(
  (sum, result) =>
    sum +
    result.runs.reduce((innerSum, run) => {
      const directDroppedDelta = Math.max(
        0,
        numeric(run.summary?.directStatsEnd?.framesDropped) -
          numeric(run.summary?.directStatsStart?.framesDropped),
      );
      return (
        innerSum +
        Math.max(numeric(run.summary?.droppedDelta), directDroppedDelta)
      );
    }, 0),
  0,
);
const elapsedMs = Date.now() - startedAt;
const summary = {
  ok: failures.length === 0,
  viewerUrl: viewerUrl.toString(),
  durationMs,
  totalClients,
  steadyClients,
  churnClients,
  churnSessionMs,
  settleMs,
  completedRuns,
  successfulRuns,
  reconnects,
  decoderDrops,
  maxDecoderDrops,
  activeStreamLeak,
  elapsedMs,
  initialMetrics,
  finalMetrics,
  results: results.map((result) => ({
    label: result.label,
    runs: result.runs.map((run) => ({
      ok: run.ok,
      exitCode: run.exitCode,
      durationMs: run.durationMs,
      reconnectDelta: run.summary?.reconnectDelta,
      decodedDelta: run.summary?.decodedDelta,
      receivedDelta: run.summary?.receivedDelta,
      droppedDelta: run.summary?.droppedDelta,
      maxObservedFrameGapMs: run.summary?.maxObservedFrameGapMs,
      maxPeerDisconnectedObservedMs: run.summary?.maxPeerDisconnectedObservedMs,
    })),
    failures: result.failures,
  })),
  failures,
};

console.log(JSON.stringify(summary, null, 2));
await rm(outputDir, { force: true, recursive: true });
if (!summary.ok) {
  process.exit(1);
}

async function runChurnViewer({ index, label, port }) {
  const runs = [];
  const failures = [];
  let iteration = 0;
  while (Date.now() - startedAt < durationMs) {
    iteration += 1;
    const remaining = durationMs - (Date.now() - startedAt);
    if (remaining < 3000) {
      break;
    }
    const runDurationMs = Math.min(churnSessionMs, remaining);
    const run = await runViewerOnce({
      label: `${label}-${iteration}`,
      durationMs: runDurationMs,
      port: port + iteration * totalClients,
    });
    runs.push(run);
    if (!run.ok) {
      failures.push(`${label}-${iteration}: ${run.failure}`);
    }
    await sleep(250 + index * 50);
  }
  return { label, runs, failures };
}

async function runViewer({ label, durationMs, port }) {
  const run = await runViewerOnce({ label, durationMs, port });
  return {
    label,
    runs: [run],
    failures: run.ok ? [] : [`${label}: ${run.failure}`],
  };
}

function runViewerOnce({ label, durationMs, port }) {
  const started = Date.now();
  return new Promise((resolve) => {
    let stdout = "";
    let stderr = "";
    const outputJsonPath = join(outputDir, `${label}.json`);
    const child = spawn(
      process.execPath,
      [childScriptPath, viewerUrl.toString(), String(durationMs)],
      {
        env: {
          ...process.env,
          SIMDECK_E2E_CHROME_PORT: String(port),
          SIMDECK_E2E_INTERACTIONS: "0",
          SIMDECK_E2E_MAX_PEER_DISCONNECTED_MS: String(maxPeerDisconnectedMs),
          SIMDECK_E2E_MAX_DECODER_DROPS: String(maxDecoderDrops),
          SIMDECK_E2E_OUTPUT_JSON: outputJsonPath,
          SIMDECK_E2E_REQUIRE_VISUAL: "0",
          SIMDECK_E2E_VISUAL_SAMPLE_INTERVAL_MS:
            process.env.SIMDECK_E2E_VISUAL_SAMPLE_INTERVAL_MS ?? "0",
          SIMDECK_E2E_WEBRTC_MS: String(durationMs),
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    const timeout = setTimeout(() => {
      child.kill("SIGTERM");
      setTimeout(() => {
        child.kill("SIGKILL");
      }, 5000).unref();
    }, durationMs + 90_000).unref();
    child.on("close", async (exitCode, signal) => {
      clearTimeout(timeout);
      const summary =
        (await readSummaryFile(outputJsonPath)) ?? parseLastJsonObject(stdout);
      const ok = exitCode === 0 && summary?.ok === true;
      resolve({
        ok,
        label,
        exitCode,
        signal,
        durationMs: Date.now() - started,
        summary,
        failure: ok
          ? ""
          : summary?.failures?.join("; ") ||
            stderr.trim().slice(-1000) ||
            stdout.trim().slice(-1000) ||
            `viewer exited with ${exitCode ?? signal}`,
      });
    });
  });
}

async function readSummaryFile(path) {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch {
    return null;
  }
}

function parseLastJsonObject(output) {
  const first = output.indexOf("{");
  const last = output.lastIndexOf("}");
  if (first >= 0 && last > first) {
    try {
      return JSON.parse(output.slice(first, last + 1));
    } catch {
      // Fall back to scanning below when stdout contains extra braces.
    }
  }
  for (
    let index = output.lastIndexOf("{");
    index >= 0;
    index = output.lastIndexOf("{", index - 1)
  ) {
    try {
      return JSON.parse(output.slice(index));
    } catch {
      continue;
    }
  }
  return null;
}

async function fetchMetrics() {
  const response = await fetch(endpoint("/api/metrics"), {
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) {
    throw new Error(
      `metrics returned ${response.status}: ${await response.text()}`,
    );
  }
  return response.json();
}

function endpoint(path) {
  return new URL(
    `${apiRootPathForViewerUrl(viewerUrl)}${path}`,
    viewerUrl,
  ).toString();
}

function apiRootPathForViewerUrl(url) {
  const match = url.pathname.match(/^\/simulator\/([^/]+)/);
  if (!match) {
    return "";
  }
  return `/api/provider-sessions/${match[1]}/simdeck`;
}

function positiveInt(value, fallback) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

function numeric(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
