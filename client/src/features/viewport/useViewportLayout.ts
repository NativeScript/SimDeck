import { computeCenterScale, computeFitScale, clampZoom } from "./viewportMath";
import type { ViewportLayoutState } from "./types";

export function useViewportLayout({
  canvasSize,
  chromeProfile,
  deviceNaturalSize,
  rotationQuarterTurns,
  reservedBottomInset,
  viewMode,
  zoom,
}: ViewportLayoutState) {
  const fitScale = computeFitScale(
    canvasSize,
    deviceNaturalSize,
    chromeProfile,
    reservedBottomInset,
    rotationQuarterTurns,
  );
  const centerScale = computeCenterScale(
    canvasSize,
    deviceNaturalSize,
    chromeProfile,
    reservedBottomInset,
    rotationQuarterTurns,
  );
  const effectiveZoom = clampZoom(
    viewMode === "fit"
      ? fitScale
      : viewMode === "center"
        ? centerScale
        : (zoom ?? centerScale),
    fitScale,
  );

  return {
    fitScale,
    centerScale,
    effectiveZoom,
  };
}
