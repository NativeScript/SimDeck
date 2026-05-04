#!/usr/bin/env node

import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";

const serverUrl = new URL(process.env.SIMDECK_SERVER_URL ?? process.argv[2] ?? "http://127.0.0.1:4310");
const durationMs = Number(process.env.SIMDECK_E2E_WEBRTC_MS ?? process.argv[3] ?? 60_000);
const pollMs = Number(process.env.SIMDECK_E2E_WEBRTC_POLL_MS ?? 1000);
const chromePath =
  process.env.CHROME_PATH ??
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const debugPort = Number(process.env.SIMDECK_E2E_CHROME_PORT ?? 9339);
const maxFrameGapMs = Number(process.env.SIMDECK_E2E_MAX_FRAME_GAP_MS ?? 250);
const maxInteractionLatencyMs = Number(process.env.SIMDECK_E2E_MAX_INTERACTION_LATENCY_MS ?? 750);
const maxPeerDisconnectedMs = Number(process.env.SIMDECK_E2E_MAX_PEER_DISCONNECTED_MS ?? 1000);
const visualSampleIntervalMs = Number(process.env.SIMDECK_E2E_VISUAL_SAMPLE_INTERVAL_MS ?? 1000);
const maxVisualMeanDiff = Number(process.env.SIMDECK_E2E_MAX_VISUAL_MEAN_DIFF ?? 18);
const maxVisualBadPixelRatio = Number(process.env.SIMDECK_E2E_MAX_VISUAL_BAD_PIXEL_RATIO ?? 0.08);
const maxVisualTileDiff = Number(process.env.SIMDECK_E2E_MAX_VISUAL_TILE_DIFF ?? 42);

function endpoint(path) {
  return new URL(path, serverUrl).toString();
}

async function fetchJson(url, init) {
  const response = await fetch(url, init);
  if (!response.ok) {
    throw new Error(`${url} returned ${response.status}: ${await response.text()}`);
  }
  return response.json();
}

async function waitForDevTools() {
  const versionUrl = `http://127.0.0.1:${debugPort}/json/version`;
  const startedAt = Date.now();
  while (Date.now() - startedAt < 10_000) {
    try {
      return await fetchJson(versionUrl);
    } catch {
      await sleep(100);
    }
  }
  throw new Error("Timed out waiting for Chrome DevTools endpoint.");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

class CdpClient {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id && this.pending.has(message.id)) {
        const { resolve, reject } = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error) {
          reject(new Error(message.error.message));
        } else {
          resolve(message.result);
        }
      }
    });
  }

  close() {
    this.ws.close();
  }

  send(method, params = {}) {
    const id = this.nextId++;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      setTimeout(() => {
        if (this.pending.delete(id)) {
          reject(new Error(`CDP command timed out: ${method}`));
        }
      }, 10_000);
    });
  }
}

async function connectCdp(webSocketDebuggerUrl) {
  const ws = new WebSocket(webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", reject, { once: true });
  });
  return new CdpClient(ws);
}

async function evaluate(cdp, expression, awaitPromise = true) {
  const result = await cdp.send("Runtime.evaluate", {
    awaitPromise,
    expression,
    returnByValue: true,
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text ?? "Runtime evaluation failed.");
  }
  return result.result.value;
}

function streamKey(stream) {
  return `${stream.clientId ?? "unknown"}:${stream.kind ?? "unknown"}`;
}

function numeric(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function findClientStreams(metrics, clientId) {
  return (metrics.client_streams ?? []).filter((stream) => stream.clientId === clientId);
}

function latestByKind(streams, kind) {
  return streams
    .filter((stream) => stream.kind === kind)
    .sort((a, b) => numeric(b.timestampMs) - numeric(a.timestampMs))[0];
}

const profileDir = await mkdtemp(join(tmpdir(), "simdeck-webrtc-e2e-"));
const chromeArgs = [
  `--remote-debugging-port=${debugPort}`,
  `--user-data-dir=${profileDir}`,
  "--no-first-run",
  "--no-default-browser-check",
  "--autoplay-policy=no-user-gesture-required",
  "--use-fake-ui-for-media-stream",
  "--disable-background-timer-throttling",
  "--disable-renderer-backgrounding",
  "--window-size=1280,900",
  "about:blank",
];
if (process.env.SIMDECK_E2E_HEADFUL !== "1") {
  chromeArgs.splice(0, 0, "--headless=new");
}

const chrome = spawn(chromePath, chromeArgs, {
  stdio: ["ignore", "ignore", "pipe"],
});
let chromeStderr = "";
let cdp;
chrome.stderr.on("data", (chunk) => {
  chromeStderr += chunk.toString();
});

try {
  await waitForDevTools();
  const target = await createPageTarget("about:blank");
  cdp = await connectCdp(target.webSocketDebuggerUrl);
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Page.addScriptToEvaluateOnNewDocument", {
    source: `
      (() => {
        const NativeRTCPeerConnection = window.RTCPeerConnection;
        window.__simdeckPeerConnections = [];
        window.RTCPeerConnection = function(...args) {
          const pc = new NativeRTCPeerConnection(...args);
          window.__simdeckPeerConnections.push(pc);
          return pc;
        };
        window.RTCPeerConnection.prototype = NativeRTCPeerConnection.prototype;
        Object.setPrototypeOf(window.RTCPeerConnection, NativeRTCPeerConnection);
      })();
    `,
  });
  await cdp.send("Page.navigate", { url: serverUrl.toString() });

  const clientId = await waitForValue(cdp, `
    (() => {
      try { return window.sessionStorage.getItem("simdeck.streamClientId") || ""; }
      catch { return ""; }
    })()
  `, Boolean, 15_000);

  await waitForValue(cdp, `
    (() => {
      const videos = [...document.querySelectorAll("video.stream-video")];
      return videos.some((video) => video.readyState >= 2 && video.videoWidth > 0 && video.videoHeight > 0);
    })()
  `, Boolean, 20_000);

  const initialMetrics = await fetchJson(endpoint("/api/metrics"));
  const initialStreams = findClientStreams(initialMetrics, clientId);
  const initialPage = latestByKind(initialStreams, "page") ?? {};
  const initialWebRtc = latestByKind(initialStreams, "webrtc") ?? {};
  const directStatsStart = await collectDirectWebRtcStats(cdp);
  let maxObservedFrameGapMs = 0;
  let maxObservedDecodeQueue = 0;

  const startedAt = Date.now();
  let lastInteractionAt = 0;
  let lastVisualSampleAt = -visualSampleIntervalMs;
  let peerDisconnectedSince = 0;
  let maxPeerDisconnectedObservedMs = 0;
  const interactionLatencies = [];
  const presentedInteractionLatencies = [];
  const visualSamples = [];
  while (Date.now() - startedAt < durationMs) {
    await sleep(pollMs);
    const elapsed = Date.now() - startedAt;
    const metrics = await fetchJson(endpoint("/api/metrics"));
    const streams = findClientStreams(metrics, clientId);
    const webrtcStream = latestByKind(streams, "webrtc");
    const peerState =
      webrtcStream?.peerConnectionState ?? webrtcStream?.iceConnectionState ?? "";
    if (peerState === "disconnected" || peerState === "failed") {
      peerDisconnectedSince ||= Date.now();
      maxPeerDisconnectedObservedMs = Math.max(
        maxPeerDisconnectedObservedMs,
        Date.now() - peerDisconnectedSince,
      );
    } else {
      peerDisconnectedSince = 0;
    }
    for (const stream of streams) {
      maxObservedFrameGapMs = Math.max(maxObservedFrameGapMs, numeric(stream.latestFrameGapMs));
      maxObservedDecodeQueue = Math.max(maxObservedDecodeQueue, numeric(stream.decodeQueueSize));
    }
    const visualUdid = latestByKind(streams, "webrtc")?.udid ?? latestByKind(streams, "page")?.udid;
    if (visualUdid && elapsed - lastVisualSampleAt >= visualSampleIntervalMs) {
      lastVisualSampleAt = elapsed;
      const visualSample = await collectVisualArtifactSample(cdp, visualUdid).catch(() => null);
      if (visualSample) {
        visualSamples.push(visualSample);
      }
    }
    if (elapsed - lastInteractionAt >= 5000) {
      lastInteractionAt = elapsed;
      const beforeInteraction = await collectDirectWebRtcStats(cdp);
      await interactWithSimulatorViewport(cdp, elapsed);
      interactionLatencies.push(
        await waitForDecodedFrameAfterInteraction(cdp, beforeInteraction.framesDecoded, maxInteractionLatencyMs),
      );
      presentedInteractionLatencies.push(
        await waitForPresentedFrameAfterInteraction(
          cdp,
          beforeInteraction.totalVideoFrames,
          maxInteractionLatencyMs,
        ),
      );
    }
  }

  const finalMetrics = await fetchJson(endpoint("/api/metrics"));
  const finalStreams = findClientStreams(finalMetrics, clientId);
  const finalPage = latestByKind(finalStreams, "page") ?? {};
  const finalWebRtc = latestByKind(finalStreams, "webrtc") ?? {};
  const directStatsEnd = await collectDirectWebRtcStats(cdp);
  const failures = [];

  const renderedDelta = numeric(finalPage.renderedFrames) - numeric(initialPage.renderedFrames);
  const decodedDelta = numeric(finalWebRtc.decodedFrames) - numeric(initialWebRtc.decodedFrames);
  const receivedDelta = numeric(finalWebRtc.receivedPackets) - numeric(initialWebRtc.receivedPackets);
  const droppedDelta = numeric(finalWebRtc.droppedFrames) - numeric(initialWebRtc.droppedFrames);
  const reconnectDelta = numeric(finalWebRtc.reconnects) - numeric(initialWebRtc.reconnects);
  const directDroppedDelta = directStatsEnd.framesDropped - directStatsStart.framesDropped;
  const directDecodedDelta = directStatsEnd.framesDecoded - directStatsStart.framesDecoded;
  const directPacketsDelta = directStatsEnd.packetsReceived - directStatsStart.packetsReceived;
  const directPresentedDelta = directStatsEnd.totalVideoFrames - directStatsStart.totalVideoFrames;

  if (decodedDelta <= 0 || receivedDelta <= 0) {
    failures.push(`SimDeck metrics did not advance decoded/RTP frames: decoded=${decodedDelta} received=${receivedDelta}`);
  }
  if (directDecodedDelta <= 0 || directPacketsDelta <= 0 || directPresentedDelta <= 0) {
    failures.push(
      `browser stats did not advance decoded/presented/RTP frames: decoded=${directDecodedDelta} presented=${directPresentedDelta} received=${directPacketsDelta}`,
    );
  }
  if (droppedDelta > 0 || directDroppedDelta > 0) {
    failures.push(`decoder dropped frames: metrics=${droppedDelta} getStats=${directDroppedDelta}`);
  }
  if (reconnectDelta > 0) {
    failures.push(`peer reconnected ${reconnectDelta} times`);
  }
  if (maxPeerDisconnectedObservedMs > maxPeerDisconnectedMs) {
    failures.push(
      `peer disconnected for ${maxPeerDisconnectedObservedMs}ms, exceeded ${maxPeerDisconnectedMs}ms`,
    );
  }
  const slowInteractions = interactionLatencies.filter((latency) => latency > maxInteractionLatencyMs);
  if (slowInteractions.length > 0) {
    failures.push(
      `decode did not advance within ${maxInteractionLatencyMs}ms after ${slowInteractions.length} interactions`,
    );
  }
  const slowPresentedInteractions = presentedInteractionLatencies.filter(
    (latency) => latency > maxInteractionLatencyMs,
  );
  if (slowPresentedInteractions.length > 0) {
    failures.push(
      `presented video did not advance within ${maxInteractionLatencyMs}ms after ${slowPresentedInteractions.length} interactions`,
    );
  }
  const visualFailures = visualSamples.filter(
    (sample) =>
      sample.meanDiff > maxVisualMeanDiff ||
      sample.badPixelRatio > maxVisualBadPixelRatio ||
      sample.maxTileMeanDiff > maxVisualTileDiff,
  );
  if (visualSamples.length === 0) {
    failures.push("no visual artifact samples were collected");
  } else if (visualFailures.length > 0) {
    const worst = visualFailures
      .slice()
      .sort((a, b) => b.maxTileMeanDiff - a.maxTileMeanDiff)[0];
    failures.push(
      `visual artifact threshold exceeded in ${visualFailures.length}/${visualSamples.length} samples: ` +
        `mean=${worst.meanDiff.toFixed(2)} bad=${worst.badPixelRatio.toFixed(4)} tile=${worst.maxTileMeanDiff.toFixed(2)}`,
    );
  }

  const summary = {
    clientId,
    directStatsEnd,
    directStatsStart,
    durationMs,
    finalPage,
    finalWebRtc,
    initialPage,
    initialWebRtc,
    maxObservedDecodeQueue,
    maxObservedFrameGapMs,
    maxPeerDisconnectedMs,
    maxPeerDisconnectedObservedMs,
    maxInteractionLatencyMs,
    interactionLatencies,
    presentedInteractionLatencies,
    visualThresholds: {
      maxVisualBadPixelRatio,
      maxVisualMeanDiff,
      maxVisualTileDiff,
    },
    visualSamples,
    renderedDelta,
    decodedDelta,
    receivedDelta,
    droppedDelta,
    reconnectDelta,
    streams: finalStreams.map((stream) => ({
      key: streamKey(stream),
      status: stream.status,
      codec: stream.codec,
      decodedFrames: stream.decodedFrames,
      renderedFrames: stream.renderedFrames,
      receivedPackets: stream.receivedPackets,
      droppedFrames: stream.droppedFrames,
      latestFrameGapMs: stream.latestFrameGapMs,
      reconnects: stream.reconnects,
    })),
  };
  if (failures.length > 0) {
    console.error(JSON.stringify({ ...summary, failures }, null, 2));
    process.exitCode = 1;
  } else {
    console.log(JSON.stringify({ ...summary, ok: true }, null, 2));
  }
} finally {
  cdp?.close();
  await stopChrome(chrome);
  await rm(profileDir, { force: true, recursive: true });
}

async function stopChrome(chromeProcess) {
  if (chromeProcess.exitCode !== null || chromeProcess.signalCode !== null) {
    return;
  }
  chromeProcess.kill("SIGTERM");
  const exited = await Promise.race([
    new Promise((resolve) => chromeProcess.once("exit", () => resolve(true))),
    sleep(1500).then(() => false),
  ]);
  if (exited || chromeProcess.exitCode !== null || chromeProcess.signalCode !== null) {
    return;
  }
  chromeProcess.kill("SIGKILL");
  await Promise.race([
    new Promise((resolve) => chromeProcess.once("exit", resolve)),
    sleep(1500),
  ]);
}

async function waitForValue(cdp, expression, predicate, timeoutMs) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const value = await evaluate(cdp, expression);
    if (predicate(value)) {
      return value;
    }
    await sleep(250);
  }
  throw new Error(`Timed out waiting for expression: ${expression}\n${chromeStderr}`);
}

async function createPageTarget(url) {
  const response = await fetch(
    `http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent(url)}`,
    { method: "PUT" },
  );
  if (!response.ok) {
    throw new Error(`Failed to create Chrome target: ${response.status} ${await response.text()}`);
  }
  return response.json();
}

async function collectDirectWebRtcStats(cdp) {
  return evaluate(cdp, `
    (async () => {
      const totals = {
        framesDecoded: 0,
        framesDropped: 0,
        jitter: 0,
        packetsLost: 0,
        packetsReceived: 0,
        totalVideoFrames: 0,
      };
      for (const pc of window.__simdeckPeerConnections || []) {
        const reports = await pc.getStats();
        for (const report of reports.values()) {
          if (report.type === "inbound-rtp" && (report.kind === "video" || report.mediaType === "video")) {
            totals.framesDecoded += report.framesDecoded || 0;
            totals.framesDropped += report.framesDropped || 0;
            totals.jitter = Math.max(totals.jitter, report.jitter || 0);
            totals.packetsLost += report.packetsLost || 0;
            totals.packetsReceived += report.packetsReceived || 0;
          }
        }
      }
      for (const video of document.querySelectorAll("video.stream-video")) {
        const quality = video.getVideoPlaybackQuality?.();
        totals.totalVideoFrames += quality?.totalVideoFrames || 0;
      }
      return totals;
    })()
  `);
}

async function waitForDecodedFrameAfterInteraction(cdp, baselineFramesDecoded, timeoutMs) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const stats = await collectDirectWebRtcStats(cdp);
    if (stats.framesDecoded > baselineFramesDecoded) {
      return Date.now() - startedAt;
    }
    await sleep(25);
  }
  return Date.now() - startedAt;
}

async function waitForPresentedFrameAfterInteraction(cdp, baselinePresentedFrames, timeoutMs) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const stats = await collectDirectWebRtcStats(cdp);
    if (stats.totalVideoFrames > baselinePresentedFrames) {
      return Date.now() - startedAt;
    }
    await sleep(25);
  }
  return Date.now() - startedAt;
}

async function collectVisualArtifactSample(cdp, udid) {
  return evaluate(cdp, `
    (async () => {
      const video = document.querySelector("video.stream-video");
      if (!video || video.readyState < 2 || !video.videoWidth || !video.videoHeight) {
        throw new Error("live video is not ready for visual comparison");
      }

      const response = await fetch("/api/simulators/${udid}/screenshot.png?artifactCheck=" + Date.now(), {
        cache: "no-store",
      });
      if (!response.ok) {
        throw new Error("native screenshot failed with " + response.status);
      }
      const source = await createImageBitmap(await response.blob());
      const width = Math.min(240, video.videoWidth, source.width);
      const height = Math.max(1, Math.round(width * (video.videoHeight / video.videoWidth)));
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const videoCanvas = document.createElement("canvas");
      videoCanvas.width = width;
      videoCanvas.height = height;
      const sourceContext = canvas.getContext("2d", { willReadFrequently: true });
      const videoContext = videoCanvas.getContext("2d", { willReadFrequently: true });
      sourceContext.imageSmoothingEnabled = true;
      videoContext.imageSmoothingEnabled = true;
      sourceContext.drawImage(source, 0, 0, width, height);
      videoContext.drawImage(video, 0, 0, width, height);

      const sourceData = sourceContext.getImageData(0, 0, width, height).data;
      const videoData = videoContext.getImageData(0, 0, width, height).data;
      const tileSize = 16;
      const tileColumns = Math.ceil(width / tileSize);
      const tileRows = Math.ceil(height / tileSize);
      const tileSums = new Float64Array(tileColumns * tileRows);
      const tileCounts = new Uint32Array(tileColumns * tileRows);
      let badPixels = 0;
      let sum = 0;
      let maxPixelDiff = 0;
      for (let y = 0; y < height; y += 1) {
        for (let x = 0; x < width; x += 1) {
          const offset = (y * width + x) * 4;
          const diff = (
            Math.abs(sourceData[offset] - videoData[offset]) +
            Math.abs(sourceData[offset + 1] - videoData[offset + 1]) +
            Math.abs(sourceData[offset + 2] - videoData[offset + 2])
          ) / 3;
          sum += diff;
          maxPixelDiff = Math.max(maxPixelDiff, diff);
          if (diff > 48) {
            badPixels += 1;
          }
          const tileIndex = Math.floor(y / tileSize) * tileColumns + Math.floor(x / tileSize);
          tileSums[tileIndex] += diff;
          tileCounts[tileIndex] += 1;
        }
      }
      let maxTileMeanDiff = 0;
      for (let index = 0; index < tileSums.length; index += 1) {
        if (tileCounts[index] > 0) {
          maxTileMeanDiff = Math.max(maxTileMeanDiff, tileSums[index] / tileCounts[index]);
        }
      }
      source.close();
      return {
        badPixelRatio: badPixels / (width * height),
        height,
        maxPixelDiff,
        maxTileMeanDiff,
        meanDiff: sum / (width * height),
        udid: "${udid}",
        videoHeight: video.videoHeight,
        videoWidth: video.videoWidth,
        width,
      };
    })()
  `);
}

async function interactWithSimulatorViewport(cdp, elapsedMs) {
  const point = await evaluate(cdp, `
    (() => {
      const target = document.querySelector("canvas.stream-canvas") || document.querySelector(".device-screen");
      if (!target) return null;
      const rect = target.getBoundingClientRect();
      const x = rect.left + rect.width * (0.35 + ((Math.floor(${elapsedMs} / 5000) % 3) * 0.15));
      const y = rect.top + rect.height * 0.55;
      return { x, y };
    })()
  `);
  if (!point) {
    return;
  }
  await cdp.send("Input.dispatchMouseEvent", {
    button: "left",
    buttons: 1,
    clickCount: 1,
    type: "mousePressed",
    x: point.x,
    y: point.y,
  });
  await cdp.send("Input.dispatchMouseEvent", {
    button: "left",
    buttons: 0,
    clickCount: 1,
    type: "mouseReleased",
    x: point.x,
    y: point.y,
  });
}
