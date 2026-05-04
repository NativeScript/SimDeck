#!/usr/bin/env node

const serverUrl = new URL(
  process.env.SIMDECK_SERVER_URL ?? process.argv[2] ?? "http://127.0.0.1:4310",
);
const durationMs = Number(process.env.SIMDECK_STREAM_CHECK_MS ?? process.argv[3] ?? 15000);
const pollMs = Number(process.env.SIMDECK_STREAM_CHECK_POLL_MS ?? 1000);
const maxFrameGapMs = Number(process.env.SIMDECK_STREAM_CHECK_MAX_GAP_MS ?? 250);
const failOnServerDrops =
  (process.env.SIMDECK_STREAM_CHECK_FAIL_SERVER_DROPS ?? "0").trim() === "1";

function endpoint(path) {
  return new URL(path, serverUrl).toString();
}

async function fetchJson(path) {
  const response = await fetch(endpoint(path), { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`${path} returned ${response.status}: ${await response.text()}`);
  }
  return response.json();
}

function streamKey(stream) {
  return `${stream.clientId ?? "unknown"}:${stream.kind ?? "unknown"}`;
}

function freshClientStreams(metrics, nowMs) {
  return (metrics.client_streams ?? []).filter((stream) => {
    const timestampMs = Number(stream.timestampMs ?? 0);
    if (timestampMs <= 0 || nowMs - timestampMs > 5000) {
      return false;
    }
    return stream.status === "streaming" || stream.status === "connected";
  });
}

function numeric(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

const startedAt = Date.now();
const initial = await fetchJson("/api/metrics");
const initialStreams = new Map(
  freshClientStreams(initial, startedAt).map((stream) => [streamKey(stream), stream]),
);
let latest = initial;
const failures = [];

while (Date.now() - startedAt < durationMs) {
  await new Promise((resolve) => setTimeout(resolve, pollMs));
  latest = await fetchJson("/api/metrics");
  const now = Date.now();
  const streams = freshClientStreams(latest, now);

  if (numeric(latest.active_streams) < 1) {
    failures.push("no active WebRTC streams");
  }
  for (const stream of streams) {
    if (numeric(stream.latestFrameGapMs) > maxFrameGapMs) {
      failures.push(
        `${streamKey(stream)} latestFrameGapMs=${stream.latestFrameGapMs} exceeded ${maxFrameGapMs}`,
      );
    }
  }
}

const endedAt = Date.now();
const finalStreams = freshClientStreams(latest, endedAt);
const finalByKey = new Map(finalStreams.map((stream) => [streamKey(stream), stream]));
const serverDropDelta =
  numeric(latest.frames_dropped_server) - numeric(initial.frames_dropped_server);
if (serverDropDelta > 0) {
  const message = `server dropped ${serverDropDelta} stale frames during check`;
  if (failOnServerDrops) {
    failures.push(message);
  }
}

let advancingPageStreams = 0;
for (const [key, finalStream] of finalByKey) {
  const initialStream = initialStreams.get(key);
  if (!initialStream) {
    continue;
  }
  const renderedDelta =
    numeric(finalStream.renderedFrames) - numeric(initialStream.renderedFrames);
  const decodedDelta =
    numeric(finalStream.decodedFrames) - numeric(initialStream.decodedFrames);
  const receivedDelta =
    numeric(finalStream.receivedPackets) - numeric(initialStream.receivedPackets);
  const droppedDelta =
    numeric(finalStream.droppedFrames) - numeric(initialStream.droppedFrames);
  const reconnectDelta = numeric(finalStream.reconnects) - numeric(initialStream.reconnects);

  if (finalStream.kind === "page" && (renderedDelta > 0 || decodedDelta > 0)) {
    advancingPageStreams += 1;
  }
  if (finalStream.kind === "webrtc" && receivedDelta <= 0) {
    failures.push(`${key} did not receive RTP packets`);
  }
  if (droppedDelta > 0) {
    failures.push(`${key} reported ${droppedDelta} dropped decoder frames`);
  }
  if (reconnectDelta > 0) {
    failures.push(`${key} reconnected ${reconnectDelta} times`);
  }
}

if (advancingPageStreams < 1) {
  failures.push("no fresh page stream advanced rendered/decoded frames");
}

const summary = {
  activeStreams: latest.active_streams,
  durationMs: endedAt - startedAt,
  framesDroppedServerDelta: serverDropDelta,
  framesEncodedDelta: numeric(latest.frames_encoded) - numeric(initial.frames_encoded),
  framesSentDelta: numeric(latest.frames_sent) - numeric(initial.frames_sent),
  streams: finalStreams.map((stream) => ({
    clientId: stream.clientId,
    codec: stream.codec,
    decodedFrames: stream.decodedFrames,
    droppedFrames: stream.droppedFrames,
    iceConnectionState: stream.iceConnectionState,
    kind: stream.kind,
    latestFrameGapMs: stream.latestFrameGapMs,
    peerConnectionState: stream.peerConnectionState,
    receivedPackets: stream.receivedPackets,
    reconnects: stream.reconnects,
    renderedFrames: stream.renderedFrames,
    status: stream.status,
  })),
};

if (failures.length > 0) {
  console.error(JSON.stringify({ ...summary, failures }, null, 2));
  process.exit(1);
}

console.log(JSON.stringify({ ...summary, ok: true }, null, 2));
