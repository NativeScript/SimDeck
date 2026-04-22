import type { ChromeProfile } from "../../api/types";

export interface Size {
  width: number;
  height: number;
}

export interface Point {
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
  reservedBottomInset?: number;
  viewMode: ViewMode;
  zoom: number | null;
}
