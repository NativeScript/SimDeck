import type { CSSProperties, ReactNode, Ref } from "react";

import type {
  AccessibilityNode,
  ChromeProfile,
  SimulatorMetadata,
} from "../../api/types";
import { ZoomControls } from "../toolbar/ZoomControls";
import { DeviceChrome } from "./DeviceChrome";
import type { ViewMode } from "./types";

interface SimulatorViewportProps {
  accessibilityHoveredId: string | null;
  accessibilityPanel: ReactNode;
  accessibilityPickerActive: boolean;
  accessibilityRoots: AccessibilityNode[];
  accessibilitySelectedId: string;
  chromeProfile: ChromeProfile | null;
  chromeScreenStyle: CSSProperties | null;
  chromeUrl: string;
  deviceFrameStyle: CSSProperties;
  devicePresentationStyle: CSSProperties;
  deviceTransform: string;
  effectiveZoom: number;
  fitScale: number;
  hasFrame: boolean;
  isLoading: boolean;
  isStreamError: boolean;
  isPanning: boolean;
  onPanPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerUp: () => void;
  onPickerHover: (id: string | null) => void;
  onPickerSelect: (id: string) => void;
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
  rotationQuarterTurns: number;
  screenAspect: string;
  selectedSimulator: SimulatorMetadata | null;
  shellStyle: CSSProperties | null;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  viewMode: ViewMode;
  zoomDockRef: Ref<HTMLDivElement | null>;
  zoomAnimating: boolean;
}

export function SimulatorViewport({
  accessibilityHoveredId,
  accessibilityPanel,
  accessibilityPickerActive,
  accessibilityRoots,
  accessibilitySelectedId,
  chromeProfile,
  chromeScreenStyle,
  chromeUrl,
  deviceFrameStyle,
  devicePresentationStyle,
  deviceTransform,
  effectiveZoom,
  fitScale,
  hasFrame,
  isLoading,
  isStreamError,
  isPanning,
  onPanPointerMove,
  onPanPointerUp,
  onPickerHover,
  onPickerSelect,
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
  rotationQuarterTurns,
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
      {accessibilityPanel}
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
            <div className="device-frame" style={deviceFrameStyle}>
              <div
                className="device-presentation"
                style={devicePresentationStyle}
              >
                <DeviceChrome
                  accessibilityHoveredId={accessibilityHoveredId}
                  accessibilityPickerActive={accessibilityPickerActive}
                  accessibilityRoots={accessibilityRoots}
                  accessibilitySelectedId={accessibilitySelectedId}
                  chromeScreenStyle={chromeScreenStyle}
                  chromeUrl={chromeUrl}
                  hasFrame={hasFrame}
                  isBooted={selectedSimulator.isBooted}
                  isStreamError={isStreamError}
                  onPanPointerCancel={onPanPointerUp}
                  onPanPointerMove={onPanPointerMove}
                  onPanPointerUp={onPanPointerUp}
                  onPickerHover={onPickerHover}
                  onPickerSelect={onPickerSelect}
                  onScreenPointerCancel={onScreenPointerCancel}
                  onScreenPointerDown={onScreenPointerDown}
                  onScreenPointerMove={onScreenPointerMove}
                  onScreenPointerUp={onScreenPointerUp}
                  onStartPanning={onStartPanning}
                  rotationQuarterTurns={rotationQuarterTurns}
                  screenAspect={screenAspect}
                  shellStyle={shellStyle}
                  simulatorName={selectedSimulator.name}
                  streamCanvasRef={streamCanvasRef}
                  useChromeProfile={Boolean(chromeProfile)}
                />
              </div>
            </div>
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
