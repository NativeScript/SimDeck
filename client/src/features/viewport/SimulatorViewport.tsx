import type { CSSProperties, Ref } from "react";

import type { ChromeProfile, SimulatorMetadata } from "../../api/types";
import { ZoomControls } from "../toolbar/ZoomControls";
import { DeviceChrome } from "./DeviceChrome";
import type { ViewMode } from "./types";

interface SimulatorViewportProps {
  chromeProfile: ChromeProfile | null;
  chromeScreenStyle: CSSProperties | null;
  chromeUrl: string;
  deviceTransform: string;
  effectiveZoom: number;
  fitScale: number;
  hasFrame: boolean;
  isLoading: boolean;
  isStreamError: boolean;
  isPanning: boolean;
  onPanPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerUp: () => void;
  onScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onStartPanning: (event: React.PointerEvent<HTMLElement>) => void;
  onZoomActual: () => void;
  onZoomCenter: () => void;
  onZoomFit: () => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  outerCanvasRef: Ref<HTMLDivElement | null>;
  screenAspect: string;
  selectedSimulator: SimulatorMetadata | null;
  shellStyle: CSSProperties | null;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  viewMode: ViewMode;
  zoomDockRef: Ref<HTMLDivElement | null>;
  zoomAnimating: boolean;
}

export function SimulatorViewport({
  chromeProfile,
  chromeScreenStyle,
  chromeUrl,
  deviceTransform,
  effectiveZoom,
  fitScale,
  hasFrame,
  isLoading,
  isStreamError,
  isPanning,
  onPanPointerMove,
  onPanPointerUp,
  onScreenPointerCancel,
  onScreenPointerDown,
  onScreenPointerMove,
  onScreenPointerUp,
  onStartPanning,
  onZoomActual,
  onZoomCenter,
  onZoomFit,
  onZoomIn,
  onZoomOut,
  outerCanvasRef,
  screenAspect,
  selectedSimulator,
  shellStyle,
  streamCanvasRef,
  viewMode,
  zoomDockRef,
  zoomAnimating,
}: SimulatorViewportProps) {
  return (
    <div className="main">
      <div
        className={`canvas ${effectiveZoom > fitScale + 0.001 ? "pan-enabled" : ""} ${isPanning ? "panning" : ""}`}
        onContextMenu={(event) => event.preventDefault()}
        onPointerCancel={onPanPointerUp}
        onPointerDown={(event) => {
          if (event.target !== event.currentTarget) {
            return;
          }
          onStartPanning(event);
        }}
        onPointerMove={onPanPointerMove}
        onPointerUp={onPanPointerUp}
        ref={outerCanvasRef}
      >
        {selectedSimulator ? (
          <div
            className={`device-anchor ${zoomAnimating ? "animated" : ""}`}
            style={{ transform: deviceTransform }}
          >
            <DeviceChrome
              chromeScreenStyle={chromeScreenStyle}
              chromeUrl={chromeUrl}
              hasFrame={hasFrame}
              isBooted={selectedSimulator.isBooted}
              isStreamError={isStreamError}
              onPanPointerCancel={onPanPointerUp}
              onPanPointerMove={onPanPointerMove}
              onPanPointerUp={onPanPointerUp}
              onScreenPointerCancel={onScreenPointerCancel}
              onScreenPointerDown={onScreenPointerDown}
              onScreenPointerMove={onScreenPointerMove}
              onScreenPointerUp={onScreenPointerUp}
              onStartPanning={onStartPanning}
              screenAspect={screenAspect}
              shellStyle={shellStyle}
              simulatorName={selectedSimulator.name}
              streamCanvasRef={streamCanvasRef}
              useChromeProfile={Boolean(chromeProfile)}
            />
          </div>
        ) : (
          <div className="canvas-empty">
            <p>{isLoading ? "Loading simulators…" : "Select a simulator"}</p>
          </div>
        )}
        {selectedSimulator ? (
          <div className="viewport-zoom-dock" ref={zoomDockRef}>
            <ZoomControls
              effectiveZoom={effectiveZoom}
              onZoomActual={onZoomActual}
              onZoomCenter={onZoomCenter}
              onZoomFit={onZoomFit}
              onZoomIn={onZoomIn}
              onZoomOut={onZoomOut}
              variant="floating"
              viewMode={viewMode}
            />
          </div>
        ) : null}
      </div>
    </div>
  );
}
