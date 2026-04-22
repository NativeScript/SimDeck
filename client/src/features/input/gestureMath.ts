import type { PointerEvent as ReactPointerEvent } from "react";

import type { Point } from "../viewport/types";

export function normalizedPointerCoordinates(
  event: PointerEvent | ReactPointerEvent<HTMLElement>,
): Point | null {
  const currentTarget = event.currentTarget as HTMLElement | null;
  if (!currentTarget) {
    return null;
  }
  const rect = currentTarget.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) {
    return null;
  }
  return {
    x: Math.min(Math.max((event.clientX - rect.left) / rect.width, 0), 1),
    y: Math.min(Math.max((event.clientY - rect.top) / rect.height, 0), 1),
  };
}
