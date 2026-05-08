import type { Size } from "../viewport/types";

export interface StreamConnectTarget {
  clientId?: string;
  remote?: boolean;
  streamConfig?: StreamConfig;
  transport?: StreamTransport;
  udid: string;
}

export type StreamEncoder = "auto" | "hardware" | "software";
export type StreamFps = number;
export type StreamTransport = "auto" | "h264" | "webrtc";
export type StreamQualityPreset =
  | "auto"
  | "balanced"
  | "ci-software"
  | "economy"
  | "fast"
  | "full"
  | "low"
  | "quality"
  | "smooth"
  | "tiny";

export interface StreamConfig {
  encoder: StreamEncoder;
  fps: StreamFps;
  maxEdge?: number;
  quality: StreamQualityPreset;
}

export interface StreamPacketMetadata extends Size {
  codec?: string;
  description?: string | Uint8Array;
  frameSequence: number;
  isKeyFrame: boolean;
  timestampUs: number;
}

export interface StreamPacket {
  metadata: StreamPacketMetadata;
  payload: Uint8Array;
}

export interface StreamStats extends Size {
  averageRenderMs: number;
  codec: string;
  decodeQueueSize: number;
  decodedFrames: number;
  decoderDroppedFrames: number;
  droppedFrames: number;
  frameSequence: number;
  h264ParseFailures: number;
  h264SocketBytes: number;
  h264SocketMessages: number;
  h264SocketMessageType: string;
  iceRestartReason: string;
  iceRestarts: number;
  latestFrameGapMs: number;
  latestRenderMs: number;
  maxRenderMs: number;
  packetsLost: number;
  presentationDroppedFrames: number;
  receivedPackets: number;
  reconnectReason: string;
  reconnects: number;
  renderedFrames: number;
  waitingForKeyFrame: boolean;
}

export interface StreamRuntimeInfo {
  gpuLikelyHardware: boolean | null;
  gpuRenderer: string;
  gpuVendor: string;
  renderBackend: string;
  streamBackend: string;
  webGL2: boolean;
}

export type StreamConnectionState =
  | "idle"
  | "connecting"
  | "streaming"
  | "error";

export interface StreamStatus {
  detail?: string;
  error?: string;
  state: StreamConnectionState;
}

export type WorkerToMainMessage =
  | { type: "stats"; stats: StreamStats }
  | { type: "status"; status: StreamStatus }
  | { type: "video-config"; size: Size };

export type MainToWorkerMessage =
  | { type: "attach-canvas"; canvas: OffscreenCanvas }
  | { type: "clear" }
  | { type: "connect"; target: StreamConnectTarget }
  | { type: "disconnect" };
