import type { ViewMode } from "../viewport/types";

interface ZoomControlsProps {
  effectiveZoom: number;
  onZoomActual: () => void;
  onZoomCenter: () => void;
  onZoomFit: () => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  variant?: "toolbar" | "floating";
  viewMode: ViewMode;
}

export function ZoomControls({
  effectiveZoom,
  onZoomActual,
  onZoomCenter,
  onZoomFit,
  onZoomIn,
  onZoomOut,
  variant = "toolbar",
  viewMode,
}: ZoomControlsProps) {
  return (
    <div
      className={`zoom-controls ${variant === "floating" ? "zoom-controls-floating" : ""}`}
    >
      <button className="tbtn" onClick={onZoomOut} title="Zoom out">
        &minus;
      </button>
      <span className="zoom-label">{Math.round(effectiveZoom * 100)}%</span>
      <button className="tbtn" onClick={onZoomIn} title="Zoom in">
        +
      </button>
      <button
        className={`tbtn ${viewMode === "center" ? "active" : ""}`}
        onClick={onZoomCenter}
        title="Center viewport"
      >
        Center
      </button>
      <button
        className={`tbtn ${viewMode === "fit" ? "active" : ""}`}
        onClick={onZoomFit}
        title="Fit to canvas"
      >
        Fit
      </button>
      <button className="tbtn" onClick={onZoomActual} title="Actual size">
        1:1
      </button>
    </div>
  );
}
