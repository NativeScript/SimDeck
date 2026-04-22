import type { Size } from "../viewport/types";

export interface StreamConnectTarget {
  stamp: number;
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
  codec: string;
  decodedFrames: number;
  droppedFrames: number;
  frameSequence: number;
  receivedPackets: number;
  reconnects: number;
}

export interface StreamRuntimeInfo {
  gpuLikelyHardware: boolean | null;
  gpuRenderer: string;
  gpuVendor: string;
  renderBackend: string;
  streamBackend: string;
  webCodecs: boolean;
  webGL2: boolean;
  webTransport: boolean;
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
  | { type: "disconnect" }
  | { type: "resize"; devicePixelRatio: number; width: number; height: number };
