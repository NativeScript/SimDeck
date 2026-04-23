/// <reference lib="webworker" />

import {
  appendBytes,
  consumeBinaryVideoPackets,
  decoderDescriptionBytes,
  decoderDescriptionKey,
  hexToUint8Array,
} from "../features/stream/streamProtocol";
import { createEmptyStreamStats } from "../features/stream/stats";
import { VideoFrameRenderer } from "../features/stream/videoFrameRenderer";
import type {
  MainToWorkerMessage,
  StreamPacket,
  StreamConnectTarget,
  StreamPacketMetadata,
  StreamStatus,
  WorkerToMainMessage,
} from "../features/stream/streamTypes";

const workerScope = self as DedicatedWorkerGlobalScope;
const STATS_POST_INTERVAL_MS = 120;
const CLIENT_TELEMETRY_INTERVAL_MS = 1000;
const DECODE_QUEUE_SOFT_LIMIT = 8;
const REFRESH_REQUEST_INTERVAL_MS = 200;
const CLIENT_TELEMETRY_ID =
  workerScope.crypto?.randomUUID?.() ??
  `worker-${Math.random().toString(36).slice(2)}`;

let canvas: OffscreenCanvas | null = null;
let renderer: VideoFrameRenderer | null = null;
let decoder: VideoDecoder | null = null;
let abortController: AbortController | null = null;
let transport: WebTransport | null = null;
let currentConnectionId = 0;
let currentTarget: StreamConnectTarget | null = null;
let configuredDecoderKey = "";
let waitingForKeyFrame = false;
let lastStatsPostAt = 0;
let lastStatusKey = "";
let lastVideoConfigKey = "";
let lastRefreshRequestAt = 0;
let lastRenderedAt = 0;
let lastTelemetryAt = 0;
let lastTelemetryDecodedFrames = 0;
let lastTelemetryDroppedFrames = 0;
let lastTelemetryReceivedPackets = 0;
let latestStatusState = "idle";
let renderDurationTotalMs = 0;
let stats = createEmptyStreamStats();
let statsPostTimeout = 0;
let reconnectTimeout = 0;
let telemetryInFlight = false;

function describeError(error: unknown): string {
  if (error instanceof Error) {
    return error.name && error.name !== "Error"
      ? `${error.name}: ${error.message}`
      : error.message;
  }
  return String(error);
}

function isTransientTransportError(message: string): boolean {
  return (
    message.includes("WebTransport") ||
    message.includes("opening handshake failed") ||
    message.includes("stream closed") ||
    message.includes("session closed") ||
    message.includes("closed before sending")
  );
}

function postMessage(message: WorkerToMainMessage) {
  workerScope.postMessage(message);
}

function postStatus(status: StreamStatus) {
  latestStatusState = status.state;
  const statusKey = `${status.state}|${status.detail ?? ""}|${status.error ?? ""}`;
  if (statusKey === lastStatusKey) {
    return;
  }

  lastStatusKey = statusKey;
  postMessage({ type: "status", status });
}

function buildClientTelemetryUrl(): string {
  return new URL(
    "/api/client-stream-stats",
    workerScope.location.href,
  ).toString();
}

function resetTelemetryCounters() {
  lastRenderedAt = 0;
  lastTelemetryAt = 0;
  lastTelemetryDecodedFrames = 0;
  lastTelemetryDroppedFrames = 0;
  lastTelemetryReceivedPackets = 0;
  renderDurationTotalMs = 0;
  telemetryInFlight = false;
}

function updateDecodeQueueSize() {
  stats.decodeQueueSize = decoder?.decodeQueueSize ?? 0;
  stats.waitingForKeyFrame = waitingForKeyFrame;
}

function postClientTelemetry(force = false) {
  const target = currentTarget;
  if (!target || telemetryInFlight) {
    return;
  }

  const now = performance.now();
  if (
    !force &&
    lastTelemetryAt &&
    now - lastTelemetryAt < CLIENT_TELEMETRY_INTERVAL_MS
  ) {
    return;
  }

  const elapsedMs = lastTelemetryAt ? now - lastTelemetryAt : 0;
  const packetDelta = stats.receivedPackets - lastTelemetryReceivedPackets;
  const decodedDelta = stats.decodedFrames - lastTelemetryDecodedFrames;
  const droppedDelta = stats.droppedFrames - lastTelemetryDroppedFrames;
  const perSecond = elapsedMs > 0 ? 1000 / elapsedMs : 0;

  lastTelemetryAt = now;
  lastTelemetryReceivedPackets = stats.receivedPackets;
  lastTelemetryDecodedFrames = stats.decodedFrames;
  lastTelemetryDroppedFrames = stats.droppedFrames;
  updateDecodeQueueSize();

  telemetryInFlight = true;
  void fetch(buildClientTelemetryUrl(), {
    body: JSON.stringify({
      ...stats,
      clientId: CLIENT_TELEMETRY_ID,
      connectionId: currentConnectionId,
      decodedFps: decodedDelta * perSecond,
      droppedFps: droppedDelta * perSecond,
      kind: "worker",
      packetFps: packetDelta * perSecond,
      status: latestStatusState,
      timestampMs: Date.now(),
      udid: target.udid,
      url: workerScope.location.href,
      userAgent: workerScope.navigator.userAgent,
    }),
    cache: "no-store",
    headers: { "content-type": "application/json" },
    method: "POST",
  })
    .catch(() => {
      // Telemetry is diagnostic only; never disturb the stream.
    })
    .finally(() => {
      telemetryInFlight = false;
    });
}

function flushStats() {
  statsPostTimeout = 0;
  lastStatsPostAt = performance.now();
  updateDecodeQueueSize();
  postMessage({ type: "stats", stats: { ...stats } });
  postClientTelemetry();
}

function clearReconnectTimeout() {
  if (!reconnectTimeout) {
    return;
  }
  clearTimeout(reconnectTimeout);
  reconnectTimeout = 0;
}

function postStats(force = false) {
  const now = performance.now();
  if (
    force ||
    lastStatsPostAt === 0 ||
    now - lastStatsPostAt >= STATS_POST_INTERVAL_MS
  ) {
    if (statsPostTimeout) {
      clearTimeout(statsPostTimeout);
      statsPostTimeout = 0;
    }
    flushStats();
    return;
  }

  if (statsPostTimeout) {
    return;
  }

  const delay = Math.max(0, STATS_POST_INTERVAL_MS - (now - lastStatsPostAt));
  statsPostTimeout = setTimeout(() => {
    flushStats();
  }, delay);
}

function postVideoConfig(width: number, height: number) {
  const configKey = `${width}x${height}`;
  if (configKey === lastVideoConfigKey) {
    return;
  }

  lastVideoConfigKey = configKey;
  postMessage({ type: "video-config", size: { width, height } });
}

function ensureContext() {
  if (!canvas || renderer) {
    return;
  }
  renderer = new VideoFrameRenderer(canvas);
}

function clearCanvas() {
  if (!renderer) {
    return;
  }
  renderer.clear();
}

function resizeCanvas(width: number, height: number, devicePixelRatio: number) {
  void width;
  void height;
  void devicePixelRatio;
}

function resetDecoder() {
  if (decoder) {
    try {
      decoder.close();
    } catch {
      // Ignore shutdown races while reconnecting.
    }
  }
  decoder = null;
  configuredDecoderKey = "";
  waitingForKeyFrame = false;
}

function resetTransport() {
  if (!transport) {
    return;
  }

  try {
    transport.close({ closeCode: 0, reason: "disconnect" });
  } catch {
    // Ignore close races during reconnect.
  }
  transport = null;
}

function resetReportedState() {
  lastStatusKey = "";
  lastVideoConfigKey = "";
  lastStatsPostAt = 0;
  lastRefreshRequestAt = 0;
  clearReconnectTimeout();
  if (statsPostTimeout) {
    clearTimeout(statsPostTimeout);
    statsPostTimeout = 0;
  }
}

function disconnect() {
  postClientTelemetry(true);
  currentConnectionId += 1;
  abortController?.abort();
  abortController = null;
  currentTarget = null;
  resetTransport();
  resetDecoder();
  resetReportedState();
  resetTelemetryCounters();
  clearCanvas();
  postStatus({ state: "idle" });
}

function scheduleReconnect(
  reason: string,
  expectedConnectionId = currentConnectionId,
) {
  const reconnectTarget = currentTarget;
  if (
    !reconnectTarget ||
    reconnectTimeout ||
    expectedConnectionId !== currentConnectionId
  ) {
    return;
  }
  postStatus({ detail: reason, state: "connecting" });
  reconnectTimeout = setTimeout(() => {
    reconnectTimeout = 0;
    if (
      !reconnectTarget ||
      expectedConnectionId !== currentConnectionId ||
      reconnectTarget !== currentTarget
    ) {
      return;
    }
    void connect(reconnectTarget, true);
  }, 150);
}

async function ensureDecoder(
  metadata: StreamPacketMetadata,
  expectedConnectionId: number,
): Promise<boolean> {
  if (expectedConnectionId !== currentConnectionId) {
    return false;
  }

  if (typeof VideoDecoder !== "function") {
    postStatus({
      error: "This browser does not support WebCodecs.",
      state: "error",
    });
    return false;
  }

  const codec = metadata.codec ?? stats.codec;
  const description = metadata.description;
  const decoderKey = `${codec}:${decoderDescriptionKey(description)}:${metadata.width}x${metadata.height}`;

  if (decoder && configuredDecoderKey === decoderKey) {
    return true;
  }

  if (
    decoder &&
    !description &&
    configuredDecoderKey.startsWith(`${codec}:`) &&
    stats.width === metadata.width &&
    stats.height === metadata.height
  ) {
    return true;
  }

  if (!codec || !description) {
    return false;
  }

  resetDecoder();
  const decoderConnectionId = expectedConnectionId;
  decoder = new VideoDecoder({
    output(frame) {
      if (decoderConnectionId !== currentConnectionId) {
        frame.close();
        return;
      }

      try {
        ensureContext();
      } catch (error) {
        frame.close();
        postStatus({
          error:
            error instanceof Error
              ? error.message
              : "Unable to initialize the GPU renderer.",
          state: "error",
        });
        return;
      }

      if (!renderer) {
        frame.close();
        return;
      }

      try {
        const renderStartedAt = performance.now();
        renderer.drawFrame(frame);
        const renderFinishedAt = performance.now();
        const renderMs = renderFinishedAt - renderStartedAt;
        stats.renderedFrames += 1;
        renderDurationTotalMs += renderMs;
        stats.latestRenderMs = renderMs;
        stats.maxRenderMs = Math.max(stats.maxRenderMs, renderMs);
        stats.averageRenderMs = renderDurationTotalMs / stats.renderedFrames;
        if (lastRenderedAt > 0) {
          stats.latestFrameGapMs = renderFinishedAt - lastRenderedAt;
        }
        lastRenderedAt = renderFinishedAt;
      } catch (error) {
        frame.close();
        postStatus({
          error:
            error instanceof Error
              ? error.message
              : "Unable to render the decoded frame.",
          state: "error",
        });
        return;
      }
      frame.close();
      stats.decodedFrames += 1;
      updateDecodeQueueSize();
      postStats();
      postStatus({ state: "streaming" });
    },
    error(error) {
      if (decoderConnectionId !== currentConnectionId) {
        return;
      }
      postStatus({ error: error.message, state: "error" });
      scheduleReconnect("Reconnecting live stream…", decoderConnectionId);
    },
  });

  const config: VideoDecoderConfig = {
    codedHeight: metadata.height,
    codedWidth: metadata.width,
    codec,
    description: decoderDescriptionBytes(description),
    optimizeForLatency: true,
  };

  const support = await VideoDecoder.isConfigSupported(config);
  if (expectedConnectionId !== currentConnectionId || !decoder) {
    return false;
  }
  if (!support.supported) {
    postStatus({
      error: `Unsupported decoder configuration for ${codec} at ${metadata.width}x${metadata.height}.`,
      state: "error",
    });
    resetDecoder();
    return false;
  }
  decoder.configure(support.config ?? config);
  configuredDecoderKey = decoderKey;
  stats.codec = codec;
  return true;
}

async function handlePacket(
  packet: StreamPacket,
  expectedConnectionId: number,
) {
  if (expectedConnectionId !== currentConnectionId) {
    return;
  }

  stats.receivedPackets += 1;
  stats.frameSequence = packet.metadata.frameSequence;
  stats.width = packet.metadata.width;
  stats.height = packet.metadata.height;
  postVideoConfig(packet.metadata.width, packet.metadata.height);

  if (packet.metadata.isKeyFrame) {
    waitingForKeyFrame = false;
  } else if (waitingForKeyFrame) {
    stats.droppedFrames += 1;
    updateDecodeQueueSize();
    postStats();
    return;
  }

  const ready = await ensureDecoder(packet.metadata, expectedConnectionId);
  if (!ready || !decoder) {
    postStats();
    return;
  }

  if (expectedConnectionId !== currentConnectionId) {
    return;
  }

  if (
    decoder.decodeQueueSize > DECODE_QUEUE_SOFT_LIMIT &&
    !packet.metadata.isKeyFrame
  ) {
    stats.droppedFrames += 1;
    waitingForKeyFrame = true;
    updateDecodeQueueSize();
    postStats();
    if (currentTarget) {
      void requestRefresh(currentTarget.udid);
    }
    return;
  }

  try {
    decoder.decode(
      new EncodedVideoChunk({
        data: packet.payload,
        timestamp: Number(packet.metadata.timestampUs ?? 0),
        type: packet.metadata.isKeyFrame ? "key" : "delta",
      }),
    );
  } catch {
    if (expectedConnectionId !== currentConnectionId) {
      return;
    }
    stats.droppedFrames += 1;
    waitingForKeyFrame = true;
    updateDecodeQueueSize();
    postStats();
    if (currentTarget) {
      void requestRefresh(currentTarget.udid);
    }
    return;
  }
  updateDecodeQueueSize();
  postStats();
}

interface HealthPayload {
  webTransport?: {
    certificateHash?: {
      algorithm?: string;
      value?: string;
    };
    urlTemplate?: string;
  };
}

interface ControlHello {
  codec?: string;
  height: number;
  packet_format: string;
  simulator_udid: string;
  version: number;
  width: number;
}

function buildHealthUrl(): string {
  return new URL("/api/health", workerScope.location.href).toString();
}

function buildRefreshUrl(udid: string): string {
  return new URL(
    `/api/simulators/${encodeURIComponent(udid)}/refresh`,
    workerScope.location.href,
  ).toString();
}

function buildWebTransportUrl(urlTemplate: string, udid: string): string {
  return urlTemplate.replace("{udid}", encodeURIComponent(udid));
}

async function resolveTransportCloseReason(
  expectedConnectionId: number,
  activeTransport: WebTransport | null,
  currentReason: string,
): Promise<string> {
  if (
    !activeTransport ||
    currentReason ||
    expectedConnectionId !== currentConnectionId
  ) {
    return currentReason;
  }

  const timeoutReason = await Promise.race([
    activeTransport.closed
      .then(() => "WebTransport session closed.")
      .catch((error) => describeError(error)),
    new Promise<string>((resolve) => {
      setTimeout(() => resolve(""), 75);
    }),
  ]);
  return expectedConnectionId === currentConnectionId
    ? timeoutReason
    : currentReason;
}

async function fetchHealth(signal: AbortSignal): Promise<HealthPayload> {
  const response = await fetch(buildHealthUrl(), {
    cache: "no-store",
    signal,
  });
  if (!response.ok) {
    throw new Error(`Health check failed with status ${response.status}`);
  }
  return (await response.json()) as HealthPayload;
}

async function readStreamBytes(
  stream: ReadableStream<Uint8Array>,
): Promise<Uint8Array> {
  const reader = stream.getReader();
  let buffer: Uint8Array<ArrayBufferLike> = new Uint8Array(0);

  while (true) {
    const { done, value } = await reader.read();
    if (done) {
      return new Uint8Array(buffer);
    }
    if (!value?.length) {
      continue;
    }
    buffer = appendBytes(buffer, value);
  }
}

async function readControlHello(
  stream: ReadableStream<Uint8Array>,
): Promise<ControlHello> {
  const bytes = await readStreamBytes(stream);
  return JSON.parse(new TextDecoder().decode(bytes)) as ControlHello;
}

async function requestRefresh(udid: string) {
  const now = performance.now();
  if (now - lastRefreshRequestAt < REFRESH_REQUEST_INTERVAL_MS) {
    return;
  }

  lastRefreshRequestAt = now;
  try {
    await fetch(buildRefreshUrl(udid), {
      cache: "no-store",
      method: "POST",
    });
  } catch {
    // Best-effort recovery hint. The live stream stays open.
  }
}

async function connect(target: StreamConnectTarget, isReconnect = false) {
  currentConnectionId += 1;
  const connectionId = currentConnectionId;
  let transportCloseReason = "";
  currentTarget = target;
  abortController?.abort();
  abortController = new AbortController();
  clearReconnectTimeout();
  resetTransport();
  resetDecoder();
  resetReportedState();
  clearCanvas();
  stats = createEmptyStreamStats();
  resetTelemetryCounters();
  if (isReconnect) {
    stats.reconnects += 1;
  }
  postStats(true);
  postStatus({ detail: "Opening live stream…", state: "connecting" });

  try {
    if (typeof WebTransport !== "function") {
      throw new Error("This browser does not support WebTransport.");
    }

    const health = await fetchHealth(abortController.signal);
    const urlTemplate = health.webTransport?.urlTemplate;
    const certificateHash = health.webTransport?.certificateHash?.value;
    if (!urlTemplate || !certificateHash) {
      throw new Error(
        "Server did not provide WebTransport connection details.",
      );
    }

    transport = new WebTransport(
      buildWebTransportUrl(urlTemplate, target.udid),
      {
        congestionControl: "low-latency",
        serverCertificateHashes: [
          {
            algorithm: "sha-256",
            value: new Uint8Array(hexToUint8Array(certificateHash)),
          },
        ],
      },
    );
    void transport.closed
      .then(() => {
        if (connectionId !== currentConnectionId) {
          return;
        }
        transportCloseReason ||= "WebTransport session closed.";
      })
      .catch((error) => {
        if (connectionId !== currentConnectionId) {
          return;
        }
        transportCloseReason = describeError(error);
      });
    await transport.ready;

    const incomingStreams = transport.incomingUnidirectionalStreams.getReader();
    const controlResult = await incomingStreams.read();
    if (controlResult.done || !controlResult.value) {
      throw new Error("WebTransport closed before sending control stream.");
    }
    const hello = await readControlHello(controlResult.value);
    if (hello.packet_format !== "binary-video-v1") {
      throw new Error(
        `Unsupported WebTransport packet format ${hello.packet_format}.`,
      );
    }
    stats.codec = hello.codec ?? "";
    postVideoConfig(hello.width, hello.height);
    postStats(true);

    const videoResult = await incomingStreams.read();
    if (videoResult.done || !videoResult.value) {
      throw new Error("WebTransport closed before sending video stream.");
    }

    let buffer: Uint8Array<ArrayBufferLike> = new Uint8Array(0);
    const reader = videoResult.value.getReader();
    while (connectionId === currentConnectionId) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      if (!value?.length) {
        continue;
      }

      buffer = appendBytes(buffer, value);
      const result = consumeBinaryVideoPackets(buffer);
      buffer = result.remainder;
      for (const packet of result.packets) {
        if (connectionId !== currentConnectionId) {
          return;
        }
        packet.metadata.codec ??= hello.codec;
        await handlePacket(packet, connectionId);
      }
    }

    if (connectionId === currentConnectionId) {
      scheduleReconnect("Live stream ended. Reconnecting…", connectionId);
    }
  } catch (error) {
    if ((error as Error).name === "AbortError") {
      return;
    }

    const message = describeError(error);
    const closeReason = await resolveTransportCloseReason(
      connectionId,
      transport,
      transportCloseReason,
    );
    const errorMessage =
      closeReason && closeReason !== message
        ? `${message} (${closeReason})`
        : message;
    if (connectionId === currentConnectionId) {
      if (isTransientTransportError(errorMessage)) {
        postStatus({
          detail: "Reconnecting live stream…",
          state: "connecting",
        });
      } else {
        postStatus({
          error: errorMessage,
          state: "error",
        });
      }
      scheduleReconnect("Reconnecting live stream…", connectionId);
    }
  }
}

workerScope.onmessage = (event: MessageEvent<MainToWorkerMessage>) => {
  const message = event.data;
  switch (message.type) {
    case "attach-canvas":
      canvas = message.canvas;
      renderer = null;
      try {
        ensureContext();
        clearCanvas();
      } catch (error) {
        postStatus({
          error:
            error instanceof Error
              ? error.message
              : "Unable to initialize the GPU renderer.",
          state: "error",
        });
      }
      break;
    case "resize":
      resizeCanvas(message.width, message.height, message.devicePixelRatio);
      break;
    case "connect":
      void connect(message.target);
      break;
    case "disconnect":
      disconnect();
      break;
    case "clear":
      clearCanvas();
      break;
  }
};
