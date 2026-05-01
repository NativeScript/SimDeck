#!/usr/bin/env node
import { execFileSync } from "node:child_process";

const args = parseArgs(process.argv.slice(2));
const serverUrl = String(
  args["server-url"] ??
    process.env.SIMDECK_STRESS_SERVER_URL ??
    "http://127.0.0.1:4310",
).replace(/\/$/, "");
const iterations = positiveInt(
  args.iterations ?? process.env.SIMDECK_STRESS_ITERATIONS,
  500,
);
const concurrency = positiveInt(
  args.concurrency ?? process.env.SIMDECK_STRESS_CONCURRENCY,
  8,
);
const sampleEvery = positiveInt(
  args["sample-every"] ?? process.env.SIMDECK_STRESS_SAMPLE_EVERY,
  25,
);
const maxRssMb = optionalNumber(
  args["max-rss-mb"] ?? process.env.SIMDECK_STRESS_MAX_RSS_MB,
);
const maxRssGrowthMb =
  optionalNumber(
    args["max-rss-growth-mb"] ?? process.env.SIMDECK_STRESS_MAX_RSS_GROWTH_MB,
  ) ?? 256;
const udid = args.udid ?? process.env.SIMDECK_STRESS_UDID;
const mutating = booleanArg(
  args.mutating ?? process.env.SIMDECK_STRESS_MUTATING,
);
const pid =
  optionalInt(args.pid ?? process.env.SIMDECK_STRESS_PID) ??
  discoverListenerPid(serverUrl);

const samples = [];
const failures = [];
let completed = 0;
let nextIndex = 0;

if (!pid) {
  console.warn(
    "Unable to discover SimDeck PID; RSS leak checks will be skipped. Pass --pid to enable them.",
  );
} else {
  sampleRss("start");
}

await assertHealthy();

const startedAt = Date.now();
await Promise.all(
  Array.from({ length: concurrency }, async () => {
    while (true) {
      const index = nextIndex++;
      if (index >= iterations) {
        return;
      }
      try {
        await runIteration(index);
      } catch (error) {
        failures.push({
          index,
          error: error instanceof Error ? error.message : String(error),
        });
      } finally {
        completed += 1;
        if (completed % sampleEvery === 0) {
          sampleRss(`iteration-${completed}`);
        }
      }
    }
  }),
);

sampleRss("end");

const elapsedSeconds = (Date.now() - startedAt) / 1000;
const firstRss = samples.find((sample) => sample.rssMb != null)?.rssMb;
const lastRss = [...samples]
  .reverse()
  .find((sample) => sample.rssMb != null)?.rssMb;
const peakRss = samples.reduce(
  (peak, sample) => Math.max(peak, sample.rssMb ?? 0),
  0,
);
const rssGrowth =
  firstRss != null && lastRss != null ? lastRss - firstRss : null;

const summary = {
  ok: failures.length === 0,
  serverUrl,
  pid,
  iterations,
  concurrency,
  completed,
  failures: failures.slice(0, 10),
  elapsedSeconds: Number(elapsedSeconds.toFixed(2)),
  requestsPerSecond: Number(
    (completed / Math.max(elapsedSeconds, 0.001)).toFixed(2),
  ),
  rss: {
    startMb: firstRss,
    endMb: lastRss,
    peakMb: peakRss || null,
    growthMb: rssGrowth == null ? null : Number(rssGrowth.toFixed(2)),
    samples,
  },
};

if (maxRssMb != null && peakRss > maxRssMb) {
  summary.ok = false;
  failures.push({
    index: -1,
    error: `Peak RSS ${peakRss.toFixed(2)} MB exceeded ${maxRssMb} MB`,
  });
}
if (rssGrowth != null && rssGrowth > maxRssGrowthMb) {
  summary.ok = false;
  failures.push({
    index: -1,
    error: `RSS growth ${rssGrowth.toFixed(2)} MB exceeded ${maxRssGrowthMb} MB`,
  });
}
summary.failures = failures.slice(0, 10);

console.log(JSON.stringify(summary, null, 2));
if (!summary.ok) {
  process.exit(1);
}

async function runIteration(index) {
  const endpoints = [
    ["GET", "/api/health"],
    ["GET", "/api/metrics"],
    ["GET", "/api/simulators"],
    ["GET", "/api/stream-quality"],
  ];
  if (udid) {
    endpoints.push(["GET", `/api/simulators/${encodeURIComponent(udid)}`]);
    if (index % 5 === 0) {
      endpoints.push([
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/stream/refresh`,
        {},
      ]);
    }
    if (mutating && index % 10 === 0) {
      endpoints.push([
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/touch`,
        { x: 0.5, y: 0.5, phase: "moved" },
      ]);
    }
  }

  const [method, path, body] = endpoints[index % endpoints.length];
  await request(method, path, body);
}

async function assertHealthy() {
  const health = await request("GET", "/api/health");
  if (health?.ok !== true) {
    throw new Error("SimDeck health endpoint did not return ok=true");
  }
}

async function request(method, path, body) {
  const response = await fetch(`${serverUrl}${path}`, {
    method,
    headers: body ? { "content-type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(
      `${method} ${path} failed with ${response.status}: ${text.slice(0, 500)}`,
    );
  }
  if (!text) {
    return null;
  }
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function sampleRss(label) {
  if (!pid) {
    return;
  }
  const rssKb = rssKbForPid(pid);
  if (rssKb == null) {
    failures.push({ index: -1, error: `Unable to sample RSS for pid ${pid}` });
    return;
  }
  samples.push({
    label,
    completed,
    rssMb: Number((rssKb / 1024).toFixed(2)),
  });
}

function discoverListenerPid(url) {
  const port =
    new URL(url).port || (new URL(url).protocol === "https:" ? "443" : "80");
  try {
    const output = execFileSync(
      "lsof",
      ["-nP", "-ti", `tcp:${port}`, "-sTCP:LISTEN"],
      {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      },
    );
    return optionalInt(output.trim().split(/\s+/)[0]);
  } catch {
    return null;
  }
}

function rssKbForPid(value) {
  try {
    const output = execFileSync("ps", ["-o", "rss=", "-p", String(value)], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    return optionalInt(output);
  } catch {
    return null;
  }
}

function parseArgs(values) {
  const parsed = {};
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (!value.startsWith("--")) {
      continue;
    }
    const [rawKey, inlineValue] = value.slice(2).split("=", 2);
    if (inlineValue != null) {
      parsed[rawKey] = inlineValue;
    } else if (values[index + 1] && !values[index + 1].startsWith("--")) {
      parsed[rawKey] = values[index + 1];
      index += 1;
    } else {
      parsed[rawKey] = "true";
    }
  }
  return parsed;
}

function positiveInt(value, fallback) {
  const parsed = optionalInt(value);
  return parsed && parsed > 0 ? parsed : fallback;
}

function optionalInt(value) {
  if (value == null || value === "") {
    return null;
  }
  const parsed = Number.parseInt(String(value), 10);
  return Number.isFinite(parsed) ? parsed : null;
}

function optionalNumber(value) {
  if (value == null || value === "") {
    return null;
  }
  const parsed = Number.parseFloat(String(value));
  return Number.isFinite(parsed) ? parsed : null;
}

function booleanArg(value) {
  return ["1", "true", "yes", "on"].includes(
    String(value ?? "")
      .trim()
      .toLowerCase(),
  );
}
