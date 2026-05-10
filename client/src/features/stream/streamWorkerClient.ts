import {
  accessTokenFromLocation,
  apiHeaders,
  fetchHealth,
} from "../../api/client";
import { apiUrl } from "../../api/config";
import type { HealthResponse } from "../../api/types";
import { createEmptyStreamStats } from "./stats";
import type {
  StreamConnectTarget,
  StreamConfig,
  StreamQualityPreset,
  StreamStats,
  StreamTransport,
  WorkerToMainMessage,
} from "./streamTypes";

const HAVE_CURRENT_DATA = 2;
const WEBRTC_CONTROL_CHANNEL_LABEL = "simdeck-control";
const WEBRTC_TELEMETRY_CHANNEL_LABEL = "simdeck-telemetry";
const WEBRTC_RGBA_CHANNEL_LABEL = "simdeck-rgba";
const WEBRTC_FIRST_FRAME_TIMEOUT_MS = 10000;
const WEBRTC_STALLED_FRAME_TIMEOUT_MS = 3000;
const WEBRTC_LOCAL_RECEIVER_BUFFER_SECONDS = 0.001;
const WEBRTC_REMOTE_RECEIVER_BUFFER_SECONDS = 0.06;
const WEBRTC_LOCAL_DISCONNECTED_GRACE_MS = 1000;
const WEBRTC_REMOTE_DISCONNECTED_GRACE_MS = 10000;
const WEBRTC_REMOTE_ICE_RESTART_GRACE_MS = 1500;
const WEBRTC_RECONNECT_BASE_DELAY_MS = 250;
const WEBRTC_RECONNECT_MAX_DELAY_MS = 1000;
const H264_WS_FIRST_FRAME_TIMEOUT_MS = 10000;
const H264_WS_STALLED_FRAME_TIMEOUT_MS = 5000;
const H264_WS_HEADER_BYTES = 40;
const H264_WS_MAGIC = 0x53444831;
const H264_WS_FLAG_KEYFRAME = 1 << 0;
const H264_WS_FLAG_CONFIG = 1 << 1;
const WEBRTC_RGBA_CHUNK_HEADER_BYTES = 48;
const WEBRTC_RGBA_CHUNK_MAGIC = 0x53445243;
const WEBRTC_RGBA_VERSION = 1;
const WEBRTC_RGBA_FORMAT_RGBA8888 = 1;
const H264_WS_LOCAL_AUTO_PROFILES: StreamQualityPreset[] = [
  "low",
  "economy",
  "smooth",
  "balanced",
  "full",
];
const H264_WS_REMOTE_AUTO_PROFILES: StreamQualityPreset[] = [
  "tiny",
  "low",
  "economy",
  "smooth",
  "balanced",
  "full",
];
const H264_WS_LOCAL_INITIAL_AUTO_PROFILE: StreamQualityPreset = "full";
const H264_WS_REMOTE_INITIAL_AUTO_PROFILE: StreamQualityPreset = "low";
const H264_WS_ADAPTIVE_SAMPLE_MS = 1000;
const H264_WS_AUTO_STABLE_SAMPLES_TO_UPGRADE = 3;
const CONTROL_BACKLOG_DROP_BYTES = 4096;

let activeWebRtcControlChannel: RTCDataChannel | null = null;
let activeWebRtcTelemetryChannel: RTCDataChannel | null = null;
let activeInputSocket: WebSocket | null = null;
let activeH264StreamSocket: WebSocket | null = null;
let activeStreamClient: StreamWorkerClient | null = null;

export type StreamBackend = "h264-ws" | "webrtc";

export function sendWebRtcControlMessage(
  encoded: string,
  options: { dropIfBacklogged?: boolean } = {},
): boolean {
  return (
    sendDataChannelMessage(activeWebRtcControlChannel, encoded, options) ||
    sendWebSocketMessage(activeInputSocket, encoded, options)
  );
}

export function sendStreamClientStats(stats: unknown): boolean {
  const encoded = JSON.stringify({ stats, type: "clientStats" });
  return (
    sendDataChannelMessage(activeWebRtcTelemetryChannel, encoded) ||
    sendWebSocketMessage(activeH264StreamSocket, encoded)
  );
}

export function sendWebRtcStreamControl(options: {
  forceKeyframe?: boolean;
  snapshot?: boolean;
}): boolean {
  return sendDataChannelMessage(
    activeWebRtcControlChannel,
    JSON.stringify({ ...options, type: "streamControl" }),
  );
}

function sendStreamQualityConfig(config: StreamConfig): boolean {
  const encoded = JSON.stringify({
    config: streamQualityPayload(config),
    type: "streamQuality",
  });
  return (
    sendDataChannelMessage(activeWebRtcControlChannel, encoded) ||
    sendWebSocketMessage(activeH264StreamSocket, encoded)
  );
}

function sendDataChannelMessage(
  channel: RTCDataChannel | null,
  encoded: string,
  options: { dropIfBacklogged?: boolean } = {},
): boolean {
  if (channel?.readyState !== "open") {
    return false;
  }
  if (
    options.dropIfBacklogged &&
    channel.bufferedAmount > CONTROL_BACKLOG_DROP_BYTES
  ) {
    return true;
  }
  channel.send(encoded);
  return true;
}

function sendWebSocketMessage(
  socket: WebSocket | null,
  encoded: string,
  options: { dropIfBacklogged?: boolean } = {},
): boolean {
  if (socket?.readyState !== WebSocket.OPEN) {
    return false;
  }
  if (
    options.dropIfBacklogged &&
    socket.bufferedAmount > CONTROL_BACKLOG_DROP_BYTES
  ) {
    return true;
  }
  socket.send(encoded);
  return true;
}

function compareVideoToImage(
  video: HTMLVideoElement,
  source: ImageBitmap,
): VisualArtifactSample {
  const width = Math.min(240, video.videoWidth, source.width);
  const height = Math.max(
    1,
    Math.round(width * (video.videoHeight / video.videoWidth)),
  );
  const sourceCanvas = document.createElement("canvas");
  sourceCanvas.width = width;
  sourceCanvas.height = height;
  const videoCanvas = document.createElement("canvas");
  videoCanvas.width = width;
  videoCanvas.height = height;
  const sourceContext = sourceCanvas.getContext("2d", {
    willReadFrequently: true,
  });
  const videoContext = videoCanvas.getContext("2d", {
    willReadFrequently: true,
  });
  if (!sourceContext || !videoContext) {
    return {
      badPixelRatio: 1,
      maxPixelDiff: 255,
      maxTileMeanDiff: 255,
      meanDiff: 255,
    };
  }
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
  let maxPixelDiff = 0;
  let sum = 0;

  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const offset = (y * width + x) * 4;
      const diff =
        (Math.abs(sourceData[offset] - videoData[offset]) +
          Math.abs(sourceData[offset + 1] - videoData[offset + 1]) +
          Math.abs(sourceData[offset + 2] - videoData[offset + 2])) /
        3;
      sum += diff;
      maxPixelDiff = Math.max(maxPixelDiff, diff);
      if (diff > 48) {
        badPixels += 1;
      }
      const tileIndex =
        Math.floor(y / tileSize) * tileColumns + Math.floor(x / tileSize);
      tileSums[tileIndex] += diff;
      tileCounts[tileIndex] += 1;
    }
  }

  let maxTileMeanDiff = 0;
  for (let index = 0; index < tileSums.length; index += 1) {
    if (tileCounts[index] > 0) {
      maxTileMeanDiff = Math.max(
        maxTileMeanDiff,
        tileSums[index] / tileCounts[index],
      );
    }
  }

  return {
    badPixelRatio: badPixels / (width * height),
    maxPixelDiff,
    maxTileMeanDiff,
    meanDiff: sum / (width * height),
  };
}

export function buildStreamTarget(
  udid: string,
  options: {
    clientId?: string;
    platform?: string;
    remote?: boolean;
    streamConfig?: StreamConfig;
    transport?: StreamTransport;
  } = {},
): StreamConnectTarget {
  return {
    clientId: options.clientId,
    platform: options.platform,
    remote: options.remote,
    streamConfig: options.streamConfig,
    transport: options.transport,
    udid,
  };
}

function webSocketApiUrl(path: string): string {
  const url = new URL(apiUrl(path), window.location.href);
  const token = accessTokenFromLocation();
  if (token) {
    url.searchParams.set("simdeckToken", token);
  }
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

function isLoopbackHost(hostname: string): boolean {
  return (
    hostname === "localhost" ||
    hostname === "127.0.0.1" ||
    hostname === "::1" ||
    hostname.endsWith(".localhost")
  );
}

export function canUseWebRtc(): boolean {
  return typeof RTCPeerConnection === "function";
}

export function canUseH264WebSocket(): boolean {
  const { EncodedVideoChunk, VideoDecoder } = webCodecsConstructors();
  return (
    typeof WebSocket === "function" &&
    typeof VideoDecoder === "function" &&
    typeof EncodedVideoChunk === "function"
  );
}

interface StreamClientBackend {
  attachCanvas(canvasElement: HTMLCanvasElement): void;
  clear(): void;
  connect(target: StreamConnectTarget): void | Promise<void>;
  collectVisualArtifactSample?(
    udid: string,
  ): Promise<VisualArtifactSample | null>;
  destroy(): void;
  disconnect(): void;
  applyStreamConfig?(config?: StreamConfig): void | Promise<void>;
  sendControl?(payload: unknown): boolean;
}

export interface VisualArtifactSample {
  badPixelRatio: number;
  maxPixelDiff: number;
  maxTileMeanDiff: number;
  meanDiff: number;
}

interface H264WebSocketFrame {
  config: Uint8Array;
  height: number;
  isKeyFrame: boolean;
  payload: Uint8Array;
  sequence: number;
  timestampUs: number;
  width: number;
}

interface WebRtcRgbaFrame {
  height: number;
  payload: Uint8Array;
  sequence: number;
  timestampUs: number;
  width: number;
}

interface WebRtcRgbaChunk {
  chunkOffset: number;
  height: number;
  payload: Uint8Array;
  payloadBytes: number;
  sequence: number;
  timestampUs: number;
  width: number;
}

interface WebRtcRgbaAssembly {
  buffer: Uint8Array;
  height: number;
  receivedBytes: number;
  receivedRanges: Array<[number, number]>;
  sequence: number;
  timestampUs: number;
  width: number;
}

interface WebCodecsVideoFrame {
  close(): void;
  codedHeight?: number;
  codedWidth?: number;
  displayHeight?: number;
  displayWidth?: number;
  timestamp?: number;
}

interface WebCodecsVideoDecoderConfig {
  codec: string;
  codedHeight?: number;
  codedWidth?: number;
  description?: BufferSource;
  hardwareAcceleration?:
    | "no-preference"
    | "prefer-hardware"
    | "prefer-software";
  optimizeForLatency?: boolean;
}

interface WebCodecsVideoDecoder {
  readonly decodeQueueSize: number;
  readonly state?: string;
  close(): void;
  configure(config: WebCodecsVideoDecoderConfig): void;
  decode(chunk: WebCodecsEncodedVideoChunk): void;
}

interface WebCodecsEncodedVideoChunk {
  readonly byteLength?: number;
}

interface WebCodecsEncodedVideoChunkConstructor {
  new (init: {
    data: BufferSource;
    timestamp: number;
    type: "delta" | "key";
  }): WebCodecsEncodedVideoChunk;
}

interface WebCodecsVideoDecoderConstructor {
  new (init: {
    error: (error: Error) => void;
    output: (frame: WebCodecsVideoFrame) => void;
  }): WebCodecsVideoDecoder;
  isConfigSupported?: (config: WebCodecsVideoDecoderConfig) => Promise<{
    supported: boolean;
  }>;
}

interface WebRtcAnswerPayload extends RTCSessionDescriptionInit {
  video?: {
    height?: number;
    width?: number;
  };
}

interface PendingVideoFrame {
  frame: WebCodecsVideoFrame;
  sequence: number | null;
}

function webCodecsConstructors(): {
  EncodedVideoChunk?: WebCodecsEncodedVideoChunkConstructor;
  VideoDecoder?: WebCodecsVideoDecoderConstructor;
} {
  return globalThis as typeof globalThis & {
    EncodedVideoChunk?: WebCodecsEncodedVideoChunkConstructor;
    VideoDecoder?: WebCodecsVideoDecoderConstructor;
  };
}

function hasArrayBufferMethod(
  value: unknown,
): value is { arrayBuffer: () => Promise<ArrayBuffer> } {
  return (
    typeof value === "object" &&
    value !== null &&
    "arrayBuffer" in value &&
    typeof (value as { arrayBuffer?: unknown }).arrayBuffer === "function"
  );
}

class H264WebSocketStreamClient implements StreamClientBackend {
  private adaptiveInterval = 0;
  private adaptiveLastAt = 0;
  private adaptiveLastDecodedFrames = 0;
  private adaptiveLastRenderedFrames = 0;
  private autoProfile: StreamQualityPreset =
    H264_WS_REMOTE_INITIAL_AUTO_PROFILE;
  private autoProfileStableSamples = 0;
  private animationFrame = 0;
  private canvas: HTMLCanvasElement | null = null;
  private canvasContext: CanvasRenderingContext2D | null = null;
  private connectGeneration = 0;
  private decoder: WebCodecsVideoDecoder | null = null;
  private decoderConfigKey = "";
  private frameWatchdogTimeout = 0;
  private inputSocket: WebSocket | null = null;
  private h264DecodeTimestampUs = 0;
  private lastFrameAt = 0;
  private pendingFrame: PendingVideoFrame | null = null;
  private pendingFrameSequences = new Map<number, number>();
  private reportedVideoHeight = 0;
  private reportedVideoWidth = 0;
  private shouldReconnect = false;
  private stats: StreamStats = createEmptyStreamStats();
  private stalledFrameWatchdogCount = 0;
  private streamingReported = false;
  private streamConfig?: StreamConfig;
  private streamSocket: WebSocket | null = null;
  private streamTarget: StreamConnectTarget | null = null;
  private waitingForKeyFrame = true;

  constructor(
    private readonly onMessage: (message: WorkerToMainMessage) => void,
  ) {}

  attachCanvas(canvasElement: HTMLCanvasElement) {
    this.canvas = canvasElement;
    this.canvasContext = canvasElement.getContext("2d", {
      alpha: false,
      desynchronized: true,
    });
  }

  async connect(target: StreamConnectTarget) {
    this.disconnect();
    if (!this.canvas) {
      return;
    }
    if (!canUseH264WebSocket()) {
      this.handleError("H264 WebSocket requires browser WebCodecs support.");
      return;
    }

    const generation = ++this.connectGeneration;
    this.autoProfile = initialH264AutoProfile(target);
    this.autoProfileStableSamples = 0;
    const effectiveConfig = h264WebSocketStreamConfig(
      target.streamConfig,
      this.autoProfile,
    );
    this.shouldReconnect = true;
    this.streamTarget = target;
    this.streamConfig = target.streamConfig;
    this.streamingReported = false;
    this.waitingForKeyFrame = true;
    this.h264DecodeTimestampUs = 0;
    this.stats = createEmptyStreamStats();
    this.stats.codec = "h264-ws";
    this.stats.waitingForKeyFrame = true;
    this.lastFrameAt = 0;
    this.stalledFrameWatchdogCount = 0;
    this.onMessage({ type: "stats", stats: { ...this.stats } });
    this.onMessage({
      type: "status",
      status: { detail: "Opening H264 WebSocket stream", state: "connecting" },
    });

    const socket = new WebSocket(
      webSocketApiUrl(
        `/api/simulators/${encodeURIComponent(target.udid)}/h264${streamQualityQuery(effectiveConfig)}`,
      ),
    );
    socket.binaryType = "arraybuffer";
    this.streamSocket = socket;
    socket.addEventListener("open", () => {
      if (socket === this.streamSocket) {
        socket.binaryType = "arraybuffer";
        activeH264StreamSocket = socket;
        this.startAdaptiveQuality(generation);
      }
    });
    socket.addEventListener("message", (event) => {
      if (socket !== this.streamSocket) {
        return;
      }
      this.recordH264SocketMessage(event.data);
      if (hasArrayBufferMethod(event.data)) {
        void event.data.arrayBuffer().then((buffer) => {
          if (socket === this.streamSocket) {
            this.handleSocketMessage(buffer);
          }
        });
        return;
      }
      this.handleSocketMessage(event.data);
    });
    socket.addEventListener("close", () => {
      if (activeH264StreamSocket === socket) {
        activeH264StreamSocket = null;
      }
      if (socket === this.streamSocket && this.shouldReconnect) {
        this.handleError("H264 WebSocket stream closed.");
      }
    });
    socket.addEventListener("error", () => {
      if (socket === this.streamSocket) {
        this.handleError("H264 WebSocket stream failed.");
      }
    });

    this.connectInputSocket(target, generation);
    this.scheduleFrameWatchdog(generation);
  }

  disconnect() {
    this.shouldReconnect = false;
    this.connectGeneration += 1;
    this.clearAdaptiveQuality();
    this.clearFrameWatchdog();
    window.cancelAnimationFrame(this.animationFrame);
    this.animationFrame = 0;
    this.closePendingFrame();
    this.closeDecoder();
    this.pendingFrameSequences.clear();
    this.h264DecodeTimestampUs = 0;
    this.streamSocket?.close();
    if (activeH264StreamSocket === this.streamSocket) {
      activeH264StreamSocket = null;
    }
    this.streamSocket = null;
    this.inputSocket?.close();
    if (activeInputSocket === this.inputSocket) {
      activeInputSocket = null;
    }
    this.inputSocket = null;
    this.streamingReported = false;
    this.streamTarget = null;
    this.lastFrameAt = 0;
    this.stalledFrameWatchdogCount = 0;
    this.waitingForKeyFrame = true;
    this.reportedVideoHeight = 0;
    this.reportedVideoWidth = 0;
  }

  destroy() {
    this.disconnect();
  }

  clear() {
    if (!this.canvas) {
      return;
    }
    this.ensureCanvasContext()?.clearRect(
      0,
      0,
      this.canvas.width,
      this.canvas.height,
    );
  }

  sendControl(payload: unknown): boolean {
    if (
      payload &&
      typeof payload === "object" &&
      "type" in payload &&
      payload.type === "streamControl"
    ) {
      return sendWebSocketMessage(this.streamSocket, JSON.stringify(payload));
    }
    return sendWebSocketMessage(this.inputSocket, JSON.stringify(payload));
  }

  async applyStreamConfig(config?: StreamConfig) {
    this.autoProfile = initialH264AutoProfile(this.streamTarget);
    this.autoProfileStableSamples = 0;
    const effectiveConfig = h264WebSocketStreamConfig(config, this.autoProfile);
    this.streamConfig = config;
    this.clearAdaptiveQuality();
    if (config?.quality === "auto") {
      this.startAdaptiveQuality(this.connectGeneration);
    }
    if (effectiveConfig) {
      if (!sendStreamQualityConfig(effectiveConfig)) {
        await postStreamConfigWithAuthRetry(effectiveConfig, {
          remote: this.streamTarget?.remote,
        });
      }
      this.sendControl({
        forceKeyframe: true,
        snapshot: true,
        type: "streamControl",
      });
    }
  }

  private connectInputSocket(target: StreamConnectTarget, generation: number) {
    const socket = new WebSocket(
      webSocketApiUrl(
        `/api/simulators/${encodeURIComponent(target.udid)}/input`,
      ),
    );
    this.inputSocket = socket;
    activeInputSocket = socket;
    socket.addEventListener("open", () => {
      if (generation === this.connectGeneration) {
        activeInputSocket = socket;
      }
    });
    socket.addEventListener("close", () => {
      if (activeInputSocket === socket) {
        activeInputSocket = null;
      }
    });
    socket.addEventListener("error", () => {
      if (generation === this.connectGeneration) {
        console.warn("H264 input WebSocket failed.");
      }
    });
  }

  private handleSocketMessage(data: unknown) {
    const frame = parseH264WebSocketFrame(data);
    if (!frame) {
      this.stats.h264ParseFailures += 1;
      this.onMessage({ type: "stats", stats: { ...this.stats } });
      return;
    }
    this.stats.receivedPackets += 1;
    this.stats.frameSequence = frame.sequence;
    this.stats.width = frame.width;
    this.stats.height = frame.height;
    this.stats.codec = "h264-ws";

    if (this.waitingForKeyFrame && !frame.isKeyFrame) {
      this.stats.droppedFrames += 1;
      this.stats.waitingForKeyFrame = true;
      this.onMessage({ type: "stats", stats: { ...this.stats } });
      this.sendControl({ forceKeyframe: true, type: "streamControl" });
      return;
    }

    if (!this.ensureDecoderConfigured(frame)) {
      return;
    }
    const decoder = this.decoder;
    if (!decoder) {
      return;
    }
    if (decoder.state === "closed") {
      this.closeDecoder();
      this.waitingForKeyFrame = true;
      this.stats.waitingForKeyFrame = true;
      this.sendControl({ forceKeyframe: true, type: "streamControl" });
      return;
    }
    this.stats.decodeQueueSize = decoder.decodeQueueSize;

    const { EncodedVideoChunk } = webCodecsConstructors();
    if (!EncodedVideoChunk) {
      this.handleError("H264 WebSocket requires EncodedVideoChunk support.");
      return;
    }
    const decodeTimestampUs = this.nextDecodeTimestampUs(frame.timestampUs);
    try {
      this.pendingFrameSequences.set(decodeTimestampUs, frame.sequence);
      this.trimPendingFrameSequenceMap();
      decoder.decode(
        new EncodedVideoChunk({
          data: frame.payload as BufferSource,
          timestamp: decodeTimestampUs,
          type: frame.isKeyFrame ? "key" : "delta",
        }),
      );
      this.waitingForKeyFrame = false;
      this.stats.waitingForKeyFrame = false;
      this.stats.decodeQueueSize = decoder.decodeQueueSize;
    } catch (error) {
      this.pendingFrameSequences.delete(decodeTimestampUs);
      this.closeDecoder();
      this.waitingForKeyFrame = true;
      this.stats.waitingForKeyFrame = true;
      this.stats.decoderDroppedFrames += 1;
      this.sendControl({ forceKeyframe: true, type: "streamControl" });
      this.onMessage({ type: "stats", stats: { ...this.stats } });
      if (!this.streamingReported) {
        this.onMessage({
          type: "status",
          status: { detail: "Recovering H264 decoder", state: "connecting" },
        });
      }
    }
  }

  private ensureDecoderConfigured(frame: H264WebSocketFrame): boolean {
    const { VideoDecoder } = webCodecsConstructors();
    if (!VideoDecoder) {
      this.handleError("H264 WebSocket requires VideoDecoder support.");
      return false;
    }
    if (this.decoder?.state === "closed") {
      this.closeDecoder();
    }
    if (this.decoder && frame.config.byteLength === 0) {
      return true;
    }
    const configKey = h264DecoderConfigKey(frame);
    if (this.decoder && this.decoderConfigKey === configKey) {
      return true;
    }

    this.closeDecoder();
    const decoder = new VideoDecoder({
      error: (error) => {
        this.stats.decoderDroppedFrames += 1;
        this.waitingForKeyFrame = true;
        this.stats.waitingForKeyFrame = true;
        if (this.decoder === decoder) {
          this.closeDecoder();
        }
        this.sendControl({ forceKeyframe: true, type: "streamControl" });
        this.onMessage({ type: "stats", stats: { ...this.stats } });
        if (!this.streamingReported) {
          this.onMessage({
            type: "status",
            status: { detail: "Recovering H264 decoder", state: "connecting" },
          });
        }
      },
      output: (videoFrame) => this.queueDecodedFrame(videoFrame),
    });
    const config: WebCodecsVideoDecoderConfig = {
      codec: h264CodecStringFromAvcC(frame.config) ?? "avc1.42E01F",
      codedHeight: frame.height || undefined,
      codedWidth: frame.width || undefined,
      hardwareAcceleration: "prefer-hardware",
      optimizeForLatency: true,
    };
    if (frame.config.byteLength > 0) {
      config.description = arrayBufferCopy(frame.config);
    }
    try {
      decoder.configure(config);
    } catch (error) {
      decoder.close();
      this.handleError(error instanceof Error ? error.message : String(error));
      return false;
    }
    this.decoder = decoder;
    this.decoderConfigKey = configKey;
    return true;
  }

  private nextDecodeTimestampUs(sourceTimestampUs: number): number {
    const sourceTimestamp = Number.isFinite(sourceTimestampUs)
      ? Math.max(0, Math.floor(sourceTimestampUs))
      : 0;
    this.h264DecodeTimestampUs = Math.max(
      sourceTimestamp,
      this.h264DecodeTimestampUs + 1,
    );
    return this.h264DecodeTimestampUs;
  }

  private queueDecodedFrame(videoFrame: WebCodecsVideoFrame) {
    const sequence = this.takeRenderedFrameSequence(videoFrame);
    this.stats.decodedFrames += 1;
    if (this.pendingFrame) {
      this.stats.droppedFrames += 1;
      this.closePendingFrame();
    }
    this.pendingFrame = { frame: videoFrame, sequence };
    this.schedulePaint();
  }

  private schedulePaint() {
    if (this.animationFrame) {
      return;
    }
    this.animationFrame = window.requestAnimationFrame(this.paintPendingFrame);
  }

  private readonly paintPendingFrame = () => {
    this.animationFrame = 0;
    const pending = this.pendingFrame;
    this.pendingFrame = null;
    if (!pending) {
      return;
    }
    this.paintDecodedFrame(pending);
  };

  private paintDecodedFrame(pending: PendingVideoFrame) {
    const canvas = this.canvas;
    if (!canvas) {
      pending.frame.close();
      return;
    }
    const videoFrame = pending.frame;
    const width =
      videoFrame.displayWidth ?? videoFrame.codedWidth ?? this.stats.width;
    const height =
      videoFrame.displayHeight ?? videoFrame.codedHeight ?? this.stats.height;
    this.syncCanvasSize(width, height);
    const startedAt = performance.now();
    try {
      this.ensureCanvasContext()?.drawImage(
        videoFrame as unknown as CanvasImageSource,
        0,
        0,
        canvas.width,
        canvas.height,
      );
    } finally {
      videoFrame.close();
    }
    const finishedAt = performance.now();
    const previousFrameAt = this.lastFrameAt;
    this.lastFrameAt = finishedAt;
    this.stalledFrameWatchdogCount = 0;
    this.reportVideoConfig(canvas.width, canvas.height);
    this.stats.codec = "h264-ws";
    if (pending.sequence !== null) {
      this.stats.frameSequence = pending.sequence;
    }
    this.stats.renderedFrames += 1;
    this.stats.latestRenderMs = finishedAt - startedAt;
    this.stats.maxRenderMs = Math.max(
      this.stats.maxRenderMs,
      this.stats.latestRenderMs,
    );
    this.stats.averageRenderMs =
      this.stats.averageRenderMs <= 0
        ? this.stats.latestRenderMs
        : this.stats.averageRenderMs * 0.9 + this.stats.latestRenderMs * 0.1;
    this.stats.latestFrameGapMs =
      previousFrameAt > 0 ? finishedAt - previousFrameAt : 0;
    this.stats.decodeQueueSize = this.decoder?.decodeQueueSize ?? 0;
    this.stats.waitingForKeyFrame = false;
    this.onMessage({ type: "stats", stats: { ...this.stats } });
    if (!this.streamingReported) {
      this.streamingReported = true;
      this.onMessage({
        type: "status",
        status: {
          detail: "H264 WebSocket stream connected",
          state: "streaming",
        },
      });
    }
  }

  private ensureCanvasContext(): CanvasRenderingContext2D | null {
    const canvas = this.canvas;
    if (!canvas) {
      this.canvasContext = null;
      return null;
    }
    if (this.canvasContext?.canvas === canvas) {
      return this.canvasContext;
    }
    this.canvasContext = canvas.getContext("2d", {
      alpha: false,
      desynchronized: true,
    });
    return this.canvasContext;
  }

  private syncCanvasSize(width: number, height: number) {
    if (!this.canvas) {
      return;
    }
    const nextWidth = Math.max(1, Math.round(width));
    const nextHeight = Math.max(1, Math.round(height));
    if (this.canvas.width !== nextWidth) {
      this.canvas.width = nextWidth;
    }
    if (this.canvas.height !== nextHeight) {
      this.canvas.height = nextHeight;
    }
  }

  private reportVideoConfig(width: number, height: number) {
    if (
      this.reportedVideoWidth === width &&
      this.reportedVideoHeight === height
    ) {
      return;
    }
    this.reportedVideoWidth = width;
    this.reportedVideoHeight = height;
    this.onMessage({ type: "video-config", size: { height, width } });
  }

  private scheduleFrameWatchdog(generation: number) {
    this.clearFrameWatchdog();
    this.frameWatchdogTimeout = window.setTimeout(
      () => {
        this.frameWatchdogTimeout = 0;
        if (generation !== this.connectGeneration || !this.shouldReconnect) {
          return;
        }
        const now = performance.now();
        if (this.lastFrameAt <= 0) {
          this.handleError("H264 WebSocket stream did not render a frame.");
          return;
        }
        if (now - this.lastFrameAt > H264_WS_STALLED_FRAME_TIMEOUT_MS) {
          this.stalledFrameWatchdogCount += 1;
          this.sendControl({ snapshot: true, type: "streamControl" });
          this.sendControl({ forceKeyframe: true, type: "streamControl" });
          if (this.stalledFrameWatchdogCount >= 2 && this.streamTarget) {
            const target = this.streamTarget;
            this.onMessage({
              type: "status",
              status: {
                detail: "Reconnecting stalled H264 WebSocket stream",
                state: "connecting",
              },
            });
            void this.connect({
              ...target,
              streamConfig: this.streamConfig,
            });
            return;
          }
        } else {
          this.stalledFrameWatchdogCount = 0;
        }
        this.scheduleFrameWatchdog(generation);
      },
      this.lastFrameAt > 0
        ? H264_WS_STALLED_FRAME_TIMEOUT_MS
        : H264_WS_FIRST_FRAME_TIMEOUT_MS,
    );
  }

  private clearFrameWatchdog() {
    if (!this.frameWatchdogTimeout) {
      return;
    }
    window.clearTimeout(this.frameWatchdogTimeout);
    this.frameWatchdogTimeout = 0;
  }

  private closeDecoder() {
    try {
      this.decoder?.close();
    } catch {
      // Closing a failed decoder is best effort.
    }
    this.decoder = null;
    this.decoderConfigKey = "";
    this.pendingFrameSequences.clear();
  }

  private closePendingFrame() {
    if (!this.pendingFrame) {
      return;
    }
    try {
      this.pendingFrame.frame.close();
    } catch {
      // VideoFrame cleanup is best effort during disconnect/replacement.
    }
    this.pendingFrame = null;
  }

  private takeRenderedFrameSequence(
    videoFrame: WebCodecsVideoFrame,
  ): number | null {
    if (typeof videoFrame.timestamp !== "number") {
      return null;
    }
    const sequence = this.pendingFrameSequences.get(videoFrame.timestamp);
    this.pendingFrameSequences.delete(videoFrame.timestamp);
    return typeof sequence === "number" ? sequence : null;
  }

  private trimPendingFrameSequenceMap() {
    while (this.pendingFrameSequences.size > 256) {
      const firstKey = this.pendingFrameSequences.keys().next().value;
      if (typeof firstKey !== "number") {
        break;
      }
      this.pendingFrameSequences.delete(firstKey);
    }
  }

  private recordH264SocketMessage(data: unknown) {
    this.stats.h264SocketMessages += 1;
    const byteLength =
      typeof data === "string"
        ? data.length
        : typeof (data as { byteLength?: unknown })?.byteLength === "number"
          ? ((data as { byteLength: number }).byteLength ?? 0)
          : typeof (data as { size?: unknown })?.size === "number"
            ? ((data as { size: number }).size ?? 0)
            : 0;
    this.stats.h264SocketBytes += Math.max(0, byteLength);
    this.stats.h264SocketMessageType = Object.prototype.toString.call(data);
  }

  private startAdaptiveQuality(generation: number) {
    this.clearAdaptiveQuality();
    if (this.streamConfig?.quality !== "auto") {
      return;
    }
    this.adaptiveLastAt = performance.now();
    this.adaptiveLastDecodedFrames = this.stats.decodedFrames;
    this.adaptiveLastRenderedFrames = this.stats.renderedFrames;
    this.adaptiveInterval = window.setInterval(() => {
      if (generation !== this.connectGeneration || !this.shouldReconnect) {
        this.clearAdaptiveQuality();
        return;
      }
      void this.sampleAdaptiveQuality();
    }, H264_WS_ADAPTIVE_SAMPLE_MS);
  }

  private clearAdaptiveQuality() {
    if (!this.adaptiveInterval) {
      return;
    }
    window.clearInterval(this.adaptiveInterval);
    this.adaptiveInterval = 0;
  }

  private async sampleAdaptiveQuality() {
    if (this.streamConfig?.quality !== "auto") {
      this.clearAdaptiveQuality();
      return;
    }
    const now = performance.now();
    const elapsedSeconds = Math.max((now - this.adaptiveLastAt) / 1000, 0.001);
    const renderedDelta =
      this.stats.renderedFrames - this.adaptiveLastRenderedFrames;
    const decodedDelta =
      this.stats.decodedFrames - this.adaptiveLastDecodedFrames;
    this.adaptiveLastAt = now;
    this.adaptiveLastRenderedFrames = this.stats.renderedFrames;
    this.adaptiveLastDecodedFrames = this.stats.decodedFrames;

    const renderedFps = renderedDelta / elapsedSeconds;
    const decodedFps = decodedDelta / elapsedSeconds;
    const underPressure =
      this.stats.decodeQueueSize > 1 ||
      this.stats.latestFrameGapMs > 80 ||
      this.stats.averageRenderMs > 4;
    if (underPressure) {
      this.autoProfileStableSamples = 0;
      await this.setAutoProfile(this.nextLowerAutoProfile());
      return;
    }

    if (
      renderedFps > 0 &&
      decodedFps > 0 &&
      this.stats.decodeQueueSize === 0 &&
      this.stats.latestFrameGapMs < 40 &&
      this.stats.averageRenderMs < 2
    ) {
      this.autoProfileStableSamples += 1;
    } else {
      this.autoProfileStableSamples = 0;
    }
    if (
      this.autoProfileStableSamples >= H264_WS_AUTO_STABLE_SAMPLES_TO_UPGRADE
    ) {
      this.autoProfileStableSamples = 0;
      await this.setAutoProfile(this.nextHigherAutoProfile());
    }
  }

  private nextLowerAutoProfile(): StreamQualityPreset {
    const profiles = h264AutoProfiles(this.streamTarget);
    const index = profiles.indexOf(this.autoProfile);
    return profiles[Math.max(0, index - 1)] ?? this.autoProfile;
  }

  private nextHigherAutoProfile(): StreamQualityPreset {
    const profiles = h264AutoProfiles(this.streamTarget);
    const index = profiles.indexOf(this.autoProfile);
    return (
      profiles[Math.min(profiles.length - 1, Math.max(0, index) + 1)] ??
      this.autoProfile
    );
  }

  private async setAutoProfile(profile: StreamQualityPreset) {
    if (profile === this.autoProfile || this.streamConfig?.quality !== "auto") {
      return;
    }
    this.autoProfile = profile;
    const effectiveConfig = h264WebSocketStreamConfig(
      this.streamConfig,
      this.autoProfile,
    );
    if (!effectiveConfig) {
      return;
    }
    if (!sendStreamQualityConfig(effectiveConfig)) {
      await postStreamConfigWithAuthRetry(effectiveConfig, {
        remote: this.streamTarget?.remote,
      }).catch(() => {
        // Stream-quality adaptation is opportunistic; the stream socket handles reachability.
      });
    }
    this.sendControl({ forceKeyframe: true, type: "streamControl" });
  }

  private handleError(message: string) {
    this.onMessage({
      type: "status",
      status: { error: message.replace(/\.$/, ""), state: "error" },
    });
  }
}

function parseH264WebSocketFrame(data: unknown): H264WebSocketFrame | null {
  const bytes = bytesFromBinaryMessage(data);
  if (!bytes || bytes.byteLength < H264_WS_HEADER_BYTES) {
    return null;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0, false) !== H264_WS_MAGIC || view.getUint8(4) !== 1) {
    return null;
  }
  const flags = view.getUint8(5);
  const headerBytes = view.getUint16(6, false);
  if (headerBytes < H264_WS_HEADER_BYTES || headerBytes > bytes.byteLength) {
    return null;
  }
  const configBytes = view.getUint32(32, false);
  const payloadBytes = view.getUint32(36, false);
  const payloadOffset = headerBytes + configBytes;
  const payloadEnd = payloadOffset + payloadBytes;
  if (payloadEnd > bytes.byteLength) {
    return null;
  }
  const config =
    flags & H264_WS_FLAG_CONFIG
      ? bytes.subarray(headerBytes, payloadOffset)
      : new Uint8Array();
  return {
    config,
    height: view.getUint32(28, false),
    isKeyFrame: Boolean(flags & H264_WS_FLAG_KEYFRAME),
    payload: bytes.subarray(payloadOffset, payloadEnd),
    sequence: Number(view.getBigUint64(8, false)),
    timestampUs: Number(view.getBigUint64(16, false)),
    width: view.getUint32(24, false),
  };
}

function parseWebRtcRgbaChunk(data: unknown): WebRtcRgbaChunk | null {
  const bytes = bytesFromBinaryMessage(data);
  if (!bytes || bytes.byteLength < WEBRTC_RGBA_CHUNK_HEADER_BYTES) {
    return null;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (
    view.getUint32(0, false) !== WEBRTC_RGBA_CHUNK_MAGIC ||
    view.getUint8(4) !== WEBRTC_RGBA_VERSION ||
    view.getUint8(5) !== WEBRTC_RGBA_FORMAT_RGBA8888
  ) {
    return null;
  }
  const headerBytes = view.getUint16(6, false);
  if (
    headerBytes < WEBRTC_RGBA_CHUNK_HEADER_BYTES ||
    headerBytes > bytes.byteLength
  ) {
    return null;
  }
  const width = view.getUint32(24, false);
  const height = view.getUint32(28, false);
  const payloadBytes = view.getUint32(32, false);
  const chunkOffset = view.getUint32(36, false);
  const chunkBytes = view.getUint32(40, false);
  const payloadEnd = headerBytes + chunkBytes;
  if (
    width <= 0 ||
    height <= 0 ||
    payloadBytes !== width * height * 4 ||
    chunkBytes <= 0 ||
    chunkOffset + chunkBytes > payloadBytes ||
    payloadEnd > bytes.byteLength
  ) {
    return null;
  }
  return {
    chunkOffset,
    height,
    payload: bytes.subarray(headerBytes, payloadEnd),
    payloadBytes,
    sequence: Number(view.getBigUint64(8, false)),
    timestampUs: Number(view.getBigUint64(16, false)),
    width,
  };
}

function bytesFromBinaryMessage(data: unknown): Uint8Array | null {
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  if (data instanceof ArrayBuffer) {
    return new Uint8Array(data);
  }
  if (
    Object.prototype.toString.call(data) === "[object ArrayBuffer]" &&
    typeof (data as { byteLength?: unknown }).byteLength === "number"
  ) {
    return new Uint8Array(data as ArrayBuffer);
  }
  if (
    typeof data === "object" &&
    data !== null &&
    typeof (data as { byteLength?: unknown }).byteLength === "number"
  ) {
    try {
      return new Uint8Array(data as ArrayBufferLike);
    } catch {
      return null;
    }
  }
  return null;
}

function rangeAlreadyReceived(
  ranges: Array<[number, number]>,
  start: number,
  end: number,
): boolean {
  return ranges.some(
    ([rangeStart, rangeEnd]) => start >= rangeStart && end <= rangeEnd,
  );
}

function h264CodecStringFromAvcC(config: Uint8Array): string | null {
  if (config.byteLength < 4 || config[0] !== 1) {
    return null;
  }
  return `avc1.${hexByte(config[1])}${hexByte(config[2])}${hexByte(config[3])}`;
}

function h264DecoderConfigKey(frame: H264WebSocketFrame): string {
  const codec = h264CodecStringFromAvcC(frame.config) ?? "avc1.42E01F";
  const prefix = frame.config.byteLength
    ? Array.from(frame.config.slice(0, 16), hexByte).join("")
    : "";
  return `${codec}:${frame.width}x${frame.height}:${frame.config.byteLength}:${prefix}`;
}

function arrayBufferCopy(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

function hexByte(byte: number): string {
  return byte.toString(16).padStart(2, "0").toUpperCase();
}

class WebRtcStreamClient implements StreamClientBackend {
  private animationFrame = 0;
  private canvas: HTMLCanvasElement | null = null;
  private canvasContext: CanvasRenderingContext2D | null = null;
  private connectGeneration = 0;
  private controlChannel: RTCDataChannel | null = null;
  private diagnostics = createWebRtcDiagnostics();
  private disconnectGraceTimeout = 0;
  private frameWatchdogTimeout = 0;
  private hasRenderedFrame = false;
  private iceRestartInFlight = false;
  private iceRestartTimeout = 0;
  private lastVideoFrameAt = 0;
  private peerConnection: RTCPeerConnection | null = null;
  private reconnectTimeout = 0;
  private reconnectDelayMs = WEBRTC_RECONNECT_BASE_DELAY_MS;
  private reconnecting = false;
  private remoteMode = false;
  private reportedVideoHeight = 0;
  private reportedVideoWidth = 0;
  private receiverStatsInterval = 0;
  private receiverStatsSeen = false;
  private shouldReconnect = false;
  private latestRgbaSequence = -1;
  private rgbaAssemblies = new Map<number, WebRtcRgbaAssembly>();
  private rgbaChannel: RTCDataChannel | null = null;
  private rgbaMode = false;
  private streamConfigGeneration = 0;
  private streamTarget: StreamConnectTarget | null = null;
  private telemetryChannel: RTCDataChannel | null = null;
  private stats: StreamStats = createEmptyStreamStats();
  private video: HTMLVideoElement | null = null;
  private videoFrameCallback = 0;

  constructor(
    private readonly onMessage: (message: WorkerToMainMessage) => void,
  ) {}

  attachCanvas(canvasElement: HTMLCanvasElement) {
    this.canvas = canvasElement;
    this.canvasContext = canvasElement.getContext("2d", {
      alpha: false,
      desynchronized: true,
    });
    if (
      this.video &&
      this.video.parentElement !== canvasElement.parentElement
    ) {
      canvasElement.parentElement?.insertBefore(
        this.video,
        canvasElement.nextSibling,
      );
    }
  }

  clear() {
    this.ensureCanvasContext()?.clearRect(
      0,
      0,
      this.canvas?.width ?? 0,
      this.canvas?.height ?? 0,
    );
  }

  async collectVisualArtifactSample(
    udid: string,
  ): Promise<VisualArtifactSample | null> {
    if (
      !this.video ||
      this.video.readyState < HAVE_CURRENT_DATA ||
      this.video.videoWidth <= 0 ||
      this.video.videoHeight <= 0
    ) {
      return null;
    }
    const response = await fetch(
      new URL(
        apiUrl(`/api/simulators/${encodeURIComponent(udid)}/screenshot.png`),
        window.location.href,
      ),
      { cache: "no-store", headers: apiHeaders() },
    );
    if (!response.ok) {
      return null;
    }
    const source = await createImageBitmap(await response.blob());
    try {
      return compareVideoToImage(this.video, source);
    } finally {
      source.close();
    }
  }

  async connect(target: StreamConnectTarget) {
    const wasReconnecting = this.reconnecting;
    this.reconnecting = false;
    if (wasReconnecting) {
      this.clearReconnectTimeout();
      this.clearDisconnectGraceTimeout();
      this.clearIceRestartTimeout();
      this.clearFrameWatchdog();
      this.closeActiveConnection();
    } else {
      this.disconnect();
    }
    if (!this.canvas) {
      return;
    }
    const canvasElement = this.canvas;
    const generation = ++this.connectGeneration;
    this.shouldReconnect = true;
    this.remoteMode = Boolean(target.remote);
    this.streamTarget = target;
    if (!wasReconnecting) {
      this.reconnectDelayMs = WEBRTC_RECONNECT_BASE_DELAY_MS;
    }
    this.resetFrameStateForNewConnection();
    this.diagnostics = createWebRtcDiagnostics();
    this.reportedVideoHeight = 0;
    this.reportedVideoWidth = 0;
    this.receiverStatsSeen = false;
    this.onMessage({
      type: "status",
      status: { detail: "Creating WebRTC offer", state: "connecting" },
    });

    try {
      const health = await fetchHealth().catch(() => null);
      if (generation !== this.connectGeneration) {
        return;
      }
      const peerConnection = new RTCPeerConnection({
        iceServers: iceServers(health),
        iceTransportPolicy: iceTransportPolicy(health),
      });
      this.peerConnection = peerConnection;
      const useRgbaTransport = shouldUseLocalAndroidRgbaWebRtc(target);
      this.rgbaMode = useRgbaTransport;
      this.attachDiagnostics(peerConnection, target, generation);
      if (!useRgbaTransport) {
        this.startReceiverStatsPolling(peerConnection, target, generation);
        const transceiver = peerConnection.addTransceiver("video", {
          direction: "recvonly",
        });
        configureReceiverCodecPreferences(transceiver);
        configureLowLatencyReceiver(
          transceiver.receiver,
          receiverBufferSeconds(target),
        );
      }
      const controlChannel = peerConnection.createDataChannel(
        WEBRTC_CONTROL_CHANNEL_LABEL,
        {
          ordered: true,
        },
      );
      this.controlChannel = controlChannel;
      activeWebRtcControlChannel = controlChannel;
      controlChannel.addEventListener("close", () => {
        if (activeWebRtcControlChannel === controlChannel) {
          activeWebRtcControlChannel = null;
        }
      });
      const telemetryChannel = peerConnection.createDataChannel(
        WEBRTC_TELEMETRY_CHANNEL_LABEL,
        {
          maxRetransmits: 0,
          ordered: false,
        },
      );
      this.telemetryChannel = telemetryChannel;
      activeWebRtcTelemetryChannel = telemetryChannel;
      telemetryChannel.addEventListener("close", () => {
        if (activeWebRtcTelemetryChannel === telemetryChannel) {
          activeWebRtcTelemetryChannel = null;
        }
      });

      peerConnection.ondatachannel = (event) => {
        if (
          generation !== this.connectGeneration ||
          event.channel.label !== WEBRTC_RGBA_CHANNEL_LABEL
        ) {
          return;
        }
        this.attachRgbaDataChannel(event.channel, generation);
      };

      peerConnection.ontrack = (event) => {
        if (useRgbaTransport) {
          return;
        }
        if (generation !== this.connectGeneration) {
          return;
        }
        event.track.contentHint = "motion";
        for (const receiver of peerConnection.getReceivers()) {
          configureLowLatencyReceiver(receiver, receiverBufferSeconds(target));
        }
        const stream = event.streams[0] ?? new MediaStream([event.track]);
        const video = document.createElement("video");
        video.autoplay = true;
        video.className = "stream-video";
        video.disablePictureInPicture = true;
        video.muted = true;
        video.playsInline = true;
        video.defaultPlaybackRate = 1;
        video.playbackRate = 1;
        video.setAttribute("playsinline", "");
        video.setAttribute("webkit-playsinline", "");
        video.preload = "auto";
        (video as HTMLVideoElement & { latencyHint?: string }).latencyHint =
          "interactive";
        video.srcObject = stream;
        const mountCanvas = this.canvas ?? canvasElement;
        mountCanvas.parentElement?.insertBefore(video, mountCanvas.nextSibling);
        this.video = video;
        const startPlayback = () => {
          if (generation !== this.connectGeneration) {
            return;
          }
          void video.play().catch(() => {
            // The media stream can be detached during reconnect; retry on the next track.
          });
          if (video.videoWidth <= 0 || video.videoHeight <= 0) {
            return;
          }
          this.syncCanvasSize(video.videoWidth, video.videoHeight);
          this.reportVideoConfig(video.videoWidth, video.videoHeight);
          this.scheduleVideoFrame();
        };
        video.addEventListener("loadedmetadata", startPlayback);
        video.addEventListener("loadeddata", startPlayback);
        video.addEventListener("canplay", startPlayback);
        video.addEventListener("resize", startPlayback);
        void video.play().catch(() => {
          // The readiness listeners above retry once the media stream has data.
        });
        this.scheduleVideoFrame();
      };

      peerConnection.onconnectionstatechange = () => {
        this.diagnostics.peerConnectionState = peerConnection.connectionState;
        this.postDiagnostics(target, "connectionstatechange");
        if (generation !== this.connectGeneration) {
          return;
        }
        if (peerConnection.connectionState === "connected") {
          this.clearDisconnectGraceTimeout();
          this.clearIceRestartTimeout();
          this.iceRestartInFlight = false;
          this.reconnectDelayMs = WEBRTC_RECONNECT_BASE_DELAY_MS;
          if (this.reportedVideoWidth > 0 && this.reportedVideoHeight > 0) {
            this.onMessage({
              type: "status",
              status: {
                detail: useRgbaTransport
                  ? "WebRTC RGBA stream connected"
                  : "WebRTC media connected",
                state: "streaming",
              },
            });
          }
          return;
        }
        if (peerConnection.connectionState === "disconnected") {
          if (this.hasRenderedFrame) {
            this.scheduleIceRestart(
              target,
              generation,
              "connection-disconnected",
            );
            this.scheduleDisconnectedGrace(target, generation);
            return;
          }
          this.handleConnectionError(
            target,
            generation,
            new Error("WebRTC connection disconnected."),
            "connection-disconnected-before-first-frame",
          );
          return;
        }
        if (peerConnection.connectionState === "failed") {
          void this.updateSelectedCandidatePair(peerConnection, target);
          void this.restartIceOrReconnect(
            target,
            generation,
            "connection-failed",
          );
        }
      };

      await this.negotiatePeerConnection(peerConnection, target, generation, {
        detailPrefix: "local",
        transport: useRgbaTransport ? "rgba" : undefined,
      });
      this.scheduleFrameWatchdog(target, generation);
    } catch (error) {
      this.handleConnectionError(target, generation, error);
    }
  }

  disconnect() {
    this.shouldReconnect = false;
    this.reconnecting = false;
    this.connectGeneration += 1;
    this.streamTarget = null;
    this.clearReconnectTimeout();
    this.clearDisconnectGraceTimeout();
    this.clearIceRestartTimeout();
    this.clearFrameWatchdog();
    this.closeActiveConnection();
    this.onMessage({ type: "status", status: { state: "idle" } });
  }

  sendControl(payload: unknown): boolean {
    return sendDataChannelMessage(this.controlChannel, JSON.stringify(payload));
  }

  async applyStreamConfig(config?: StreamConfig) {
    if (!config) {
      return;
    }
    const generation = ++this.streamConfigGeneration;
    if (!sendStreamQualityConfig(config)) {
      await postStreamConfigWithAuthRetry(config, { remote: this.remoteMode });
    }
    if (generation !== this.streamConfigGeneration) {
      return;
    }
    this.sendControl({ forceKeyframe: true, type: "streamControl" });
  }

  private async negotiatePeerConnection(
    peerConnection: RTCPeerConnection,
    target: StreamConnectTarget,
    generation: number,
    options: {
      detailPrefix: string;
      iceRestart?: boolean;
      transport?: "rgba";
    },
  ) {
    const offer = safariBaselineH264Offer(
      await peerConnection.createOffer({ iceRestart: options.iceRestart }),
    );
    if (generation !== this.connectGeneration) {
      return;
    }
    await peerConnection.setLocalDescription(offer);
    await waitForIceGathering(peerConnection, {
      resolveOnHostCandidate: !target.remote,
      timeoutMs: target.remote ? 3000 : 250,
    });
    if (generation !== this.connectGeneration) {
      return;
    }
    const localDescription = peerConnection.localDescription;
    if (!localDescription) {
      throw new Error("WebRTC local offer was not created.");
    }
    this.diagnostics.localCandidateSummary = summarizeSdpCandidates(
      localDescription.sdp,
    );
    this.postDiagnostics(target, `${options.detailPrefix}-offer`);
    if (target.remote && !sdpHasCandidateType(localDescription.sdp, "host")) {
      throw new Error(
        "WebRTC gathered no host ICE candidates for this remote browser.",
      );
    }

    const response = await postWebRtcOfferWithAuthRetry(
      target,
      localDescription,
      options.transport,
    );
    const answer = (await response.json()) as WebRtcAnswerPayload;
    if (generation !== this.connectGeneration) {
      return;
    }
    this.diagnostics.remoteCandidateSummary = summarizeSdpCandidates(
      answer.sdp ?? "",
    );
    this.postDiagnostics(target, `${options.detailPrefix}-answer`);
    await peerConnection.setRemoteDescription(answer);
    if (
      typeof answer.video?.width === "number" &&
      typeof answer.video?.height === "number" &&
      answer.video.width > 0 &&
      answer.video.height > 0
    ) {
      this.syncCanvasSize(answer.video.width, answer.video.height);
      this.reportVideoConfig(answer.video.width, answer.video.height);
    }
  }

  destroy() {
    this.disconnect();
  }

  private closeActiveConnection() {
    window.cancelAnimationFrame(this.animationFrame);
    this.animationFrame = 0;
    this.clearFrameWatchdog();
    this.clearDisconnectGraceTimeout();
    this.clearIceRestartTimeout();
    this.iceRestartInFlight = false;
    this.clearReceiverStatsPolling();
    this.cancelVideoFrameCallback();
    this.captureCurrentVideoFrame();
    this.video?.pause();
    if (this.video) {
      this.video.srcObject = null;
      this.video.remove();
    }
    this.video = null;
    this.reportedVideoHeight = 0;
    this.reportedVideoWidth = 0;
    this.controlChannel?.close();
    if (activeWebRtcControlChannel === this.controlChannel) {
      activeWebRtcControlChannel = null;
    }
    this.controlChannel = null;
    this.telemetryChannel?.close();
    if (activeWebRtcTelemetryChannel === this.telemetryChannel) {
      activeWebRtcTelemetryChannel = null;
    }
    this.telemetryChannel = null;
    this.rgbaChannel?.close();
    this.rgbaChannel = null;
    this.rgbaAssemblies.clear();
    this.latestRgbaSequence = -1;
    this.rgbaMode = false;
    this.peerConnection?.close();
    this.peerConnection = null;
  }

  private handleConnectionError(
    target: StreamConnectTarget,
    generation: number,
    error: unknown,
    reason = "connection-error",
  ) {
    if (generation !== this.connectGeneration || !this.shouldReconnect) {
      return;
    }
    const message = error instanceof Error ? error.message : String(error);
    const friendlyMessage = friendlyStreamError(message);
    this.stats.reconnectReason = reason;
    this.closeActiveConnection();
    this.onMessage({
      type: "status",
      status: this.hasRenderedFrame
        ? streamErrorIsServerUnreachable(message)
          ? {
              error: friendlyMessage,
              detail: "Reconnecting in the background.",
              state: "connecting",
            }
          : {
              detail: "Reconnecting in the background.",
              state: "connecting",
            }
        : { error: friendlyMessage, state: "error" },
    });
    this.scheduleReconnect(target, generation, reason);
  }

  private scheduleDisconnectedGrace(
    target: StreamConnectTarget,
    generation: number,
  ) {
    if (this.disconnectGraceTimeout) {
      return;
    }
    this.disconnectGraceTimeout = window.setTimeout(() => {
      this.disconnectGraceTimeout = 0;
      if (generation !== this.connectGeneration || !this.shouldReconnect) {
        return;
      }
      this.handleConnectionError(
        target,
        generation,
        new Error("WebRTC connection disconnected."),
        "connection-disconnected-grace-expired",
      );
    }, disconnectedGraceMs(target));
  }

  private scheduleReconnect(
    target: StreamConnectTarget,
    generation: number,
    reason: string,
  ) {
    if (
      this.reconnectTimeout ||
      generation !== this.connectGeneration ||
      !this.shouldReconnect
    ) {
      return;
    }
    this.stats.reconnects += 1;
    this.stats.reconnectReason = reason;
    this.onMessage({ type: "stats", stats: { ...this.stats } });
    const delayMs = this.reconnectDelayMs;
    this.reconnectDelayMs = Math.min(
      WEBRTC_RECONNECT_MAX_DELAY_MS,
      Math.round(this.reconnectDelayMs * 1.6),
    );
    this.reconnectTimeout = window.setTimeout(() => {
      this.reconnectTimeout = 0;
      if (generation === this.connectGeneration && this.shouldReconnect) {
        this.reconnecting = true;
        void this.connect(target);
      }
    }, delayMs);
  }

  private resetFrameStateForNewConnection() {
    const iceRestartReason = this.stats.iceRestartReason;
    const iceRestarts = this.stats.iceRestarts;
    const reconnectReason = this.stats.reconnectReason;
    const reconnects = this.stats.reconnects;
    this.hasRenderedFrame = false;
    this.lastVideoFrameAt = 0;
    this.stats = createEmptyStreamStats();
    this.stats.iceRestartReason = iceRestartReason;
    this.stats.iceRestarts = iceRestarts;
    this.stats.reconnectReason = reconnectReason;
    this.stats.reconnects = reconnects;
    this.onMessage({ type: "stats", stats: { ...this.stats } });
  }

  private scheduleFrameWatchdog(
    target: StreamConnectTarget,
    generation: number,
  ) {
    this.clearFrameWatchdog();
    this.frameWatchdogTimeout = window.setTimeout(
      () => {
        this.frameWatchdogTimeout = 0;
        if (generation !== this.connectGeneration || !this.shouldReconnect) {
          return;
        }
        const now = performance.now();
        const hasMediaProgress =
          this.hasRenderedFrame ||
          this.stats.renderedFrames > 0 ||
          this.stats.decodedFrames > 0 ||
          this.stats.receivedPackets > 0;
        const frameAgeMs =
          this.lastVideoFrameAt > 0 ? now - this.lastVideoFrameAt : Infinity;
        if (!hasMediaProgress) {
          this.handleConnectionError(
            target,
            generation,
            new Error("WebRTC video stalled before rendering fresh frames."),
            "first-frame-timeout",
          );
          return;
        }
        if (!this.hasRenderedFrame) {
          this.scheduleFrameWatchdog(target, generation);
          return;
        }
        if (frameAgeMs > WEBRTC_STALLED_FRAME_TIMEOUT_MS) {
          this.sendControl({ snapshot: true, type: "streamControl" });
          this.scheduleFrameWatchdog(target, generation);
          return;
        }
        this.scheduleFrameWatchdog(target, generation);
      },
      this.stats.renderedFrames > 0
        ? WEBRTC_STALLED_FRAME_TIMEOUT_MS
        : WEBRTC_FIRST_FRAME_TIMEOUT_MS,
    );
  }

  private clearFrameWatchdog() {
    if (!this.frameWatchdogTimeout) {
      return;
    }
    window.clearTimeout(this.frameWatchdogTimeout);
    this.frameWatchdogTimeout = 0;
  }

  private clearReconnectTimeout() {
    if (!this.reconnectTimeout) {
      return;
    }
    window.clearTimeout(this.reconnectTimeout);
    this.reconnectTimeout = 0;
  }

  private clearDisconnectGraceTimeout() {
    if (!this.disconnectGraceTimeout) {
      return;
    }
    window.clearTimeout(this.disconnectGraceTimeout);
    this.disconnectGraceTimeout = 0;
  }

  private clearIceRestartTimeout() {
    if (!this.iceRestartTimeout) {
      return;
    }
    window.clearTimeout(this.iceRestartTimeout);
    this.iceRestartTimeout = 0;
  }

  private scheduleIceRestart(
    target: StreamConnectTarget,
    generation: number,
    reason: string,
  ) {
    if (
      !target.remote ||
      this.iceRestartTimeout ||
      this.iceRestartInFlight ||
      generation !== this.connectGeneration ||
      !this.shouldReconnect
    ) {
      return;
    }
    this.iceRestartTimeout = window.setTimeout(() => {
      this.iceRestartTimeout = 0;
      void this.restartIceOrReconnect(target, generation, reason);
    }, WEBRTC_REMOTE_ICE_RESTART_GRACE_MS);
  }

  private async restartIceOrReconnect(
    target: StreamConnectTarget,
    generation: number,
    reason: string,
  ) {
    if (generation !== this.connectGeneration || !this.shouldReconnect) {
      return;
    }
    if (!target.remote) {
      this.handleConnectionError(
        target,
        generation,
        new Error("WebRTC connection failed."),
        reason,
      );
      return;
    }
    if (this.iceRestartInFlight) {
      return;
    }
    const restarted = await this.tryIceRestart(target, generation, reason);
    if (
      !restarted &&
      generation === this.connectGeneration &&
      this.shouldReconnect
    ) {
      this.handleConnectionError(
        target,
        generation,
        new Error("WebRTC ICE restart failed."),
        `${reason}-ice-restart-failed`,
      );
    }
  }

  private async tryIceRestart(
    target: StreamConnectTarget,
    generation: number,
    reason: string,
  ): Promise<boolean> {
    const peerConnection = this.peerConnection;
    if (
      !peerConnection ||
      peerConnection.connectionState === "closed" ||
      peerConnection.signalingState !== "stable" ||
      this.iceRestartInFlight ||
      generation !== this.connectGeneration ||
      !this.shouldReconnect
    ) {
      return false;
    }
    this.iceRestartInFlight = true;
    this.stats.iceRestartReason = reason;
    this.stats.iceRestarts += 1;
    this.onMessage({ type: "stats", stats: { ...this.stats } });
    this.postDiagnostics(target, "ice-restart-start");
    try {
      await this.negotiatePeerConnection(peerConnection, target, generation, {
        detailPrefix: "ice-restart",
        iceRestart: true,
      });
      if (generation !== this.connectGeneration || !this.shouldReconnect) {
        return true;
      }
      this.iceRestartInFlight = false;
      this.scheduleFrameWatchdog(target, generation);
      this.postDiagnostics(target, "ice-restart-complete");
      return true;
    } catch (error) {
      this.iceRestartInFlight = false;
      this.diagnostics.selectedCandidatePair = `ice-restart-error:${error instanceof Error ? error.message : String(error)}`;
      this.postDiagnostics(target, "ice-restart-error");
      return false;
    }
  }

  private attachDiagnostics(
    peerConnection: RTCPeerConnection,
    target: StreamConnectTarget,
    generation: number,
  ) {
    peerConnection.onicecandidate = (event) => {
      if (generation !== this.connectGeneration) {
        return;
      }
      if (event.candidate) {
        this.diagnostics.localCandidateSummary = summarizeCandidateLines([
          ...(this.diagnostics.localCandidateLines ?? []),
          event.candidate.candidate,
        ]);
        this.diagnostics.localCandidateLines = [
          ...(this.diagnostics.localCandidateLines ?? []),
          event.candidate.candidate,
        ];
      }
      this.postDiagnostics(
        target,
        event.candidate ? "local-candidate" : "local-candidates-complete",
      );
    };
    peerConnection.oniceconnectionstatechange = () => {
      if (generation !== this.connectGeneration) {
        return;
      }
      this.diagnostics.iceConnectionState = peerConnection.iceConnectionState;
      this.postDiagnostics(target, "iceconnectionstatechange");
      if (
        peerConnection.iceConnectionState === "connected" ||
        peerConnection.iceConnectionState === "completed" ||
        peerConnection.iceConnectionState === "failed"
      ) {
        void this.updateSelectedCandidatePair(peerConnection, target);
      }
    };
    peerConnection.onicegatheringstatechange = () => {
      if (generation !== this.connectGeneration) {
        return;
      }
      this.diagnostics.iceGatheringState = peerConnection.iceGatheringState;
      this.postDiagnostics(target, "icegatheringstatechange");
    };
    peerConnection.onsignalingstatechange = () => {
      if (generation !== this.connectGeneration) {
        return;
      }
      this.diagnostics.signalingState = peerConnection.signalingState;
      this.postDiagnostics(target, "signalingstatechange");
    };
  }

  private async updateSelectedCandidatePair(
    peerConnection: RTCPeerConnection,
    target: StreamConnectTarget,
  ) {
    try {
      const stats = await peerConnection.getStats();
      let selectedPair: RTCStats | undefined;
      stats.forEach((report) => {
        const pair = report as RTCStats & {
          nominated?: boolean;
          selected?: boolean;
          state?: string;
          localCandidateId?: string;
          remoteCandidateId?: string;
        };
        if (
          report.type === "candidate-pair" &&
          (pair.selected || pair.nominated || pair.state === "succeeded")
        ) {
          selectedPair = report;
        }
      });
      if (!selectedPair) {
        this.diagnostics.selectedCandidatePair = "none";
        this.postDiagnostics(target, "candidate-pair-none");
        return;
      }
      const pair = selectedPair as RTCStats & {
        localCandidateId?: string;
        remoteCandidateId?: string;
        state?: string;
        currentRoundTripTime?: number;
      };
      const local = pair.localCandidateId
        ? stats.get(pair.localCandidateId)
        : undefined;
      const remote = pair.remoteCandidateId
        ? stats.get(pair.remoteCandidateId)
        : undefined;
      this.diagnostics.selectedCandidatePair = `state=${pair.state ?? "?"},rtt=${pair.currentRoundTripTime ?? "?"},local=${candidateStatsSummary(local)},remote=${candidateStatsSummary(remote)}`;
      this.postDiagnostics(target, "candidate-pair-selected");
    } catch (error) {
      this.diagnostics.selectedCandidatePair = `stats-error:${error instanceof Error ? error.message : String(error)}`;
      this.postDiagnostics(target, "candidate-pair-error");
    }
  }

  private postDiagnostics(target: StreamConnectTarget, detail: string) {
    const payload = {
      ...this.stats,
      clientId: target.clientId ?? "webrtc-page",
      connectionId: this.connectGeneration,
      detail,
      iceConnectionState: this.diagnostics.iceConnectionState,
      iceGatheringState: this.diagnostics.iceGatheringState,
      kind: "webrtc",
      localCandidateSummary: this.diagnostics.localCandidateSummary,
      peerConnectionState: this.diagnostics.peerConnectionState,
      remoteCandidateSummary: this.diagnostics.remoteCandidateSummary,
      selectedCandidatePair: this.diagnostics.selectedCandidatePair,
      signalingState: this.diagnostics.signalingState,
      status:
        this.diagnostics.peerConnectionState ||
        this.diagnostics.iceConnectionState,
      timestampMs: Date.now(),
      udid: target.udid,
      url: window.location.href,
      userAgent: window.navigator.userAgent,
    };
    if (sendStreamClientStats(payload) || this.remoteMode) {
      return;
    }
  }

  private attachRgbaDataChannel(channel: RTCDataChannel, generation: number) {
    this.rgbaChannel?.close();
    this.rgbaChannel = channel;
    channel.binaryType = "arraybuffer";
    channel.addEventListener("message", (event) => {
      if (generation !== this.connectGeneration || channel !== this.rgbaChannel) {
        return;
      }
      if (hasArrayBufferMethod(event.data)) {
        void event.data.arrayBuffer().then((buffer) => {
          if (generation === this.connectGeneration && channel === this.rgbaChannel) {
            this.handleRgbaMessage(buffer);
          }
        });
        return;
      }
      this.handleRgbaMessage(event.data);
    });
    channel.addEventListener("close", () => {
      if (this.rgbaChannel === channel) {
        this.rgbaChannel = null;
      }
    });
  }

  private handleRgbaMessage(data: unknown) {
    const chunk = parseWebRtcRgbaChunk(data);
    if (!chunk) {
      this.stats.h264ParseFailures += 1;
      this.onMessage({ type: "stats", stats: { ...this.stats } });
      return;
    }
    this.stats.receivedPackets += 1;
    if (chunk.sequence < this.latestRgbaSequence) {
      this.stats.droppedFrames += 1;
      this.onMessage({ type: "stats", stats: { ...this.stats } });
      return;
    }
    const frame = this.appendRgbaChunk(chunk);
    if (!frame) {
      return;
    }
    this.stats.decodedFrames += 1;
    this.stats.width = frame.width;
    this.stats.height = frame.height;
    this.stats.frameSequence = frame.sequence;
    this.stats.codec = "webrtc-rgba";
    this.latestRgbaSequence = frame.sequence;
    this.paintRgbaFrame(frame);
  }

  private appendRgbaChunk(chunk: WebRtcRgbaChunk): WebRtcRgbaFrame | null {
    let assembly = this.rgbaAssemblies.get(chunk.sequence);
    if (
      assembly &&
      (assembly.width !== chunk.width ||
        assembly.height !== chunk.height ||
        assembly.buffer.byteLength !== chunk.payloadBytes)
    ) {
      this.rgbaAssemblies.delete(chunk.sequence);
      assembly = undefined;
    }
    if (!assembly) {
      assembly = {
        buffer: new Uint8Array(chunk.payloadBytes),
        height: chunk.height,
        receivedBytes: 0,
        receivedRanges: [],
        sequence: chunk.sequence,
        timestampUs: chunk.timestampUs,
        width: chunk.width,
      };
      this.rgbaAssemblies.set(chunk.sequence, assembly);
      this.trimRgbaAssemblies(chunk.sequence);
    }
    assembly.buffer.set(chunk.payload, chunk.chunkOffset);
    const chunkEnd = chunk.chunkOffset + chunk.payload.byteLength;
    if (!rangeAlreadyReceived(assembly.receivedRanges, chunk.chunkOffset, chunkEnd)) {
      assembly.receivedRanges.push([chunk.chunkOffset, chunkEnd]);
      assembly.receivedBytes += chunk.payload.byteLength;
    }
    if (assembly.receivedBytes < assembly.buffer.byteLength) {
      return null;
    }
    this.rgbaAssemblies.delete(chunk.sequence);
    return {
      height: assembly.height,
      payload: assembly.buffer,
      sequence: assembly.sequence,
      timestampUs: assembly.timestampUs,
      width: assembly.width,
    };
  }

  private trimRgbaAssemblies(latestSequence: number) {
    while (this.rgbaAssemblies.size > 3) {
      const firstKey = this.rgbaAssemblies.keys().next().value;
      if (typeof firstKey !== "number") {
        break;
      }
      this.rgbaAssemblies.delete(firstKey);
    }
    for (const sequence of this.rgbaAssemblies.keys()) {
      if (latestSequence - sequence > 2) {
        this.rgbaAssemblies.delete(sequence);
      }
    }
  }

  private paintRgbaFrame(frame: WebRtcRgbaFrame) {
    const canvas = this.canvas;
    const context = this.ensureCanvasContext();
    if (!canvas || !context) {
      return;
    }
    this.syncCanvasSize(frame.width, frame.height);
    const startedAt = performance.now();
    const rgba = new Uint8ClampedArray(
      frame.payload.buffer as ArrayBuffer,
      frame.payload.byteOffset,
      frame.payload.byteLength,
    );
    try {
      context.putImageData(new ImageData(rgba, frame.width, frame.height), 0, 0);
    } catch {
      this.stats.droppedFrames += 1;
      this.onMessage({ type: "stats", stats: { ...this.stats } });
      return;
    }
    const finishedAt = performance.now();
    const previousFrameAt = this.lastVideoFrameAt;
    this.lastVideoFrameAt = finishedAt;
    this.hasRenderedFrame = true;
    this.stats.renderedFrames += 1;
    this.stats.latestRenderMs = finishedAt - startedAt;
    this.stats.maxRenderMs = Math.max(
      this.stats.maxRenderMs,
      this.stats.latestRenderMs,
    );
    this.stats.averageRenderMs =
      this.stats.averageRenderMs <= 0
        ? this.stats.latestRenderMs
        : this.stats.averageRenderMs * 0.9 + this.stats.latestRenderMs * 0.1;
    this.stats.latestFrameGapMs =
      previousFrameAt > 0 ? finishedAt - previousFrameAt : 0;
    this.reportVideoConfig(frame.width, frame.height);
    this.onMessage({ type: "stats", stats: { ...this.stats } });
  }

  private drawVideoFrame = () => {
    this.videoFrameCallback = 0;
    if (!this.canvas || !this.video) {
      return;
    }
    if (
      this.video.readyState >= HAVE_CURRENT_DATA &&
      this.video.videoWidth > 0 &&
      this.video.videoHeight > 0
    ) {
      this.syncCanvasSize(this.video.videoWidth, this.video.videoHeight);
      this.reportVideoConfig(this.video.videoWidth, this.video.videoHeight);
      const now = performance.now();
      if (!this.receiverStatsSeen) {
        this.stats.decodedFrames += 1;
        this.stats.receivedPackets += 1;
      }
      this.stats.renderedFrames += 1;
      this.stats.width = this.canvas.width;
      this.stats.height = this.canvas.height;
      this.stats.codec = "webrtc";
      this.hasRenderedFrame = true;
      this.stats.latestRenderMs = 0;
      if (this.lastVideoFrameAt > 0) {
        this.stats.latestFrameGapMs = now - this.lastVideoFrameAt;
      }
      this.lastVideoFrameAt = now;
      this.onMessage({ type: "stats", stats: { ...this.stats } });
    }
    this.scheduleVideoFrame();
  };

  private scheduleVideoFrame() {
    this.cancelVideoFrameCallback();
    if (!this.video) {
      return;
    }
    const video = this.video as HTMLVideoElement & {
      requestVideoFrameCallback?: (callback: () => void) => number;
    };
    if (video.requestVideoFrameCallback) {
      this.videoFrameCallback = video.requestVideoFrameCallback(
        this.drawVideoFrame,
      );
      return;
    }
    window.cancelAnimationFrame(this.animationFrame);
    this.animationFrame = window.requestAnimationFrame(this.drawVideoFrame);
  }

  private reportVideoConfig(width: number, height: number) {
    if (
      this.reportedVideoWidth === width &&
      this.reportedVideoHeight === height
    ) {
      return;
    }
    this.reportedVideoWidth = width;
    this.reportedVideoHeight = height;
    this.onMessage({
      type: "video-config",
      size: { height, width },
    });
    this.onMessage({
      type: "status",
      status: {
        detail: this.rgbaMode
          ? "WebRTC RGBA stream connected"
          : "WebRTC media connected",
        state: "streaming",
      },
    });
  }

  private startReceiverStatsPolling(
    peerConnection: RTCPeerConnection,
    target: StreamConnectTarget,
    generation: number,
  ) {
    this.clearReceiverStatsPolling();
    const poll = () => {
      if (
        generation !== this.connectGeneration ||
        this.peerConnection !== peerConnection
      ) {
        return;
      }
      void this.updateReceiverStats(peerConnection, target);
    };
    poll();
    this.receiverStatsInterval = window.setInterval(poll, 1000);
  }

  private clearReceiverStatsPolling() {
    if (this.receiverStatsInterval) {
      window.clearInterval(this.receiverStatsInterval);
      this.receiverStatsInterval = 0;
    }
  }

  private async updateReceiverStats(
    peerConnection: RTCPeerConnection,
    target: StreamConnectTarget,
  ) {
    try {
      const reports = await peerConnection.getStats();
      let inbound: RTCStats | undefined;
      let codec: RTCStats | undefined;
      reports.forEach((report) => {
        const typed = report as RTCStats & {
          kind?: string;
          mediaType?: string;
          codecId?: string;
        };
        if (
          report.type === "inbound-rtp" &&
          (typed.kind === "video" || typed.mediaType === "video")
        ) {
          inbound = report;
          codec = typed.codecId ? reports.get(typed.codecId) : codec;
        }
      });
      if (!inbound) {
        return;
      }
      const video = inbound as RTCStats & {
        framesDecoded?: number;
        framesDropped?: number;
        packetsLost?: number;
        packetsReceived?: number;
      };
      const playbackQuality = this.video?.getVideoPlaybackQuality?.();
      this.receiverStatsSeen = true;
      if (typeof video.framesDecoded === "number") {
        this.stats.decodedFrames = video.framesDecoded;
      }
      if (typeof playbackQuality?.totalVideoFrames === "number") {
        this.stats.renderedFrames = playbackQuality.totalVideoFrames;
      }
      if (typeof playbackQuality?.droppedVideoFrames === "number") {
        this.stats.presentationDroppedFrames =
          playbackQuality.droppedVideoFrames;
      }
      if (typeof video.packetsReceived === "number") {
        this.stats.receivedPackets = video.packetsReceived;
      }
      if (typeof video.framesDropped === "number") {
        this.stats.decoderDroppedFrames = video.framesDropped;
        this.stats.droppedFrames = Math.max(
          this.stats.droppedFrames,
          video.framesDropped,
        );
      }
      if (typeof video.packetsLost === "number") {
        this.stats.packetsLost = Math.max(0, video.packetsLost);
      }
      const codecStats = codec as
        | (RTCStats & { mimeType?: string; payloadType?: number })
        | undefined;
      if (codecStats?.mimeType) {
        this.stats.codec = codecStats.payloadType
          ? `${codecStats.mimeType}/${codecStats.payloadType}`
          : codecStats.mimeType;
      } else if (!this.stats.codec) {
        this.stats.codec = "webrtc";
      }
      this.onMessage({ type: "stats", stats: { ...this.stats } });
      this.postDiagnostics(target, "receiver-stats");
    } catch {
      // Receiver stats are diagnostics only; drawing should continue.
    }
  }

  private cancelVideoFrameCallback() {
    if (!this.videoFrameCallback || !this.video) {
      return;
    }
    const video = this.video as HTMLVideoElement & {
      cancelVideoFrameCallback?: (handle: number) => void;
    };
    video.cancelVideoFrameCallback?.(this.videoFrameCallback);
    this.videoFrameCallback = 0;
  }

  private ensureCanvasContext(): CanvasRenderingContext2D | null {
    const canvas = this.canvas;
    if (!canvas) {
      this.canvasContext = null;
      return null;
    }
    if (this.canvasContext?.canvas === canvas) {
      return this.canvasContext;
    }
    this.canvasContext = canvas.getContext("2d", {
      alpha: false,
      desynchronized: true,
    });
    return this.canvasContext;
  }

  private syncCanvasSize(width: number, height: number) {
    if (!this.canvas) {
      return;
    }
    const nextWidth = Math.max(1, Math.round(width));
    const nextHeight = Math.max(1, Math.round(height));
    if (this.canvas.width !== nextWidth) {
      this.canvas.width = nextWidth;
    }
    if (this.canvas.height !== nextHeight) {
      this.canvas.height = nextHeight;
    }
  }

  private captureCurrentVideoFrame() {
    if (
      !this.canvas ||
      !this.video ||
      this.video.readyState < HAVE_CURRENT_DATA ||
      this.video.videoWidth <= 0 ||
      this.video.videoHeight <= 0
    ) {
      return;
    }
    this.syncCanvasSize(this.video.videoWidth, this.video.videoHeight);
    this.canvas
      .getContext("2d")
      ?.drawImage(this.video, 0, 0, this.canvas.width, this.canvas.height);
  }
}

function friendlyStreamError(message: string): string {
  if (streamErrorIsServerUnreachable(message)) {
    return "SimDeck server is unreachable.";
  }
  return message.replace(/\.$/, "");
}

function streamErrorIsServerUnreachable(message: string): boolean {
  const normalized = message.trim().toLowerCase();
  return (
    normalized === "failed to fetch" ||
    normalized === "load failed" ||
    normalized.includes("networkerror")
  );
}

async function postWebRtcOfferWithAuthRetry(
  target: StreamConnectTarget,
  localDescription: RTCSessionDescription,
  transport?: "rgba",
): Promise<Response> {
  const response = await postWebRtcOffer(target, localDescription, transport);
  if (response.status !== 401) {
    if (!response.ok) {
      throw new Error(await response.text());
    }
    return response;
  }
  if (target.remote) {
    throw new Error(await response.text());
  }
  await fetchHealth();
  const retry = await postWebRtcOffer(target, localDescription, transport);
  if (!retry.ok) {
    throw new Error(await retry.text());
  }
  return retry;
}

async function postStreamConfigWithAuthRetry(
  config: StreamConfig | undefined,
  options: { remote?: boolean } = {},
): Promise<void> {
  if (!config) {
    return;
  }
  const response = await postStreamConfig(config);
  if (response.status !== 401) {
    if (!response.ok) {
      throw new Error(await response.text());
    }
    return;
  }
  if (options.remote) {
    throw new Error(await response.text());
  }
  await fetchHealth();
  const retry = await postStreamConfig(config);
  if (!retry.ok) {
    throw new Error(await retry.text());
  }
}

function postStreamConfig(config: StreamConfig): Promise<Response> {
  return fetch(apiUrl("/api/stream-quality"), {
    body: JSON.stringify(streamQualityPayload(config)),
    headers: apiHeaders(),
    method: "POST",
  });
}

function streamQualityPayload(config: StreamConfig): {
  fps: number;
  profile: StreamQualityPreset;
  videoCodec: string;
} {
  return {
    fps: config.fps,
    profile: config.quality === "auto" ? "economy" : config.quality,
    videoCodec: config.encoder,
  };
}

function streamQualityQuery(config: StreamConfig | undefined): string {
  if (!config) {
    return "";
  }
  const params = new URLSearchParams();
  const payload = streamQualityPayload(config);
  params.set("fps", String(payload.fps));
  params.set("profile", payload.profile);
  params.set("videoCodec", payload.videoCodec);
  return `?${params.toString()}`;
}

function postWebRtcOffer(
  target: StreamConnectTarget,
  localDescription: RTCSessionDescription,
  transport?: "rgba",
): Promise<Response> {
  return fetch(
    apiUrl(`/api/simulators/${encodeURIComponent(target.udid)}/webrtc/offer`),
    {
      body: JSON.stringify({
        clientId: target.clientId,
        sdp: localDescription.sdp,
        streamConfig: target.streamConfig
          ? streamQualityPayload(target.streamConfig)
          : undefined,
        transport,
        type: localDescription.type,
      }),
      headers: apiHeaders(),
      method: "POST",
    },
  );
}

function h264WebSocketStreamConfig(
  config: StreamConfig | undefined,
  autoProfile: StreamQualityPreset,
): StreamConfig | undefined {
  if (!config) {
    return config;
  }
  if (config.quality === "auto") {
    return {
      ...config,
      fps: Math.min(config.fps || 60, 60),
      maxEdge: undefined,
      quality: autoProfile,
    };
  }
  if (config.quality !== "quality") {
    return config;
  }
  return config;
}

function h264AutoProfiles(
  target: StreamConnectTarget | null,
): StreamQualityPreset[] {
  return shouldUseRemoteH264AutoProfile(target)
    ? H264_WS_REMOTE_AUTO_PROFILES
    : H264_WS_LOCAL_AUTO_PROFILES;
}

function initialH264AutoProfile(
  target: StreamConnectTarget | null,
): StreamQualityPreset {
  return shouldUseRemoteH264AutoProfile(target)
    ? H264_WS_REMOTE_INITIAL_AUTO_PROFILE
    : H264_WS_LOCAL_INITIAL_AUTO_PROFILE;
}

function shouldUseRemoteH264AutoProfile(
  target: StreamConnectTarget | null,
): boolean {
  return Boolean(target?.remote) || !isLoopbackHost(window.location.hostname);
}

function shouldUseLocalAndroidRgbaWebRtc(target: StreamConnectTarget): boolean {
  const stream = new URLSearchParams(window.location.search).get("stream");
  return (
    !target.remote &&
    isLoopbackHost(window.location.hostname) &&
    (stream === "rgba" || stream === "webrtc-rgba") &&
    (target.platform === "android-emulator" || target.udid.startsWith("android:"))
  );
}

function configureLowLatencyReceiver(
  receiver: RTCRtpReceiver,
  bufferSeconds: number | null,
) {
  if (!bufferSeconds || bufferSeconds <= 0) {
    return;
  }
  const lowLatencyReceiver = receiver as RTCRtpReceiver & {
    jitterBufferTarget?: number;
    playoutDelayHint?: number;
  };
  if ("jitterBufferTarget" in lowLatencyReceiver) {
    lowLatencyReceiver.jitterBufferTarget = bufferSeconds;
  }
  if ("playoutDelayHint" in lowLatencyReceiver) {
    lowLatencyReceiver.playoutDelayHint = bufferSeconds;
  }
}

function receiverBufferSeconds(target: StreamConnectTarget): number | null {
  return target.remote
    ? WEBRTC_REMOTE_RECEIVER_BUFFER_SECONDS
    : WEBRTC_LOCAL_RECEIVER_BUFFER_SECONDS;
}

function disconnectedGraceMs(target: StreamConnectTarget): number {
  return target.remote
    ? WEBRTC_REMOTE_DISCONNECTED_GRACE_MS
    : WEBRTC_LOCAL_DISCONNECTED_GRACE_MS;
}

function configureReceiverCodecPreferences(transceiver: RTCRtpTransceiver) {
  if (!transceiver.setCodecPreferences) {
    return;
  }
  const capabilities = RTCRtpReceiver.getCapabilities("video");
  const codecs = capabilities?.codecs ?? [];
  const preferred = codecs.filter(
    (codec) => codec.mimeType.toLowerCase() === "video/h264",
  );
  if (preferred.length === 0) {
    return;
  }
  transceiver.setCodecPreferences([
    ...preferred,
    ...codecs.filter((codec) => codec.mimeType.toLowerCase() !== "video/h264"),
  ]);
}

function safariBaselineH264Offer(
  offer: RTCSessionDescriptionInit,
): RTCSessionDescriptionInit {
  if (!isSafariBrowser() || !offer.sdp) {
    return offer;
  }
  return {
    ...offer,
    sdp: offer.sdp.replace(
      /(a=fmtp:\d+ .*profile-level-id=)[0-9a-fA-F]{6}/g,
      "$142e01f",
    ),
  };
}

function isSafariBrowser(): boolean {
  const ua = navigator.userAgent;
  return /Safari\//.test(ua) && !/Chrome\/|Chromium\/|CriOS\/|FxiOS\//.test(ua);
}

function iceServers(health?: HealthResponse | null): RTCIceServer[] {
  const params = new URLSearchParams(window.location.search);
  const queryValue = params.get("iceServers");
  const raw = queryValue ?? "";
  if (raw === "none") {
    return [];
  }
  if (raw.trim()) {
    const server: RTCIceServer = {
      urls: raw
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean),
    };
    const username = params.get("iceUsername");
    const credential = params.get("iceCredential");
    if (username) {
      server.username = username;
    }
    if (credential) {
      server.credential = credential;
    }
    return [server];
  }
  if (health?.webRtc?.iceServers?.length) {
    return health.webRtc.iceServers;
  }
  return [{ urls: ["stun:stun.l.google.com:19302"] }];
}

function iceTransportPolicy(
  health?: HealthResponse | null,
): RTCIceTransportPolicy {
  const value = new URLSearchParams(window.location.search).get(
    "iceTransportPolicy",
  );
  if (value === "relay" || value === "all") {
    return value;
  }
  const healthValue = health?.webRtc?.iceTransportPolicy;
  return healthValue === "relay" || healthValue === "all" ? healthValue : "all";
}

interface WebRtcDiagnostics {
  iceConnectionState: string;
  iceGatheringState: string;
  localCandidateLines?: string[];
  localCandidateSummary: string;
  peerConnectionState: string;
  remoteCandidateSummary: string;
  selectedCandidatePair: string;
  signalingState: string;
}

function createWebRtcDiagnostics(): WebRtcDiagnostics {
  return {
    iceConnectionState: "",
    iceGatheringState: "",
    localCandidateSummary: "",
    peerConnectionState: "",
    remoteCandidateSummary: "",
    selectedCandidatePair: "",
    signalingState: "",
  };
}

function summarizeSdpCandidates(sdp: string): string {
  return summarizeCandidateLines(
    sdp
      .split(/\r?\n/)
      .filter((line) => line.startsWith("a=candidate:"))
      .map((line) => line.slice("a=".length)),
  );
}

function sdpHasCandidateType(sdp: string, candidateType: string): boolean {
  return sdp
    .split(/\r?\n/)
    .filter((line) => line.startsWith("a=candidate:"))
    .some((line) =>
      candidateLineHasType(line.slice("a=".length), candidateType),
    );
}

function candidateLineHasType(line: string, candidateType: string): boolean {
  const parts = line.split(/\s+/);
  const typIndex = parts.indexOf("typ");
  return typIndex >= 0 && parts[typIndex + 1] === candidateType;
}

function summarizeCandidateLines(lines: string[]): string {
  const counts: Record<string, number> = {
    host: 0,
    prflx: 0,
    relay: 0,
    srflx: 0,
    tcp: 0,
    udp: 0,
    other: 0,
  };
  for (const line of lines) {
    const parts = line.split(/\s+/);
    const typIndex = parts.indexOf("typ");
    const typ = typIndex >= 0 ? parts[typIndex + 1] : "";
    if (typ && typ in counts) {
      counts[typ] += 1;
    } else {
      counts.other += 1;
    }
    const protocol = parts[2]?.toLowerCase();
    if (protocol === "udp" || protocol === "tcp") {
      counts[protocol] += 1;
    }
  }
  return `host=${counts.host},srflx=${counts.srflx},prflx=${counts.prflx},relay=${counts.relay},udp=${counts.udp},tcp=${counts.tcp},other=${counts.other}`;
}

function candidateStatsSummary(candidate: RTCStats | undefined): string {
  if (!candidate) {
    return "none";
  }
  const stats = candidate as RTCStats & {
    address?: string;
    candidateType?: string;
    ip?: string;
    port?: number;
    protocol?: string;
  };
  return `${stats.candidateType ?? "?"}/${stats.protocol ?? "?"}/${stats.address || stats.ip ? "addr" : "noaddr"}/${stats.port ?? "?"}`;
}

function waitForIceGathering(
  peerConnection: RTCPeerConnection,
  options: { resolveOnHostCandidate?: boolean; timeoutMs?: number } = {},
) {
  if (
    peerConnection.iceGatheringState === "complete" ||
    (options.resolveOnHostCandidate &&
      sdpHasCandidateType(peerConnection.localDescription?.sdp ?? "", "host"))
  ) {
    return Promise.resolve();
  }
  return new Promise<void>((resolve) => {
    let resolved = false;
    const finish = () => {
      if (resolved) {
        return;
      }
      resolved = true;
      window.clearTimeout(timeout);
      peerConnection.removeEventListener(
        "icegatheringstatechange",
        handleGatheringStateChange,
      );
      peerConnection.removeEventListener("icecandidate", handleIceCandidate);
      resolve();
    };
    const handleGatheringStateChange = () => {
      if (peerConnection.iceGatheringState === "complete") {
        finish();
      }
    };
    const handleIceCandidate = (event: RTCPeerConnectionIceEvent) => {
      if (
        options.resolveOnHostCandidate &&
        candidateLineHasType(event.candidate?.candidate ?? "", "host")
      ) {
        finish();
      }
    };
    const timeout = window.setTimeout(finish, options.timeoutMs ?? 3000);
    peerConnection.addEventListener(
      "icegatheringstatechange",
      handleGatheringStateChange,
    );
    peerConnection.addEventListener("icecandidate", handleIceCandidate);
  });
}

export class StreamWorkerClient {
  private readonly onMessage: (message: WorkerToMainMessage) => void;
  private backend: StreamClientBackend | null = null;
  private attachedCanvas = false;
  private backendKind: StreamBackend | null = null;
  private canvasElement: HTMLCanvasElement | null = null;
  private disposed = false;
  private target: StreamConnectTarget | null = null;
  private readonly destroyOnPageExit = () => {
    this.destroy();
  };

  constructor(onMessage: (message: WorkerToMainMessage) => void) {
    this.onMessage = onMessage;
    activeStreamClient?.destroy();
    activeStreamClient = this;
    window.addEventListener("pagehide", this.destroyOnPageExit);
    window.addEventListener("beforeunload", this.destroyOnPageExit);
  }

  attachCanvas(canvasElement: HTMLCanvasElement) {
    this.canvasElement = canvasElement;
    this.attachedCanvas = true;
    this.backend?.attachCanvas(canvasElement);
  }

  connect(target: StreamConnectTarget) {
    try {
      this.target = target;
      const backendKind = initialStreamBackend(target);
      this.setBackend(backendKind);
      const result = this.backend?.connect(target);
      if (result && typeof result.catch === "function") {
        result.catch((error: unknown) => {
          this.onMessage({
            type: "status",
            status: {
              error: error instanceof Error ? error.message : String(error),
              state: "error",
            },
          });
        });
      }
    } catch (error) {
      this.onMessage({
        type: "status",
        status: {
          error: error instanceof Error ? error.message : String(error),
          state: "error",
        },
      });
    }
  }

  disconnect() {
    this.backend?.disconnect();
    this.target = null;
  }

  clear() {
    this.backend?.clear();
  }

  async collectVisualArtifactSample(udid: string) {
    return (await this.backend?.collectVisualArtifactSample?.(udid)) ?? null;
  }

  sendStreamControl(options: { forceKeyframe?: boolean; snapshot?: boolean }) {
    return Boolean(
      this.backend?.sendControl?.({ ...options, type: "streamControl" }),
    );
  }

  applyStreamConfig(config?: StreamConfig) {
    try {
      const result = this.backend?.applyStreamConfig?.(config);
      if (result && typeof result.catch === "function") {
        result.catch((error: unknown) => {
          console.warn("Failed to apply stream configuration.", error);
        });
      }
    } catch (error) {
      console.warn("Failed to apply stream configuration.", error);
    }
  }

  destroy() {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    window.removeEventListener("pagehide", this.destroyOnPageExit);
    window.removeEventListener("beforeunload", this.destroyOnPageExit);
    this.backend?.destroy();
    this.backend = null;
    if (activeStreamClient === this) {
      activeStreamClient = null;
    }
  }

  private setBackend(kind: StreamBackend) {
    if (this.backend && this.backendKind === kind) {
      return;
    }
    this.backend?.destroy();
    if (kind === "h264-ws") {
      this.backend = new H264WebSocketStreamClient(this.handleBackendMessage);
    } else {
      this.backend = new WebRtcStreamClient(this.handleBackendMessage);
    }
    this.backendKind = kind;
    if (this.canvasElement) {
      this.backend.attachCanvas(this.canvasElement);
    }
  }

  private readonly handleBackendMessage = (message: WorkerToMainMessage) => {
    if (
      message.type === "status" &&
      message.status.state === "error" &&
      preferredStreamBackend(this.target) === "auto" &&
      this.target
    ) {
      const nextBackend = nextAutoFallbackBackend(this.backendKind);
      if (!nextBackend) {
        this.onMessage(message);
        return;
      }
      const target = this.target;
      this.setBackend(nextBackend);
      this.onMessage({
        type: "status",
        status: {
          detail: "Falling back to H264 WebSocket",
          state: "connecting",
        },
      });
      this.backend?.connect(target);
      return;
    }
    this.onMessage(message);
  };
}

export function preferredStreamBackend(
  target?: StreamConnectTarget | null,
): "auto" | StreamBackend {
  const value =
    target?.transport ??
    new URLSearchParams(window.location.search).get("stream");
  if (value === "h264" || value === "h264-ws") {
    return "h264-ws";
  }
  return value === "webrtc" || value === "rgba" || value === "webrtc-rgba"
    ? "webrtc"
    : "auto";
}

export function initialStreamBackend(
  target: StreamConnectTarget,
): StreamBackend {
  const preferredBackend = preferredStreamBackend(target);
  if (preferredBackend === "h264-ws") {
    return canUseH264WebSocket() ? "h264-ws" : "webrtc";
  }
  if (preferredBackend === "webrtc") {
    return canUseWebRtc() ? "webrtc" : "h264-ws";
  }
  return canUseWebRtc() ? "webrtc" : "h264-ws";
}

function nextAutoFallbackBackend(
  current: StreamBackend | null,
): StreamBackend | null {
  if (current === "webrtc") {
    return canUseH264WebSocket() ? "h264-ws" : null;
  }
  return null;
}
