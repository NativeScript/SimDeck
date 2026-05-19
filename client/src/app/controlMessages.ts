import type { ControlMessage } from "../api/controls";

export function isMoveControlMessage(message: ControlMessage): boolean {
  return (
    (message.type === "touch" ||
      message.type === "edgeTouch" ||
      message.type === "multiTouch") &&
    message.phase === "moved"
  );
}
