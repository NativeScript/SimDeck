import type { ChromeProfile } from "../../api/types";
import type { Point, Size, ViewMode } from "./types";
import { clampPan } from "./viewportMath";

const PAN_EPSILON = 0.001;

interface ViewportWheelState {
  viewMode: ViewMode;
  zoom: number | null;
}

interface ViewportWheelPanOptions {
  canvasSize: Size | null;
  chromeProfile: ChromeProfile | null;
  deltaX: number;
  deltaY: number;
  deviceNaturalSize: Size | null;
  effectiveZoom: number;
  fitScale: number;
  pan: Point;
  reservedBottomInset: number;
  rotationQuarterTurns: number;
  viewMode: ViewMode;
  zoom: number | null;
}

export function nextViewportWheelPanState({
  canvasSize,
  chromeProfile,
  deltaX,
  deltaY,
  deviceNaturalSize,
  effectiveZoom,
  fitScale,
  pan,
  reservedBottomInset,
  rotationQuarterTurns,
  viewMode,
  zoom,
}: ViewportWheelPanOptions): ViewportWheelState & { pan: Point } {
  if (effectiveZoom <= fitScale + PAN_EPSILON) {
    return { pan, viewMode, zoom };
  }

  const nextPan = clampPan(
    {
      x: pan.x - deltaX,
      y: pan.y - deltaY,
    },
    effectiveZoom,
    canvasSize,
    deviceNaturalSize,
    chromeProfile,
    rotationQuarterTurns,
    viewMode === "manual" ? reservedBottomInset : 0,
  );

  return {
    pan: nextPan,
    viewMode,
    zoom,
  };
}
