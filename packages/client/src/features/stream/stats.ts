import type { StreamStats } from "./streamTypes";

export function createEmptyStreamStats(): StreamStats {
  return {
    averageRenderMs: 0,
    codec: "",
    decodeQueueSize: 0,
    decodedFrames: 0,
    decoderDroppedFrames: 0,
    droppedFrames: 0,
    frameSequence: 0,
    height: 0,
    iceRestartReason: "",
    iceRestarts: 0,
    latestFrameGapMs: 0,
    latestRenderMs: 0,
    maxRenderMs: 0,
    packetsLost: 0,
    presentationDroppedFrames: 0,
    receivedPackets: 0,
    reconnectReason: "",
    reconnects: 0,
    renderedFrames: 0,
    waitingForKeyFrame: false,
    width: 0,
  };
}
