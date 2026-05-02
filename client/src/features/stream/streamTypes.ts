import type { Size } from "../viewport/types";

export interface StreamConnectTarget {
  remote?: boolean;
  udid: string;
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
  droppedFrames: number;
  frameSequence: number;
  latestFrameGapMs: number;
  latestRenderMs: number;
  maxRenderMs: number;
  receivedPackets: number;
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
