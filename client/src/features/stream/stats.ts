import type { StreamStats } from "./streamTypes";

export function createEmptyStreamStats(): StreamStats {
  return {
    averageRenderMs: 0,
    codec: "",
    decodeQueueSize: 0,
    decodedFrames: 0,
    droppedFrames: 0,
    frameSequence: 0,
    height: 0,
    latestFrameGapMs: 0,
    latestRenderMs: 0,
    maxRenderMs: 0,
    receivedPackets: 0,
    reconnects: 0,
    renderedFrames: 0,
    waitingForKeyFrame: false,
    width: 0,
  };
}
