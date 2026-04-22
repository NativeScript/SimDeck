import type { StreamStats } from "./streamTypes";

export function createEmptyStreamStats(): StreamStats {
  return {
    codec: "",
    decodedFrames: 0,
    droppedFrames: 0,
    frameSequence: 0,
    height: 0,
    receivedPackets: 0,
    reconnects: 0,
    width: 0,
  };
}
