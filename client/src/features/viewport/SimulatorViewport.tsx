import type { CSSProperties, ReactNode, Ref } from "react";

import type {
  AccessibilityNode,
  ChromeProfile,
  SimulatorMetadata,
} from "../../api/types";
import { ZoomControls } from "../toolbar/ZoomControls";
import { DeviceChrome } from "./DeviceChrome";
import type { TouchIndicator, ViewMode } from "./types";

interface SimulatorViewportProps {
  accessibilityHoveredId: string | null;
  debugPanel: ReactNode;
  accessibilityPanel: ReactNode;
  accessibilityPickerActive: boolean;
  accessibilityRoots: AccessibilityNode[];
  accessibilitySelectedId: string;
  chromeProfile: ChromeProfile | null;
  chromeLoaded: boolean;
  chromeRequired: boolean;
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
  onChromeButtonEvent: (
    button: string,
    phase: "down" | "up",
    usagePage?: number,
    usage?: number,
  ) => void;
  chromeButtonUrl: (button: string, pressed?: boolean) => string;
  onChromeLoad: () => void;
  onPanPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerUp: () => void;
  onPickerHover: (id: string | null) => void;
  onPickerSelect: (id: string) => void;
  onSimulatorInteraction: () => void;
  onViewportWheel: (event: React.WheelEvent<HTMLElement>) => void;
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
  screenClassName?: string;
  selectedSimulator: SimulatorMetadata | null;
  shellStyle: CSSProperties | null;
  streamBackend: string;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  streamCanvasKey: string;
  statusOverlayLabel: string;
  touchIndicators: TouchIndicator[];
  touchOverlayVisible: boolean;
  viewMode: ViewMode;
  devtoolsPanel: ReactNode;
  zoomDockRef: Ref<HTMLDivElement | null>;
  zoomAnimating: boolean;
}

export function SimulatorViewport({
  accessibilityHoveredId,
  accessibilityPanel,
  accessibilityPickerActive,
  accessibilityRoots,
  accessibilitySelectedId,
  debugPanel,
  chromeProfile,
  chromeLoaded,
  chromeRequired,
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
  onChromeButtonEvent,
  chromeButtonUrl,
  onChromeLoad,
  onPanPointerMove,
  onPanPointerUp,
  onPickerHover,
  onPickerSelect,
  onSimulatorInteraction,
  onViewportWheel,
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
  screenClassName,
  selectedSimulator,
  shellStyle,
  streamBackend,
  streamCanvasRef,
  streamCanvasKey,
  statusOverlayLabel,
  touchIndicators,
  touchOverlayVisible,
  viewMode,
  devtoolsPanel,
  zoomDockRef,
  zoomAnimating,
}: SimulatorViewportProps) {
  const showDeviceLoading = Boolean(
    selectedSimulator?.isBooted &&
    !hasFrame &&
    !isStreamError &&
    chromeRequired &&
    !chromeLoaded,
  );
  const hideDeviceWhileLoading = Boolean(
    selectedSimulator?.isBooted && chromeRequired && !chromeLoaded,
  );
  const showScreenLoading = Boolean(
    selectedSimulator?.isBooted &&
    !hasFrame &&
    !isStreamError &&
    (!chromeRequired || chromeLoaded),
  );

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
        onWheel={onViewportWheel}
        ref={outerCanvasRef}
      >
        {selectedSimulator ? (
          <div
            className={`device-anchor ${zoomAnimating ? "animated" : ""} ${hideDeviceWhileLoading ? "device-anchor-loading" : ""}`}
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
                  chromeProfile={chromeProfile}
                  chromeScreenStyle={chromeScreenStyle}
                  chromeUrl={chromeUrl}
                  chromeButtonUrl={chromeButtonUrl}
                  hasFrame={hasFrame}
                  isBooted={selectedSimulator.isBooted}
                  isLoadingStream={showScreenLoading}
                  isStreamError={isStreamError}
                  onChromeButtonEvent={onChromeButtonEvent}
                  onChromeLoad={onChromeLoad}
                  onPanPointerCancel={onPanPointerUp}
                  onPanPointerMove={onPanPointerMove}
                  onPanPointerUp={onPanPointerUp}
                  onPickerHover={onPickerHover}
                  onPickerSelect={onPickerSelect}
                  onSimulatorInteraction={onSimulatorInteraction}
                  onScreenPointerCancel={onScreenPointerCancel}
                  onScreenPointerDown={onScreenPointerDown}
                  onScreenPointerMove={onScreenPointerMove}
                  onScreenPointerUp={onScreenPointerUp}
                  onStartPanning={onStartPanning}
                  rotationQuarterTurns={rotationQuarterTurns}
                  screenAspect={screenAspect}
                  screenClassName={screenClassName}
                  shellStyle={shellStyle}
                  simulatorName={selectedSimulator.name}
                  streamBackend={streamBackend}
                  streamCanvasRef={streamCanvasRef}
                  streamCanvasKey={streamCanvasKey}
                  statusOverlayLabel={statusOverlayLabel}
                  touchIndicators={touchIndicators}
                  touchOverlayVisible={touchOverlayVisible}
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
        {showDeviceLoading ? (
          <div
            aria-label="Loading simulator"
            className="canvas-loading"
            role="status"
          >
            <span className="loading-spinner" aria-hidden="true" />
          </div>
        ) : null}
        {debugPanel ? <div className="debug-overlay">{debugPanel}</div> : null}
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
      {devtoolsPanel}
    </div>
  );
}
