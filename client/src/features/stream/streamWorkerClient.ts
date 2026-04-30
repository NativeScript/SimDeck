import { accessTokenFromLocation, apiHeaders } from "../../api/client";
import { createEmptyStreamStats } from "./stats";
import type {
  StreamConnectTarget,
  StreamStats,
  WorkerToMainMessage,
} from "./streamTypes";

const HAVE_CURRENT_DATA = 2;
const WEBRTC_CONTROL_CHANNEL_LABEL = "simdeck-control";

let activeWebRtcControlChannel: RTCDataChannel | null = null;

export type StreamBackend = "webtransport" | "webrtc";

export function isWebRtcStreamMode(): boolean {
  return streamTransportMode() === "webrtc";
}

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

export function initialStreamBackend(): StreamBackend {
  const mode = streamTransportMode();
  if (mode === "webrtc") {
    return "webrtc";
  }
  if (mode === "webtransport") {
    return "webtransport";
  }
  if (canUseWebTransport()) {
    return "webtransport";
  }
  return canUseWebRtc() ? "webrtc" : "webtransport";
}

export function streamModeIsForcedWebTransport(): boolean {
  return streamTransportMode() === "webtransport";
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

class WorkerStreamClient implements StreamClientBackend {
  private readonly worker: Worker;

  constructor(onMessage: (message: WorkerToMainMessage) => void) {
    this.worker = new Worker(
      new URL("../../workers/simulatorStream.worker.ts", import.meta.url),
      {
        type: "module",
      },
    );
    this.worker.onmessage = (event: MessageEvent<WorkerToMainMessage>) => {
      onMessage(event.data);
    };
  }

  attachCanvas(canvasElement: HTMLCanvasElement) {
    const offscreenCanvas = canvasElement.transferControlToOffscreen();
    this.worker.postMessage(
      { type: "attach-canvas", canvas: offscreenCanvas },
      [offscreenCanvas],
    );
  }

  connect(target: StreamConnectTarget) {
    this.worker.postMessage({ type: "connect", target });
  }

  disconnect() {
    this.worker.postMessage({ type: "disconnect" });
  }

  clear() {
    this.worker.postMessage({ type: "clear" });
  }

  destroy() {
    this.worker.terminate();
  }
}

class WebRtcStreamClient implements StreamClientBackend {
  private animationFrame = 0;
  private canvas: HTMLCanvasElement | null = null;
  private connectGeneration = 0;
  private context: CanvasRenderingContext2D | null = null;
  private controlChannel: RTCDataChannel | null = null;
  private diagnostics = createWebRtcDiagnostics();
  private peerConnection: RTCPeerConnection | null = null;
  private reconnectTimeout = 0;
  private shouldReconnect = false;
  private stats: StreamStats = createEmptyStreamStats();
  private video: HTMLVideoElement | null = null;
  private videoFrameCallback = 0;

  constructor(
    private readonly onMessage: (message: WorkerToMainMessage) => void,
  ) {}

  attachCanvas(canvasElement: HTMLCanvasElement) {
    this.canvas = canvasElement;
    this.context = canvasElement.getContext("2d", {
      alpha: false,
      desynchronized: true,
    } as CanvasRenderingContext2DSettings & { desynchronized: boolean });
    if (!this.context) {
      throw new Error("Unable to create a 2D canvas renderer for WebRTC.");
    }
  }

  clear() {
    if (!this.canvas || !this.context) {
      return;
    }
    this.context.clearRect(0, 0, this.canvas.width, this.canvas.height);
  }

  async connect(target: StreamConnectTarget) {
    this.disconnect();
    if (!this.canvas || !this.context) {
      return;
    }
    const generation = ++this.connectGeneration;
    this.shouldReconnect = true;
    this.diagnostics = createWebRtcDiagnostics();
    this.stats = createEmptyStreamStats();
    this.onMessage({
      type: "status",
      status: { detail: "Creating WebRTC offer", state: "connecting" },
    });

    try {
      const peerConnection = new RTCPeerConnection({
        iceServers: iceServers(),
        iceTransportPolicy: iceTransportPolicy(),
      });
      this.peerConnection = peerConnection;
      this.attachDiagnostics(peerConnection, target, generation);
      const transceiver = peerConnection.addTransceiver("video", {
        direction: "recvonly",
      });
      configureLowLatencyReceiver(transceiver.receiver);
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
        video.muted = true;
        video.playsInline = true;
        video.preload = "auto";
        video.srcObject = stream;
        this.video = video;
        video.onloadedmetadata = () => {
          if (generation !== this.connectGeneration) {
            return;
          }
          void video.play().catch(() => {
            // The media stream can be detached during reconnect; retry on the next track.
          });
          this.syncCanvasSize(video.videoWidth, video.videoHeight);
          this.onMessage({
            type: "video-config",
            size: { height: video.videoHeight, width: video.videoWidth },
          });
          this.onMessage({
            type: "status",
            status: { detail: "WebRTC media connected", state: "streaming" },
          });
          this.scheduleVideoFrame();
        };
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

      const response = await fetch(
        `/api/simulators/${encodeURIComponent(target.udid)}/webrtc/offer`,
        {
          body: JSON.stringify({
            sdp: localDescription.sdp,
            type: localDescription.type,
          }),
          headers: apiHeaders(),
          method: "POST",
        },
      );
      if (!response.ok) {
        throw new Error(await response.text());
      }
      const answer = (await response.json()) as RTCSessionDescriptionInit;
      if (generation !== this.connectGeneration) {
        return;
      }
      this.diagnostics.remoteCandidateSummary = summarizeSdpCandidates(
        answer.sdp ?? "",
      );
      this.postDiagnostics(target, "remote-answer");
      await peerConnection.setRemoteDescription(answer);
    } catch (error) {
      this.handleConnectionError(target, generation, error);
    }
  }

  disconnect() {
    this.shouldReconnect = false;
    this.connectGeneration += 1;
    this.clearReconnectTimeout();
    this.closeActiveConnection();
    this.onMessage({ type: "status", status: { state: "idle" } });
  }

  destroy() {
    this.disconnect();
  }

  private closeActiveConnection() {
    window.cancelAnimationFrame(this.animationFrame);
    this.animationFrame = 0;
    this.cancelVideoFrameCallback();
    this.video?.pause();
    if (this.video) {
      this.video.srcObject = null;
    }
    this.video = null;
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
    if (!this.canvas || !this.context || !this.video) {
      return;
    }
    if (
      this.video.readyState >= HAVE_CURRENT_DATA &&
      this.video.videoWidth > 0 &&
      this.video.videoHeight > 0
    ) {
      this.syncCanvasSize(this.video.videoWidth, this.video.videoHeight);
      try {
        this.context.drawImage(
          this.video,
          0,
          0,
          this.canvas.width,
          this.canvas.height,
        );
      } catch {
        this.scheduleVideoFrame();
        return;
      }
      this.stats.decodedFrames += 1;
      this.stats.renderedFrames += 1;
      this.stats.receivedPackets += 1;
      this.stats.width = this.canvas.width;
      this.stats.height = this.canvas.height;
      this.stats.codec = "webrtc";
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

function configureLowLatencyReceiver(receiver: RTCRtpReceiver) {
  const lowLatencyReceiver = receiver as RTCRtpReceiver & {
    jitterBufferTarget?: number;
  };
  if ("jitterBufferTarget" in lowLatencyReceiver) {
    lowLatencyReceiver.jitterBufferTarget = 0.001;
  }
}

function streamTransportMode(): string {
  if (typeof window === "undefined") {
    return "auto";
  }
  return new URLSearchParams(window.location.search).get("transport") ?? "auto";
}

function iceServers(): RTCIceServer[] {
  const params = new URLSearchParams(window.location.search);
  const raw = params.get("iceServers") ?? "stun:stun.l.google.com:19302";
  if (raw === "none") {
    return [];
  }
  return [
    {
      urls: raw
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean),
    },
  ];
}

function iceTransportPolicy(): RTCIceTransportPolicy {
  const value = new URLSearchParams(window.location.search).get(
    "iceTransportPolicy",
  );
  return value === "relay" || value === "all" ? value : "all";
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

  constructor(
    onMessage: (message: WorkerToMainMessage) => void,
    private readonly backendMode: StreamBackend,
  ) {
    this.onMessage = onMessage;
  }

  attachCanvas(canvasElement: HTMLCanvasElement) {
    if (this.attachedCanvas) {
      return;
    }

    this.backend = this.createBackend(canvasElement);
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

  destroy() {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    this.backend?.destroy();
    this.backend = null;
  }

  private createBackend(canvasElement: HTMLCanvasElement): StreamClientBackend {
    if (this.backendMode === "webrtc") {
      return new WebRtcStreamClient(this.onMessage);
    }
    void canvasElement;
    return new WorkerStreamClient(this.onMessage);
  }
}

function canUseWebTransport(): boolean {
  return typeof WebTransport === "function" && window.isSecureContext;
}
