import { apiHeaders, fetchHealth } from "../../api/client";
import type { HealthResponse } from "../../api/types";
import { createEmptyStreamStats } from "./stats";
import type {
  StreamConnectTarget,
  StreamStats,
  WorkerToMainMessage,
} from "./streamTypes";

const HAVE_CURRENT_DATA = 2;
const WEBRTC_CONTROL_CHANNEL_LABEL = "simdeck-control";
const WEBRTC_FIRST_FRAME_TIMEOUT_MS = 10000;
const WEBRTC_STALLED_FRAME_TIMEOUT_MS = 8000;

let activeWebRtcControlChannel: RTCDataChannel | null = null;
let activeStreamClient: StreamWorkerClient | null = null;

export type StreamBackend = "mjpeg-img" | "webrtc";

export function sendWebRtcControlMessage(encoded: string): boolean {
  if (activeWebRtcControlChannel?.readyState !== "open") {
    return false;
  }
  activeWebRtcControlChannel.send(encoded);
  return true;
}

export function buildStreamTarget(udid: string): StreamConnectTarget {
  return { udid };
}

export function canUseWebRtc(): boolean {
  return typeof RTCPeerConnection === "function";
}

interface StreamClientBackend {
  attachCanvas(canvasElement: HTMLCanvasElement): void;
  clear(): void;
  connect(target: StreamConnectTarget): void | Promise<void>;
  destroy(): void;
  disconnect(): void;
}

class MjpegImageStreamClient implements StreamClientBackend {
  private canvas: HTMLCanvasElement | null = null;
  private connectGeneration = 0;
  private image: HTMLImageElement | null = null;
  private statsInterval = 0;
  private stats: StreamStats = createEmptyStreamStats();

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

  connect(target: StreamConnectTarget) {
    this.disconnect();
    if (!this.canvas) {
      return;
    }
    const canvasElement = this.canvas;
    const generation = ++this.connectGeneration;
    this.stats = {
      ...createEmptyStreamStats(),
      codec: "mjpeg",
    };
    this.onMessage({
      type: "status",
      status: { detail: "Opening MJPEG image stream", state: "connecting" },
    });

    const image = document.createElement("img");
    image.alt = "";
    image.className = "stream-image";
    image.decoding = "async";
    image.draggable = false;
    image.src = buildMjpegStreamUrl(target.udid);
    image.addEventListener("load", () => {
      if (generation !== this.connectGeneration) {
        return;
      }
      this.reportImageSize(image);
      this.stats.decodedFrames = Math.max(1, this.stats.decodedFrames);
      this.stats.renderedFrames = Math.max(1, this.stats.renderedFrames);
      this.stats.receivedPackets = Math.max(1, this.stats.receivedPackets);
      this.onMessage({ type: "stats", stats: { ...this.stats } });
      this.onMessage({ type: "status", status: { state: "streaming" } });
    });
    image.addEventListener("error", () => {
      if (generation !== this.connectGeneration) {
        return;
      }
      this.onMessage({
        type: "status",
        status: { error: "MJPEG image stream failed.", state: "error" },
      });
    });
    canvasElement.after(image);
    this.image = image;
    this.statsInterval = window.setInterval(() => {
      if (generation !== this.connectGeneration || !this.image) {
        return;
      }
      this.reportImageSize(this.image);
      this.onMessage({ type: "stats", stats: { ...this.stats } });
    }, 1000);
  }

  destroy() {
    this.disconnect();
  }

  disconnect() {
    this.connectGeneration += 1;
    if (this.statsInterval) {
      window.clearInterval(this.statsInterval);
      this.statsInterval = 0;
    }
    if (this.image) {
      this.image.removeAttribute("src");
      this.image.remove();
      this.image = null;
    }
    this.onMessage({ type: "status", status: { state: "idle" } });
  }

  private reportImageSize(image: HTMLImageElement) {
    const width = image.naturalWidth;
    const height = image.naturalHeight;
    if (width <= 0 || height <= 0) {
      return;
    }
    if (this.canvas) {
      if (this.canvas.width !== width) {
        this.canvas.width = width;
      }
      if (this.canvas.height !== height) {
        this.canvas.height = height;
      }
    }
    this.stats.width = width;
    this.stats.height = height;
    this.onMessage({ type: "video-config", size: { width, height } });
  }
}

class WebRtcStreamClient implements StreamClientBackend {
  private animationFrame = 0;
  private canvas: HTMLCanvasElement | null = null;
  private connectGeneration = 0;
  private controlChannel: RTCDataChannel | null = null;
  private diagnostics = createWebRtcDiagnostics();
  private frameWatchdogTimeout = 0;
  private lastVideoFrameAt = 0;
  private peerConnection: RTCPeerConnection | null = null;
  private reconnectTimeout = 0;
  private reportedVideoConfig = false;
  private shouldReconnect = false;
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

  async connect(target: StreamConnectTarget) {
    this.disconnect();
    if (!this.canvas) {
      return;
    }
    const canvasElement = this.canvas;
    const generation = ++this.connectGeneration;
    this.shouldReconnect = true;
    this.diagnostics = createWebRtcDiagnostics();
    this.reportedVideoConfig = false;
    this.stats = createEmptyStreamStats();
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
      this.attachDiagnostics(peerConnection, target, generation);
      const transceiver = peerConnection.addTransceiver("video", {
        direction: "recvonly",
      });
      configureReceiverCodecPreferences(transceiver);
      configureLowLatencyReceiver(transceiver.receiver);
      const controlChannel = peerConnection.createDataChannel(
        WEBRTC_CONTROL_CHANNEL_LABEL,
        {
          maxPacketLifeTime: 250,
          ordered: false,
        },
      );
      this.controlChannel = controlChannel;
      activeWebRtcControlChannel = controlChannel;
      controlChannel.addEventListener("close", () => {
        if (activeWebRtcControlChannel === controlChannel) {
          activeWebRtcControlChannel = null;
        }
      });

      peerConnection.ontrack = (event) => {
        if (generation !== this.connectGeneration) {
          return;
        }
        event.track.contentHint = "motion";
        for (const receiver of peerConnection.getReceivers()) {
          configureLowLatencyReceiver(receiver);
        }
        const stream = event.streams[0] ?? new MediaStream([event.track]);
        const video = document.createElement("video");
        video.autoplay = true;
        video.className = "stream-video";
        video.disablePictureInPicture = true;
        video.muted = true;
        video.playsInline = true;
        video.setAttribute("playsinline", "");
        video.setAttribute("webkit-playsinline", "");
        video.preload = "auto";
        (video as HTMLVideoElement & { latencyHint?: string }).latencyHint =
          "interactive";
        video.srcObject = stream;
        canvasElement.after(video);
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
        if (
          generation === this.connectGeneration &&
          (peerConnection.connectionState === "failed" ||
            peerConnection.connectionState === "disconnected")
        ) {
          if (peerConnection.connectionState === "failed") {
            void this.updateSelectedCandidatePair(peerConnection, target);
          }
          this.handleConnectionError(
            target,
            generation,
            new Error(`WebRTC connection ${peerConnection.connectionState}.`),
          );
        }
      };

      const offer = await peerConnection.createOffer();
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
      this.postDiagnostics(target, "local-offer");

      const response = await postWebRtcOfferWithAuthRetry(
        target.udid,
        localDescription,
      );
      const answer = (await response.json()) as RTCSessionDescriptionInit;
      if (generation !== this.connectGeneration) {
        return;
      }
      this.diagnostics.remoteCandidateSummary = summarizeSdpCandidates(
        answer.sdp ?? "",
      );
      this.postDiagnostics(target, "remote-answer");
      await peerConnection.setRemoteDescription(answer);
      this.scheduleFrameWatchdog(target, generation);
    } catch (error) {
      this.handleConnectionError(target, generation, error);
    }
  }

  disconnect() {
    this.shouldReconnect = false;
    this.connectGeneration += 1;
    this.clearReconnectTimeout();
    this.clearFrameWatchdog();
    this.closeActiveConnection();
    this.onMessage({ type: "status", status: { state: "idle" } });
  }

  destroy() {
    this.disconnect();
  }

  private closeActiveConnection() {
    window.cancelAnimationFrame(this.animationFrame);
    this.animationFrame = 0;
    this.clearFrameWatchdog();
    this.cancelVideoFrameCallback();
    this.video?.pause();
    if (this.video) {
      this.video.srcObject = null;
      this.video.remove();
    }
    this.video = null;
    this.reportedVideoConfig = false;
    this.controlChannel?.close();
    if (activeWebRtcControlChannel === this.controlChannel) {
      activeWebRtcControlChannel = null;
    }
    this.controlChannel = null;
    this.peerConnection?.close();
    this.peerConnection = null;
  }

  private handleConnectionError(
    target: StreamConnectTarget,
    generation: number,
    error: unknown,
  ) {
    if (generation !== this.connectGeneration || !this.shouldReconnect) {
      return;
    }
    const message = error instanceof Error ? error.message : String(error);
    this.closeActiveConnection();
    this.onMessage({
      type: "status",
      status: { error: message, state: "error" },
    });
    this.scheduleReconnect(target, generation);
  }

  private scheduleReconnect(target: StreamConnectTarget, generation: number) {
    if (
      this.reconnectTimeout ||
      generation !== this.connectGeneration ||
      !this.shouldReconnect
    ) {
      return;
    }
    this.stats.reconnects += 1;
    this.onMessage({ type: "stats", stats: { ...this.stats } });
    this.reconnectTimeout = window.setTimeout(() => {
      this.reconnectTimeout = 0;
      if (generation === this.connectGeneration && this.shouldReconnect) {
        void this.connect(target);
      }
    }, 750);
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
        if (!hasRenderedFrame || frameAgeMs > WEBRTC_STALLED_FRAME_TIMEOUT_MS) {
          this.handleConnectionError(
            target,
            generation,
            new Error("WebRTC video stalled before rendering fresh frames."),
          );
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
      clientId: "webrtc-page",
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
    void fetch(new URL("/api/client-stream-stats", window.location.href), {
      body: JSON.stringify(payload),
      cache: "no-store",
      headers: apiHeaders(),
      method: "POST",
    }).catch(() => {
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
      const renderStartedAt = performance.now();
      const latestRenderMs = performance.now() - renderStartedAt;
      this.stats.decodedFrames += 1;
      this.stats.renderedFrames += 1;
      this.stats.receivedPackets += 1;
      this.stats.width = this.canvas.width;
      this.stats.height = this.canvas.height;
      this.stats.codec = "webrtc";
      this.stats.latestRenderMs = latestRenderMs;
      this.stats.maxRenderMs = Math.max(this.stats.maxRenderMs, latestRenderMs);
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
    if (this.reportedVideoConfig) {
      return;
    }
    this.reportedVideoConfig = true;
    this.onMessage({
      type: "video-config",
      size: { height, width },
    });
    this.onMessage({
      type: "status",
      status: { detail: "WebRTC media connected", state: "streaming" },
    });
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
}

async function postWebRtcOfferWithAuthRetry(
  udid: string,
  localDescription: RTCSessionDescription,
): Promise<Response> {
  const response = await postWebRtcOffer(udid, localDescription);
  if (response.status !== 401) {
    if (!response.ok) {
      throw new Error(await response.text());
    }
    return response;
  }
  await fetchHealth();
  const retry = await postWebRtcOffer(udid, localDescription);
  if (!retry.ok) {
    throw new Error(await retry.text());
  }
  return retry;
}

function postWebRtcOffer(
  udid: string,
  localDescription: RTCSessionDescription,
): Promise<Response> {
  return fetch(`/api/simulators/${encodeURIComponent(udid)}/webrtc/offer`, {
    body: JSON.stringify({
      sdp: localDescription.sdp,
      type: localDescription.type,
    }),
    headers: apiHeaders(),
    method: "POST",
  });
}

function configureLowLatencyReceiver(receiver: RTCRtpReceiver) {
  const lowLatencyReceiver = receiver as RTCRtpReceiver & {
    jitterBufferTarget?: number;
    playoutDelayHint?: number;
  };
  if ("jitterBufferTarget" in lowLatencyReceiver) {
    lowLatencyReceiver.jitterBufferTarget = 0.001;
  }
  if ("playoutDelayHint" in lowLatencyReceiver) {
    lowLatencyReceiver.playoutDelayHint = 0.001;
  }
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

function buildMjpegStreamUrl(udid: string): string {
  const url = new URL(
    `/api/simulators/${encodeURIComponent(udid)}/mjpeg`,
    window.location.href,
  );
  const token = new URL(window.location.href).searchParams.get("simdeckToken");
  if (token) {
    url.searchParams.set("simdeckToken", token);
  }
  url.searchParams.set("cacheBust", String(Date.now()));
  return url.toString();
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
  private canvas: HTMLCanvasElement | null = null;
  private attachedCanvas = false;
  private disposed = false;

  constructor(onMessage: (message: WorkerToMainMessage) => void) {
    this.onMessage = onMessage;
    if (activeStreamClient && activeStreamClient !== this) {
      activeStreamClient.destroy();
    }
    activeStreamClient = this;
  }

  attachCanvas(canvasElement: HTMLCanvasElement) {
    if (this.attachedCanvas) {
      return;
    }

    this.canvas = canvasElement;
    this.attachedCanvas = true;
  }

  async connect(target: StreamConnectTarget) {
    try {
      const health = await fetchHealth().catch(() => null);
      const shouldUseMjpeg = health?.videoCodec === "jpeg";
      const isMjpegBackend = this.backend instanceof MjpegImageStreamClient;
      if (!this.backend || shouldUseMjpeg !== isMjpegBackend) {
        this.backend?.destroy();
        this.backend = shouldUseMjpeg
          ? new MjpegImageStreamClient(this.onMessage)
          : new WebRtcStreamClient(this.onMessage);
        if (this.canvas) {
          this.backend.attachCanvas(this.canvas);
        }
      }
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

  destroy() {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    this.backend?.destroy();
    this.backend = null;
    if (activeStreamClient === this) {
      activeStreamClient = null;
    }
  }
}
