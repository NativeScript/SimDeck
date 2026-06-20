import type { ChromeProfile } from "../../api/types";

export interface Size {
  width: number;
  height: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface TouchPreviewPoint extends Point {
  clientX?: number;
  clientY?: number;
  pageX?: number;
  pageY?: number;
}

export interface TouchIndicator {
  id: number;
  phase: "began" | "moved" | "ended" | "cancelled";
  space?: "screen" | "canvas";
  x: number;
  y: number;
}

export type ViewMode = "fit" | "center" | "manual";

export interface ScreenRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ViewportLayoutState {
  canvasSize: Size | null;
  chromeProfile: ChromeProfile | null;
  deviceNaturalSize: Size | null;
  pan: Point;
  rotationQuarterTurns?: number;
  reservedBottomInset?: number;
  viewMode: ViewMode;
  zoom: number | null;
}
