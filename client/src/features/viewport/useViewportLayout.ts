import { computeCenterScale, computeFitScale, clampZoom } from "./viewportMath";
import type { ViewportLayoutState } from "./types";

export function useViewportLayout({
  canvasSize,
  chromeProfile,
  deviceNaturalSize,
  reservedBottomInset,
  viewMode,
  zoom
}: ViewportLayoutState) {
  const fitScale = computeFitScale(
    canvasSize,
    deviceNaturalSize,
    chromeProfile,
    reservedBottomInset
  );
  const centerScale = computeCenterScale(
    canvasSize,
    deviceNaturalSize,
    chromeProfile,
    reservedBottomInset
  );
  const effectiveZoom = clampZoom(
    viewMode === "fit"
      ? fitScale
      : viewMode === "center"
        ? centerScale
        : zoom ?? centerScale,
    fitScale
  );

  return {
    fitScale,
    centerScale,
    effectiveZoom
  };
}
