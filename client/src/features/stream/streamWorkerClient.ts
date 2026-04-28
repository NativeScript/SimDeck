import { accessTokenFromLocation, apiHeaders } from "../../api/client";
import { createEmptyStreamStats } from "./stats";
import type {
  StreamConnectTarget,
  StreamStats,
  WorkerToMainMessage,
} from "./streamTypes";

export function buildStreamTarget(udid: string): StreamConnectTarget {
  return { udid };
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
  private context: CanvasRenderingContext2D | null = null;
  private peerConnection: RTCPeerConnection | null = null;
  private stats: StreamStats = createEmptyStreamStats();
  private video: HTMLVideoElement | null = null;

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
    this.stats = createEmptyStreamStats();
    this.onMessage({
      type: "status",
      status: { detail: "Creating WebRTC offer", state: "connecting" },
    });

    const peerConnection = new RTCPeerConnection({
      iceServers: iceServers(),
    });
    this.peerConnection = peerConnection;
    peerConnection.addTransceiver("video", { direction: "recvonly" });

    peerConnection.ontrack = (event) => {
      const [stream] = event.streams;
      if (!stream) {
        return;
      }
      const video = document.createElement("video");
      video.autoplay = true;
      video.muted = true;
      video.playsInline = true;
      video.srcObject = stream;
      this.video = video;
      video.onloadedmetadata = () => {
        void video.play();
        this.syncCanvasSize(video.videoWidth, video.videoHeight);
        this.onMessage({
          type: "video-config",
          size: { height: video.videoHeight, width: video.videoWidth },
        });
        this.onMessage({
          type: "status",
          status: { detail: "WebRTC media connected", state: "streaming" },
        });
        this.drawVideoFrame();
      };
    };

    peerConnection.onconnectionstatechange = () => {
      if (peerConnection.connectionState === "failed") {
        this.onMessage({
          type: "status",
          status: {
            error: "WebRTC connection failed.",
            state: "error",
          },
        });
      }
    };

    const offer = await peerConnection.createOffer();
    await peerConnection.setLocalDescription(offer);
    await waitForIceGathering(peerConnection);
    const localDescription = peerConnection.localDescription;
    if (!localDescription) {
      throw new Error("WebRTC local offer was not created.");
    }

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
    await peerConnection.setRemoteDescription(answer);
  }

  disconnect() {
    window.cancelAnimationFrame(this.animationFrame);
    this.animationFrame = 0;
    this.video?.pause();
    this.video = null;
    this.peerConnection?.close();
    this.peerConnection = null;
    this.onMessage({ type: "status", status: { state: "idle" } });
  }

  destroy() {
    this.disconnect();
  }

  private drawVideoFrame = () => {
    if (!this.canvas || !this.context || !this.video) {
      return;
    }
    if (this.video.videoWidth > 0 && this.video.videoHeight > 0) {
      this.syncCanvasSize(this.video.videoWidth, this.video.videoHeight);
      this.context.drawImage(
        this.video,
        0,
        0,
        this.canvas.width,
        this.canvas.height,
      );
      this.stats.decodedFrames += 1;
      this.stats.renderedFrames += 1;
      this.stats.receivedPackets += 1;
      this.stats.width = this.canvas.width;
      this.stats.height = this.canvas.height;
      this.stats.codec = "webrtc";
      this.onMessage({ type: "stats", stats: { ...this.stats } });
    }
    this.animationFrame = window.requestAnimationFrame(this.drawVideoFrame);
  };

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

function streamTransportMode(): string {
  if (typeof window === "undefined") {
    return "webtransport";
  }
  return (
    new URLSearchParams(window.location.search).get("transport") ??
    "webtransport"
  );
}

function iceServers(): RTCIceServer[] {
  const params = new URLSearchParams(window.location.search);
  const raw = params.get("iceServers") ?? "stun:stun.l.google.com:19302";
  return [
    {
      urls: raw
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean),
    },
  ];
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

  constructor(onMessage: (message: WorkerToMainMessage) => void) {
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
    if (streamTransportMode() === "webrtc" && accessTokenFromLocation()) {
      return new WebRtcStreamClient(this.onMessage);
    }
    void canvasElement;
    return new WorkerStreamClient(this.onMessage);
  }
}
