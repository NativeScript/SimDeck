#!/usr/bin/env node

import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import net from "node:net";

const serverUrl = new URL(
  process.env.SIMDECK_SERVER_URL ?? process.argv[2] ?? "http://127.0.0.1:4310",
);
const apiRootPath =
  process.env.SIMDECK_E2E_API_ROOT ?? apiRootPathForViewerUrl(serverUrl);
const durationMs = Number(
  process.env.SIMDECK_E2E_WEBRTC_MS ?? process.argv[3] ?? 60_000,
);
const pollMs = Number(process.env.SIMDECK_E2E_WEBRTC_POLL_MS ?? 1000);
const chromePath =
  process.env.CHROME_PATH ??
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const debugPort = Number(process.env.SIMDECK_E2E_CHROME_PORT ?? 9339);
const chromeStartupTimeoutMs = Number(
  process.env.SIMDECK_E2E_CHROME_STARTUP_MS ?? 60_000,
);
const maxFrameGapMs = Number(process.env.SIMDECK_E2E_MAX_FRAME_GAP_MS ?? 250);
const maxInteractionLatencyMs = Number(
  process.env.SIMDECK_E2E_MAX_INTERACTION_LATENCY_MS ?? 750,
);
const interactionsEnabled = process.env.SIMDECK_E2E_INTERACTIONS !== "0";
const maxPeerDisconnectedMs = Number(
  process.env.SIMDECK_E2E_MAX_PEER_DISCONNECTED_MS ?? 1000,
);
const streamReadyTimeoutMs = Number(
  process.env.SIMDECK_E2E_STREAM_READY_MS ?? 90_000,
);
const warmupMs = Number(process.env.SIMDECK_E2E_WARMUP_MS ?? 0);
const maxDecoderDrops = Number(process.env.SIMDECK_E2E_MAX_DECODER_DROPS ?? 0);
const minVideoWidth = Number(process.env.SIMDECK_E2E_MIN_VIDEO_WIDTH ?? 0);
const minVideoHeight = Number(process.env.SIMDECK_E2E_MIN_VIDEO_HEIGHT ?? 0);
const minDecodedFps = Number(process.env.SIMDECK_E2E_MIN_DECODED_FPS ?? 0);
const minPresentedFps = Number(process.env.SIMDECK_E2E_MIN_PRESENTED_FPS ?? 0);
const minReceivedFps = Number(process.env.SIMDECK_E2E_MIN_RECEIVED_FPS ?? 0);
const visualSampleIntervalMs = Number(
  process.env.SIMDECK_E2E_VISUAL_SAMPLE_INTERVAL_MS ?? 5000,
);
const visualSamplesEnabled = visualSampleIntervalMs > 0;
const maxVisualMeanDiff = Number(
  process.env.SIMDECK_E2E_MAX_VISUAL_MEAN_DIFF ?? 18,
);
const maxVisualBadPixelRatio = Number(
  process.env.SIMDECK_E2E_MAX_VISUAL_BAD_PIXEL_RATIO ?? 0.08,
);
const maxVisualTileDiff = Number(
  process.env.SIMDECK_E2E_MAX_VISUAL_TILE_DIFF ?? 42,
);
const maxVisualFailureRatio = Number(
  process.env.SIMDECK_E2E_MAX_VISUAL_FAILURE_RATIO ?? 0.2,
);
const maxConsecutiveVisualFailures = Number(
  process.env.SIMDECK_E2E_MAX_CONSECUTIVE_VISUAL_FAILURES ?? 1,
);
const screenshotApiRoot = process.env.SIMDECK_E2E_SCREENSHOT_API_ROOT
  ? new URL(process.env.SIMDECK_E2E_SCREENSHOT_API_ROOT)
  : null;
const screenshotApiToken = process.env.SIMDECK_E2E_SCREENSHOT_API_TOKEN ?? "";
const outputJsonPath = process.env.SIMDECK_E2E_OUTPUT_JSON ?? "";
const requireVisualSamples = process.env.SIMDECK_E2E_REQUIRE_VISUAL !== "0";

function endpoint(path) {
  return new URL(`${apiRootPath}${path}`, serverUrl).toString();
}

function apiRootPathForViewerUrl(url) {
  const match = url.pathname.match(/^\/simulator\/([^/]+)/);
  if (!match) {
    return "";
  }
  return `/api/provider-sessions/${match[1]}/simdeck`;
}

async function fetchJson(url, init) {
  const response = await fetch(url, init);
  if (!response.ok) {
    throw new Error(
      `${url} returned ${response.status}: ${await response.text()}`,
    );
  }
  return response.json();
}

async function fetchReferenceScreenshotDataUrl(udid) {
  if (!screenshotApiRoot) {
    return null;
  }
  const url = new URL(
    `/api/simulators/${encodeURIComponent(udid)}/screenshot.png`,
    screenshotApiRoot,
  );
  url.searchParams.set("artifactCheck", Date.now().toString());
  const headers = screenshotApiToken
    ? { "x-simdeck-token": screenshotApiToken }
    : undefined;
  const response = await fetch(url, {
    cache: "no-store",
    headers,
  });
  if (!response.ok) {
    throw new Error(
      `reference screenshot failed with ${response.status}: ${await response.text()}`,
    );
  }
  const contentType = response.headers.get("content-type") ?? "image/png";
  const base64 = Buffer.from(await response.arrayBuffer()).toString("base64");
  return `data:${contentType};base64,${base64}`;
}

async function waitForDevTools(chromeProcess, getChromeOutput, getSpawnError) {
  const versionUrl = `http://127.0.0.1:${debugPort}/json/version`;
  const startedAt = Date.now();
  while (Date.now() - startedAt < chromeStartupTimeoutMs) {
    const spawnError = getSpawnError();
    if (spawnError) {
      throw new Error(
        [`Chrome failed to start: ${spawnError.message}`, getChromeOutput()]
          .filter(Boolean)
          .join("\n"),
      );
    }
    if (chromeProcess.exitCode !== null || chromeProcess.signalCode !== null) {
      throw new Error(
        [
          `Chrome exited before DevTools became available (exit=${chromeProcess.exitCode}, signal=${chromeProcess.signalCode}).`,
          getChromeOutput(),
        ]
          .filter(Boolean)
          .join("\n"),
      );
    }
    try {
      return await fetchJson(versionUrl);
    } catch {
      await sleep(100);
    }
  }
  throw new Error(
    [
      `Timed out waiting ${chromeStartupTimeoutMs}ms for Chrome DevTools endpoint at ${versionUrl}.`,
      getChromeOutput(),
    ]
      .filter(Boolean)
      .join("\n"),
  );
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

class NodeWebSocket {
  constructor(url) {
    this.url = new URL(url);
    this.listeners = new Map();
    this.buffer = Buffer.alloc(0);
    this.handshakeComplete = false;
    this.opened = this.connect();
  }

  addEventListener(type, listener, options = {}) {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push({ listener, once: Boolean(options.once) });
    this.listeners.set(type, listeners);
  }

  removeEventListener(type, listener) {
    this.listeners.set(
      type,
      (this.listeners.get(type) ?? []).filter(
        (entry) => entry.listener !== listener,
      ),
    );
  }

  emit(type, event = {}) {
    const listeners = this.listeners.get(type) ?? [];
    this.listeners.set(
      type,
      listeners.filter(({ listener, once }) => {
        listener(event);
        return !once;
      }),
    );
  }

  async connect() {
    if (this.url.protocol !== "ws:") {
      throw new Error(
        `Unsupported CDP WebSocket protocol: ${this.url.protocol}`,
      );
    }
    const port = Number(this.url.port || 80);
    const key = randomBytes(16).toString("base64");
    this.socket = net.createConnection({
      host: this.url.hostname,
      port,
    });
    this.socket.on("data", (chunk) => this.handleData(chunk));
    this.socket.on("error", (error) => this.emit("error", error));
    this.socket.on("close", () => this.emit("close", {}));
    await new Promise((resolve, reject) => {
      this.socket.once("connect", resolve);
      this.socket.once("error", reject);
    });
    const path = `${this.url.pathname}${this.url.search}`;
    this.socket.write(
      [
        `GET ${path} HTTP/1.1`,
        `Host: ${this.url.host}`,
        "Upgrade: websocket",
        "Connection: Upgrade",
        `Sec-WebSocket-Key: ${key}`,
        "Sec-WebSocket-Version: 13",
        "",
        "",
      ].join("\r\n"),
    );
    await new Promise((resolve, reject) => {
      const onOpen = () => {
        cleanup();
        resolve();
      };
      const onError = (error) => {
        cleanup();
        reject(error);
      };
      const cleanup = () => {
        this.removeEventListener("open", onOpen);
        this.removeEventListener("error", onError);
      };
      this.addEventListener("open", onOpen, { once: true });
      this.addEventListener("error", onError, { once: true });
    });
  }

  handleData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    if (!this.handshakeComplete) {
      const headerEnd = this.buffer.indexOf("\r\n\r\n");
      if (headerEnd < 0) {
        return;
      }
      const header = this.buffer.subarray(0, headerEnd).toString("utf8");
      this.buffer = this.buffer.subarray(headerEnd + 4);
      if (!header.startsWith("HTTP/1.1 101")) {
        this.emit(
          "error",
          new Error(`CDP WebSocket upgrade failed: ${header}`),
        );
        return;
      }
      this.handshakeComplete = true;
      this.emit("open", {});
    }
    this.readFrames();
  }

  readFrames() {
    while (this.buffer.length >= 2) {
      const first = this.buffer[0];
      const second = this.buffer[1];
      const opcode = first & 0x0f;
      const masked = Boolean(second & 0x80);
      let length = second & 0x7f;
      let offset = 2;
      if (length === 126) {
        if (this.buffer.length < offset + 2) return;
        length = this.buffer.readUInt16BE(offset);
        offset += 2;
      } else if (length === 127) {
        if (this.buffer.length < offset + 8) return;
        const high = this.buffer.readUInt32BE(offset);
        const low = this.buffer.readUInt32BE(offset + 4);
        length = high * 2 ** 32 + low;
        offset += 8;
      }
      const maskLength = masked ? 4 : 0;
      if (this.buffer.length < offset + maskLength + length) {
        return;
      }
      let payload = this.buffer.subarray(
        offset + maskLength,
        offset + maskLength + length,
      );
      if (masked) {
        const mask = this.buffer.subarray(offset, offset + maskLength);
        const unmasked = Buffer.alloc(payload.length);
        for (let index = 0; index < payload.length; index += 1) {
          unmasked[index] = payload[index] ^ mask[index % 4];
        }
        payload = unmasked;
      }
      this.buffer = this.buffer.subarray(offset + maskLength + length);
      if (opcode === 0x1) {
        this.emit("message", { data: payload.toString("utf8") });
      } else if (opcode === 0x8) {
        this.close();
        return;
      } else if (opcode === 0x9) {
        this.writeFrame(0x0a, payload);
      }
    }
  }

  send(data) {
    this.writeFrame(0x1, Buffer.from(data));
  }

  writeFrame(opcode, payload) {
    const mask = randomBytes(4);
    let headerLength = 2;
    if (payload.length >= 126 && payload.length <= 0xffff) {
      headerLength += 2;
    } else if (payload.length > 0xffff) {
      headerLength += 8;
    }
    const frame = Buffer.alloc(headerLength + mask.length + payload.length);
    frame[0] = 0x80 | opcode;
    if (payload.length < 126) {
      frame[1] = 0x80 | payload.length;
    } else if (payload.length <= 0xffff) {
      frame[1] = 0x80 | 126;
      frame.writeUInt16BE(payload.length, 2);
    } else {
      frame[1] = 0x80 | 127;
      frame.writeUInt32BE(Math.floor(payload.length / 2 ** 32), 2);
      frame.writeUInt32BE(payload.length >>> 0, 6);
    }
    mask.copy(frame, headerLength);
    for (let index = 0; index < payload.length; index += 1) {
      frame[headerLength + mask.length + index] =
        payload[index] ^ mask[index % 4];
    }
    this.socket.write(frame);
  }

  close() {
    this.socket?.end();
  }
}

async function createCdpSocket(url) {
  if (typeof WebSocket === "function") {
    const ws = new WebSocket(url);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", reject, { once: true });
    });
    return ws;
  }
  const ws = new NodeWebSocket(url);
  await ws.opened;
  return ws;
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
  const ws = await createCdpSocket(webSocketDebuggerUrl);
  return new CdpClient(ws);
}

async function evaluate(cdp, expression, awaitPromise = true) {
  const result = await cdp.send("Runtime.evaluate", {
    awaitPromise,
    expression,
    returnByValue: true,
  });
  if (result.exceptionDetails) {
    throw new Error(
      result.exceptionDetails.text ?? "Runtime evaluation failed.",
    );
  }
  return result.result.value;
}

function streamKey(stream) {
  return `${stream.clientId ?? "unknown"}:${stream.kind ?? "unknown"}`;
}

function numeric(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function isVisualFailure(sample) {
  return (
    sample.meanDiff > maxVisualMeanDiff ||
    sample.badPixelRatio > maxVisualBadPixelRatio ||
    sample.maxTileMeanDiff > maxVisualTileDiff
  );
}

function maxConsecutiveMatches(values, predicate) {
  let current = 0;
  let max = 0;
  for (const value of values) {
    if (predicate(value)) {
      current += 1;
      max = Math.max(max, current);
    } else {
      current = 0;
    }
  }
  return max;
}

function findClientStreams(metrics, clientId) {
  return (metrics.client_streams ?? []).filter(
    (stream) => stream.clientId === clientId,
  );
}

function latestByKind(streams, kind) {
  return streams
    .filter((stream) => stream.kind === kind)
    .sort((a, b) => numeric(b.timestampMs) - numeric(a.timestampMs))[0];
}

const profileDir = await mkdtemp(join(tmpdir(), "simdeck-webrtc-e2e-"));
const chromeArgs = [
  `--remote-debugging-port=${debugPort}`,
  "--remote-debugging-address=127.0.0.1",
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
  stdio: ["ignore", "pipe", "pipe"],
});
let chromeOutput = "";
let chromeSpawnError = null;
let cdp;
chrome.stdout.on("data", (chunk) => {
  chromeOutput += chunk.toString();
});
chrome.stderr.on("data", (chunk) => {
  chromeOutput += chunk.toString();
});
chrome.on("error", (error) => {
  chromeSpawnError = error;
});

try {
  await waitForDevTools(
    chrome,
    () => chromeOutput.trim(),
    () => chromeSpawnError,
  );
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

  const clientId = await waitForValue(
    cdp,
    `
    (() => {
      try { return window.sessionStorage.getItem("simdeck.streamClientId") || ""; }
      catch { return ""; }
    })()
  `,
    Boolean,
    15_000,
  );

  try {
    await waitForValue(
      cdp,
      `
    (() => {
      const videos = [...document.querySelectorAll("video.stream-video")];
      return videos.some((video) => video.readyState >= 2 && video.videoWidth > 0 && video.videoHeight > 0);
    })()
  `,
      Boolean,
      streamReadyTimeoutMs,
    );
  } catch (error) {
    const diagnostics = await collectReadinessDiagnostics(cdp, clientId).catch(
      (diagnosticError) => ({
        diagnosticError: String(diagnosticError?.message ?? diagnosticError),
      }),
    );
    throw new Error(
      `${error?.message ?? error}\nReadiness diagnostics: ${JSON.stringify(diagnostics, null, 2)}`,
    );
  }

  if (warmupMs > 0) {
    await sleep(warmupMs);
  }

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
  const visualSampleErrors = [];
  while (Date.now() - startedAt < durationMs) {
    await sleep(pollMs);
    const elapsed = Date.now() - startedAt;
    const metrics = await fetchJson(endpoint("/api/metrics"));
    const streams = findClientStreams(metrics, clientId);
    const webrtcStream = latestByKind(streams, "webrtc");
    const peerState =
      webrtcStream?.peerConnectionState ??
      webrtcStream?.iceConnectionState ??
      "";
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
      maxObservedFrameGapMs = Math.max(
        maxObservedFrameGapMs,
        numeric(stream.latestFrameGapMs),
      );
      maxObservedDecodeQueue = Math.max(
        maxObservedDecodeQueue,
        numeric(stream.decodeQueueSize),
      );
    }
    const visualUdid =
      latestByKind(streams, "webrtc")?.udid ??
      latestByKind(streams, "page")?.udid;
    if (
      visualSamplesEnabled &&
      visualUdid &&
      elapsed - lastVisualSampleAt >= visualSampleIntervalMs
    ) {
      lastVisualSampleAt = elapsed;
      const visualSample = await collectVisualArtifactSample(
        cdp,
        visualUdid,
      ).catch((error) => {
        visualSampleErrors.push(String(error?.message ?? error));
        return null;
      });
      if (visualSample) {
        visualSamples.push(visualSample);
      }
    }
    if (interactionsEnabled && elapsed - lastInteractionAt >= 5000) {
      lastInteractionAt = elapsed;
      const beforeInteraction = await collectDirectWebRtcStats(cdp);
      await interactWithSimulatorViewport(cdp, elapsed);
      interactionLatencies.push(
        await waitForDecodedFrameAfterInteraction(
          cdp,
          beforeInteraction.framesDecoded,
          maxInteractionLatencyMs,
        ),
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

  const renderedDelta =
    numeric(finalPage.renderedFrames) - numeric(initialPage.renderedFrames);
  const decodedDelta =
    numeric(finalWebRtc.decodedFrames) - numeric(initialWebRtc.decodedFrames);
  const receivedDelta =
    numeric(finalWebRtc.receivedPackets) -
    numeric(initialWebRtc.receivedPackets);
  const droppedDelta =
    numeric(finalWebRtc.droppedFrames) - numeric(initialWebRtc.droppedFrames);
  const reconnectDelta =
    numeric(finalWebRtc.reconnects) - numeric(initialWebRtc.reconnects);
  const directDroppedDelta =
    directStatsEnd.framesDropped - directStatsStart.framesDropped;
  const directDecodedDelta =
    directStatsEnd.framesDecoded - directStatsStart.framesDecoded;
  const directPacketsDelta =
    directStatsEnd.packetsReceived - directStatsStart.packetsReceived;
  const directPresentedDelta =
    directStatsEnd.totalVideoFrames - directStatsStart.totalVideoFrames;
  const observedDurationSeconds = Math.max(
    0.001,
    (directStatsEnd.timestampMs - directStatsStart.timestampMs) / 1000,
  );
  const decodedFps = directDecodedDelta / observedDurationSeconds;
  const presentedFps = directPresentedDelta / observedDurationSeconds;
  const receivedFps = directPacketsDelta / observedDurationSeconds;

  if (decodedDelta <= 0 || receivedDelta <= 0) {
    failures.push(
      `SimDeck metrics did not advance decoded/RTP frames: decoded=${decodedDelta} received=${receivedDelta}`,
    );
  }
  if (
    directDecodedDelta <= 0 ||
    directPacketsDelta <= 0 ||
    directPresentedDelta <= 0
  ) {
    failures.push(
      `browser stats did not advance decoded/presented/RTP frames: decoded=${directDecodedDelta} presented=${directPresentedDelta} received=${directPacketsDelta}`,
    );
  }
  if (droppedDelta > maxDecoderDrops || directDroppedDelta > maxDecoderDrops) {
    failures.push(
      `decoder dropped frames: metrics=${droppedDelta} getStats=${directDroppedDelta} max=${maxDecoderDrops}`,
    );
  }
  if (reconnectDelta > 0) {
    failures.push(`peer reconnected ${reconnectDelta} times`);
  }
  const finalVideoWidth = Math.max(
    numeric(finalPage.width),
    numeric(finalWebRtc.width),
    numeric(directStatsEnd.videoWidth),
  );
  const finalVideoHeight = Math.max(
    numeric(finalPage.height),
    numeric(finalWebRtc.height),
    numeric(directStatsEnd.videoHeight),
  );
  if (minVideoWidth > 0 && finalVideoWidth < minVideoWidth) {
    failures.push(
      `video width ${finalVideoWidth} did not meet minimum ${minVideoWidth}`,
    );
  }
  if (minVideoHeight > 0 && finalVideoHeight < minVideoHeight) {
    failures.push(
      `video height ${finalVideoHeight} did not meet minimum ${minVideoHeight}`,
    );
  }
  if (minDecodedFps > 0 && decodedFps < minDecodedFps) {
    failures.push(
      `decoded fps ${decodedFps.toFixed(2)} did not meet minimum ${minDecodedFps}`,
    );
  }
  if (minPresentedFps > 0 && presentedFps < minPresentedFps) {
    failures.push(
      `presented fps ${presentedFps.toFixed(2)} did not meet minimum ${minPresentedFps}`,
    );
  }
  if (minReceivedFps > 0 && receivedFps < minReceivedFps) {
    failures.push(
      `received packet fps ${receivedFps.toFixed(2)} did not meet minimum ${minReceivedFps}`,
    );
  }
  if (maxPeerDisconnectedObservedMs > maxPeerDisconnectedMs) {
    failures.push(
      `peer disconnected for ${maxPeerDisconnectedObservedMs}ms, exceeded ${maxPeerDisconnectedMs}ms`,
    );
  }
  const slowInteractions = interactionLatencies.filter(
    (latency) => latency > maxInteractionLatencyMs,
  );
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
  const visualFailures = visualSamples.filter(isVisualFailure);
  const visualFailureRatio =
    visualSamples.length > 0 ? visualFailures.length / visualSamples.length : 0;
  const consecutiveVisualFailures = maxConsecutiveMatches(
    visualSamples,
    isVisualFailure,
  );
  if (requireVisualSamples && visualSamples.length === 0) {
    failures.push("no visual artifact samples were collected");
  } else if (
    visualFailureRatio > maxVisualFailureRatio ||
    consecutiveVisualFailures > maxConsecutiveVisualFailures
  ) {
    const worst = visualFailures
      .slice()
      .sort((a, b) => b.maxTileMeanDiff - a.maxTileMeanDiff)[0];
    failures.push(
      `visual artifact threshold exceeded in ${visualFailures.length}/${visualSamples.length} samples: ` +
        `mean=${worst.meanDiff.toFixed(2)} bad=${worst.badPixelRatio.toFixed(4)} tile=${worst.maxTileMeanDiff.toFixed(2)}`,
    );
  }

  const summary = {
    apiRootPath,
    clientId,
    directStatsEnd,
    directStatsStart,
    durationMs,
    finalPage,
    finalWebRtc,
    finalVideoHeight,
    finalVideoWidth,
    initialPage,
    initialWebRtc,
    observedDurationSeconds,
    decodedFps,
    presentedFps,
    receivedFps,
    maxObservedDecodeQueue,
    maxObservedFrameGapMs,
    maxPeerDisconnectedMs,
    maxPeerDisconnectedObservedMs,
    maxInteractionLatencyMs,
    maxDecoderDrops,
    warmupMs,
    interactionsEnabled,
    visualSamplesEnabled,
    interactionLatencies,
    presentedInteractionLatencies,
    visualThresholds: {
      maxVisualBadPixelRatio,
      maxVisualMeanDiff,
      maxVisualTileDiff,
      maxVisualFailureRatio,
      maxConsecutiveVisualFailures,
    },
    visualFailureRatio,
    consecutiveVisualFailures,
    screenshotApiRoot: screenshotApiRoot?.origin ?? null,
    visualSamples,
    visualSampleErrors: visualSampleErrors.slice(0, 10),
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
    const result = { ...summary, failures };
    await writeSummary(result);
    console.error(JSON.stringify(result, null, 2));
    process.exitCode = 1;
  } else {
    const result = { ...summary, ok: true };
    await writeSummary(result);
    console.log(JSON.stringify(result, null, 2));
  }
} finally {
  cdp?.close();
  await stopChrome(chrome);
  await rm(profileDir, { force: true, recursive: true });
}

async function writeSummary(summary) {
  if (!outputJsonPath) {
    return;
  }
  await writeFile(outputJsonPath, JSON.stringify(summary, null, 2));
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
  if (
    exited ||
    chromeProcess.exitCode !== null ||
    chromeProcess.signalCode !== null
  ) {
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
  throw new Error(
    `Timed out waiting for expression: ${expression}\n${chromeOutput}`,
  );
}

async function collectReadinessDiagnostics(cdp, clientId) {
  const page = await evaluate(
    cdp,
    `
    (() => {
      const videos = [...document.querySelectorAll("video.stream-video")].map((video) => ({
        height: video.videoHeight || 0,
        networkState: video.networkState,
        paused: video.paused,
        readyState: video.readyState,
        width: video.videoWidth || 0,
      }));
      const peerConnections = (window.__simdeckPeerConnections || []).map((pc) => ({
        connectionState: pc.connectionState,
        iceConnectionState: pc.iceConnectionState,
        iceGatheringState: pc.iceGatheringState,
        signalingState: pc.signalingState,
      }));
      return {
        bodyText: (document.body?.innerText || "").slice(0, 1000),
        canvases: document.querySelectorAll("canvas.stream-canvas").length,
        location: window.location.href,
        peerConnections,
        sessionClientId: window.sessionStorage.getItem("simdeck.streamClientId") || "",
        statusText: document.querySelector("[data-testid='stream-status']")?.textContent || "",
        title: document.title,
        videos,
      };
    })()
  `,
  );
  const metrics = await fetchJson(endpoint("/api/metrics")).catch((error) => ({
    error: String(error?.message ?? error),
  }));
  return {
    clientId,
    page,
    streams: Array.isArray(metrics?.client_streams)
      ? findClientStreams(metrics, clientId).map((stream) => ({
          codec: stream.codec,
          decodedFrames: stream.decodedFrames,
          detail: stream.detail,
          droppedFrames: stream.droppedFrames,
          height: stream.height,
          kind: stream.kind,
          latestFrameGapMs: stream.latestFrameGapMs,
          receivedPackets: stream.receivedPackets,
          reconnects: stream.reconnects,
          renderedFrames: stream.renderedFrames,
          status: stream.status,
          udid: stream.udid,
          width: stream.width,
        }))
      : metrics,
  };
}

async function createPageTarget(url) {
  const response = await fetch(
    `http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent(url)}`,
    { method: "PUT" },
  );
  if (!response.ok) {
    throw new Error(
      `Failed to create Chrome target: ${response.status} ${await response.text()}`,
    );
  }
  return response.json();
}

async function collectDirectWebRtcStats(cdp) {
  return evaluate(
    cdp,
    `
    (async () => {
      const totals = {
        framesDecoded: 0,
      framesDropped: 0,
      jitter: 0,
      packetsLost: 0,
      packetsReceived: 0,
      timestampMs: Date.now(),
      totalVideoFrames: 0,
      videoHeight: 0,
      videoWidth: 0,
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
        totals.videoWidth = Math.max(totals.videoWidth, video.videoWidth || 0);
        totals.videoHeight = Math.max(totals.videoHeight, video.videoHeight || 0);
      }
      return totals;
    })()
  `,
  );
}

async function waitForDecodedFrameAfterInteraction(
  cdp,
  baselineFramesDecoded,
  timeoutMs,
) {
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

async function waitForPresentedFrameAfterInteraction(
  cdp,
  baselinePresentedFrames,
  timeoutMs,
) {
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
  const referenceDataUrl = await fetchReferenceScreenshotDataUrl(udid);
  const screenshotPath = `${apiRootPath}/api/simulators/${encodeURIComponent(
    udid,
  )}/screenshot.png`;
  return evaluate(
    cdp,
    `
    (async () => {
      const video = document.querySelector("video.stream-video");
      if (!video || video.readyState < 2 || !video.videoWidth || !video.videoHeight) {
        throw new Error("live video is not ready for visual comparison");
      }

      const source = await loadReferenceScreenshot();
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

      async function loadReferenceScreenshot() {
        const dataUrl = ${JSON.stringify(referenceDataUrl)};
        if (dataUrl) {
          return createImageBitmap(await (await fetch(dataUrl)).blob());
        }
        const response = await fetch(new URL(${JSON.stringify(
          screenshotPath,
        )} + "?artifactCheck=" + Date.now(), window.location.href).toString(), {
          cache: "no-store",
        });
        if (!response.ok) {
          throw new Error("native screenshot failed with " + response.status);
        }
        return createImageBitmap(await response.blob());
      }
    })()
  `,
  );
}

async function interactWithSimulatorViewport(cdp, elapsedMs) {
  const point = await evaluate(
    cdp,
    `
    (() => {
      const target = document.querySelector("canvas.stream-canvas") || document.querySelector(".device-screen");
      if (!target) return null;
      const rect = target.getBoundingClientRect();
      const x = rect.left + rect.width * (0.35 + ((Math.floor(${elapsedMs} / 5000) % 3) * 0.15));
      const y = rect.top + rect.height * 0.55;
      return { x, y };
    })()
  `,
  );
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
