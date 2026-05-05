import { apiHeaders, fetchHealth } from "../../api/client";
import { apiUrl } from "../../api/config";
import type { HealthResponse } from "../../api/types";
import { createEmptyStreamStats } from "./stats";
import type {
  StreamConnectTarget,
  StreamConfig,
  StreamStats,
  WorkerToMainMessage,
} from "./streamTypes";

const HAVE_CURRENT_DATA = 2;
const WEBRTC_CONTROL_CHANNEL_LABEL = "simdeck-control";
const WEBRTC_TELEMETRY_CHANNEL_LABEL = "simdeck-telemetry";
const WEBRTC_FIRST_FRAME_TIMEOUT_MS = 10000;
const WEBRTC_STALLED_FRAME_TIMEOUT_MS = 3000;
const WEBRTC_LOCAL_RECEIVER_BUFFER_SECONDS = 0.001;
const WEBRTC_REMOTE_RECEIVER_BUFFER_SECONDS = 0.06;
const WEBRTC_LOCAL_DISCONNECTED_GRACE_MS = 1000;
const WEBRTC_REMOTE_DISCONNECTED_GRACE_MS = 10000;
const WEBRTC_REMOTE_ICE_RESTART_GRACE_MS = 1500;
const WEBRTC_RECONNECT_BASE_DELAY_MS = 250;
const WEBRTC_RECONNECT_MAX_DELAY_MS = 1000;

let activeWebRtcControlChannel: RTCDataChannel | null = null;
let activeWebRtcTelemetryChannel: RTCDataChannel | null = null;
let activeStreamClient: StreamWorkerClient | null = null;

export type StreamBackend = "webrtc";

export function sendWebRtcControlMessage(encoded: string): boolean {
  return sendDataChannelMessage(activeWebRtcControlChannel, encoded);
}

export function sendWebRtcClientStats(stats: unknown): boolean {
  return sendDataChannelMessage(
    activeWebRtcTelemetryChannel,
    JSON.stringify({ stats, type: "clientStats" }),
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

function sendDataChannelMessage(
  channel: RTCDataChannel | null,
  encoded: string,
): boolean {
  if (channel?.readyState !== "open") {
    return false;
  }
  channel.send(encoded);
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
    remote?: boolean;
    streamConfig?: StreamConfig;
  } = {},
): StreamConnectTarget {
  return {
    clientId: options.clientId,
    remote: options.remote,
    streamConfig: options.streamConfig,
    udid,
  };
}

export function canUseWebRtc(): boolean {
  return typeof RTCPeerConnection === "function";
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

class WebRtcStreamClient implements StreamClientBackend {
  private animationFrame = 0;
  private canvas: HTMLCanvasElement | null = null;
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
  }

  clear() {
    this.canvas
      ?.getContext("2d")
      ?.clearRect(0, 0, this.canvas.width, this.canvas.height);
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
      try {
        await postStreamConfigWithAuthRetry(target.streamConfig, {
          remote: target.remote,
        });
      } catch (error) {
        if (!target.remote) {
          throw error;
        }
      }
      if (generation !== this.connectGeneration) {
        return;
      }
      const health = await fetchHealth().catch(() => null);
      if (generation !== this.connectGeneration) {
        return;
      }
      const peerConnection = new RTCPeerConnection({
        iceServers: iceServers(health),
        iceTransportPolicy: iceTransportPolicy(health),
      });
      this.peerConnection = peerConnection;
      this.attachDiagnostics(peerConnection, target, generation);
      this.startReceiverStatsPolling(peerConnection, target, generation);
      const transceiver = peerConnection.addTransceiver("video", {
        direction: "recvonly",
      });
      configureReceiverCodecPreferences(transceiver);
      configureLowLatencyReceiver(
        transceiver.receiver,
        receiverBufferSeconds(target),
      );
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

      peerConnection.ontrack = (event) => {
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
        canvasElement.parentElement?.insertBefore(
          video,
          canvasElement.nextSibling,
        );
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
              status: { detail: "WebRTC media connected", state: "streaming" },
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
    await postStreamConfigWithAuthRetry(config, { remote: this.remoteMode });
    if (generation !== this.streamConfigGeneration) {
      return;
    }
    const target = this.streamTarget;
    if (target && this.shouldReconnect) {
      await this.connect({ ...target, streamConfig: config });
      return;
    }
    this.sendControl({ forceKeyframe: true, type: "streamControl" });
  }

  private async negotiatePeerConnection(
    peerConnection: RTCPeerConnection,
    target: StreamConnectTarget,
    generation: number,
    options: { detailPrefix: string; iceRestart?: boolean },
  ) {
    const offer = safariBaselineH264Offer(
      await peerConnection.createOffer({ iceRestart: options.iceRestart }),
    );
    if (generation !== this.connectGeneration) {
      return;
    }
    await peerConnection.setLocalDescription(offer);
    await waitForIceGathering(peerConnection);
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

    const response = await postWebRtcOfferWithAuthRetry(
      target,
      localDescription,
    );
    const answer = (await response.json()) as RTCSessionDescriptionInit;
    if (generation !== this.connectGeneration) {
      return;
    }
    this.diagnostics.remoteCandidateSummary = summarizeSdpCandidates(
      answer.sdp ?? "",
    );
    this.postDiagnostics(target, `${options.detailPrefix}-answer`);
    await peerConnection.setRemoteDescription(answer);
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
        const hasRenderedFrame = this.stats.renderedFrames > 0;
        const frameAgeMs =
          this.lastVideoFrameAt > 0 ? now - this.lastVideoFrameAt : Infinity;
        if (!hasRenderedFrame) {
          this.handleConnectionError(
            target,
            generation,
            new Error("WebRTC video stalled before rendering fresh frames."),
            "first-frame-timeout",
          );
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
    if (sendWebRtcClientStats(payload) || this.remoteMode) {
      return;
    }
    void fetch(
      new URL(apiUrl("/api/client-stream-stats"), window.location.href),
      {
        body: JSON.stringify(payload),
        cache: "no-store",
        headers: apiHeaders(),
        method: "POST",
      },
    ).catch(() => {
      // Diagnostics only.
    });
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
      status: { detail: "WebRTC media connected", state: "streaming" },
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
): Promise<Response> {
  const response = await postWebRtcOffer(target, localDescription);
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
  const retry = await postWebRtcOffer(target, localDescription);
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
    body: JSON.stringify({
      fps: config.fps,
      profile: config.quality,
      videoCodec: config.encoder,
    }),
    headers: apiHeaders(),
    method: "POST",
  });
}

function postWebRtcOffer(
  target: StreamConnectTarget,
  localDescription: RTCSessionDescription,
): Promise<Response> {
  return fetch(
    apiUrl(`/api/simulators/${encodeURIComponent(target.udid)}/webrtc/offer`),
    {
      body: JSON.stringify({
        clientId: target.clientId,
        sdp: localDescription.sdp,
        type: localDescription.type,
      }),
      headers: apiHeaders(),
      method: "POST",
    },
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

function waitForIceGathering(peerConnection: RTCPeerConnection) {
  if (peerConnection.iceGatheringState === "complete") {
    return Promise.resolve();
  }
  return new Promise<void>((resolve) => {
    const timeout = window.setTimeout(resolve, 3000);
    peerConnection.addEventListener("icegatheringstatechange", () => {
      if (peerConnection.iceGatheringState === "complete") {
        window.clearTimeout(timeout);
        resolve();
      }
    });
  });
}

export class StreamWorkerClient {
  private readonly onMessage: (message: WorkerToMainMessage) => void;
  private backend: StreamClientBackend | null = null;
  private attachedCanvas = false;
  private disposed = false;
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
    if (this.attachedCanvas) {
      return;
    }

    this.backend = new WebRtcStreamClient(this.onMessage);
    this.backend.attachCanvas(canvasElement);
    this.attachedCanvas = true;
  }

  connect(target: StreamConnectTarget) {
    try {
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
}
