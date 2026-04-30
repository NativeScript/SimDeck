import type { CSSProperties, Ref } from "react";

import type { AccessibilityNode } from "../../api/types";
import { AccessibilityOverlay } from "../accessibility/AccessibilityOverlay";
import { findAccessibilityItemAtPoint } from "../accessibility/accessibilityTree";
import { normalizedPointerCoordinatesForOrientation } from "../input/gestureMath";
import type { TouchIndicator } from "./types";

interface DeviceChromeProps {
  accessibilityHoveredId: string | null;
  accessibilityPickerActive: boolean;
  accessibilityRoots: AccessibilityNode[];
  accessibilitySelectedId: string;
  chromeScreenStyle: CSSProperties | null;
  chromeUrl: string;
  hasFrame: boolean;
  isBooted: boolean;
  isStreamError: boolean;
  onChromeLoad: () => void;
  onPanPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerUp: () => void;
  onPickerHover: (id: string | null) => void;
  onPickerSelect: (id: string) => void;
  onScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onStartPanning: (event: React.PointerEvent<HTMLElement>) => void;
  rotationQuarterTurns: number;
  screenAspect: string;
  shellStyle: CSSProperties | null;
  simulatorName: string;
  streamBackend: string;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  streamCanvasKey: string;
  statusOverlayLabel: string;
  touchIndicators: TouchIndicator[];
  touchOverlayVisible: boolean;
  useChromeProfile: boolean;
}

export function DeviceChrome({
  accessibilityHoveredId,
  accessibilityPickerActive,
  accessibilityRoots,
  accessibilitySelectedId,
  chromeScreenStyle,
  chromeUrl,
  hasFrame,
  isBooted,
  isStreamError,
  onChromeLoad,
  onPanPointerCancel,
  onPanPointerMove,
  onPanPointerUp,
  onPickerHover,
  onPickerSelect,
  onScreenPointerCancel,
  onScreenPointerDown,
  onScreenPointerMove,
  onScreenPointerUp,
  onStartPanning,
  rotationQuarterTurns,
  screenAspect,
  shellStyle,
  simulatorName,
  streamBackend,
  streamCanvasRef,
  streamCanvasKey,
  statusOverlayLabel,
  touchIndicators,
  touchOverlayVisible,
  useChromeProfile,
}: DeviceChromeProps) {
  if (useChromeProfile) {
    return (
      <div
        className="device-shell"
        onPointerCancel={onPanPointerUp}
        onPointerDown={onStartPanning}
        onPointerMove={onPanPointerMove}
        onPointerUp={onPanPointerUp}
        style={shellStyle ?? undefined}
      >
        <img
          alt=""
          aria-hidden="true"
          className="device-chrome"
          draggable={false}
          onLoad={onChromeLoad}
          src={chromeUrl}
        />
        <ScreenLayer
          accessibilityHoveredId={accessibilityHoveredId}
          accessibilityPickerActive={accessibilityPickerActive}
          accessibilityRoots={accessibilityRoots}
          accessibilitySelectedId={accessibilitySelectedId}
          chromeScreenStyle={chromeScreenStyle}
          hasFrame={hasFrame}
          isBooted={isBooted}
          isStreamError={isStreamError}
          onScreenPointerCancel={onScreenPointerCancel}
          onScreenPointerDown={onScreenPointerDown}
          onScreenPointerMove={onScreenPointerMove}
          onScreenPointerUp={onScreenPointerUp}
          onPickerHover={onPickerHover}
          onPickerSelect={onPickerSelect}
          rotationQuarterTurns={rotationQuarterTurns}
          simulatorName={simulatorName}
          streamBackend={streamBackend}
          streamCanvasRef={streamCanvasRef}
          streamCanvasKey={streamCanvasKey}
          statusOverlayLabel={statusOverlayLabel}
          touchIndicators={touchIndicators}
          touchOverlayVisible={touchOverlayVisible}
          useChromeProfile
        />
      </div>
    );
  }

  return (
    <div
      className="device-shell screen-only-shell"
      onPointerCancel={onPanPointerCancel}
      onPointerDown={onStartPanning}
      onPointerMove={onPanPointerMove}
      onPointerUp={onPanPointerUp}
    >
      <ScreenLayer
        accessibilityHoveredId={accessibilityHoveredId}
        accessibilityPickerActive={accessibilityPickerActive}
        accessibilityRoots={accessibilityRoots}
        accessibilitySelectedId={accessibilitySelectedId}
        chromeScreenStyle={{
          aspectRatio: screenAspect,
          ...(chromeScreenStyle ?? {}),
        }}
        hasFrame={hasFrame}
        isBooted={isBooted}
        isStreamError={isStreamError}
        onScreenPointerCancel={onScreenPointerCancel}
        onScreenPointerDown={onScreenPointerDown}
        onScreenPointerMove={onScreenPointerMove}
        onScreenPointerUp={onScreenPointerUp}
        onPickerHover={onPickerHover}
        onPickerSelect={onPickerSelect}
        rotationQuarterTurns={rotationQuarterTurns}
        simulatorName={simulatorName}
        streamBackend={streamBackend}
        streamCanvasRef={streamCanvasRef}
        streamCanvasKey={streamCanvasKey}
        statusOverlayLabel={statusOverlayLabel}
        touchIndicators={touchIndicators}
        touchOverlayVisible={touchOverlayVisible}
        useChromeProfile={false}
      />
    </div>
  );
}

interface ScreenLayerProps {
  accessibilityHoveredId: string | null;
  accessibilityPickerActive: boolean;
  accessibilityRoots: AccessibilityNode[];
  accessibilitySelectedId: string;
  chromeScreenStyle: CSSProperties | null;
  hasFrame: boolean;
  isBooted: boolean;
  isStreamError: boolean;
  onScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onPickerHover: (id: string | null) => void;
  onPickerSelect: (id: string) => void;
  rotationQuarterTurns: number;
  simulatorName: string;
  streamBackend: string;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  streamCanvasKey: string;
  statusOverlayLabel: string;
  touchIndicators: TouchIndicator[];
  touchOverlayVisible: boolean;
  useChromeProfile: boolean;
}

function ScreenLayer({
  accessibilityHoveredId,
  accessibilityPickerActive,
  accessibilityRoots,
  accessibilitySelectedId,
  chromeScreenStyle,
  hasFrame,
  isBooted,
  isStreamError,
  onScreenPointerCancel,
  onScreenPointerDown,
  onScreenPointerMove,
  onScreenPointerUp,
  onPickerHover,
  onPickerSelect,
  rotationQuarterTurns,
  simulatorName,
  streamBackend,
  streamCanvasRef,
  streamCanvasKey,
  statusOverlayLabel,
  touchIndicators,
  touchOverlayVisible,
  useChromeProfile,
}: ScreenLayerProps) {
  return (
    <div
      className={`device-screen ${useChromeProfile ? "chrome-screen" : ""}`}
      onPointerCancel={onScreenPointerCancel}
      onPointerDown={onScreenPointerDown}
      onPointerMove={onScreenPointerMove}
      onPointerUp={onScreenPointerUp}
      style={chromeScreenStyle ?? undefined}
    >
      <canvas
        aria-label={`${simulatorName} stream`}
        className="stream-canvas"
        data-stream-backend={streamBackend}
        key={streamCanvasKey}
        ref={streamCanvasRef}
      />
      <AccessibilityOverlay
        hoveredId={accessibilityHoveredId}
        roots={accessibilityRoots}
        selectedId={accessibilitySelectedId}
      />
      {touchOverlayVisible ? (
        <TouchInteractionOverlay indicators={touchIndicators} />
      ) : null}
      {accessibilityPickerActive ? (
        <div
          className="accessibility-picker-layer"
          onPointerDown={(event) => {
            event.preventDefault();
            event.stopPropagation();
          }}
          onPointerLeave={() => onPickerHover(null)}
          onPointerMove={(event) => {
            event.preventDefault();
            event.stopPropagation();
            onPickerHover(
              hitTestAccessibilityId(
                event,
                accessibilityRoots,
                rotationQuarterTurns,
              ),
            );
          }}
          onPointerUp={(event) => {
            event.preventDefault();
            event.stopPropagation();
            const id = hitTestAccessibilityId(
              event,
              accessibilityRoots,
              rotationQuarterTurns,
            );
            if (id) {
              onPickerSelect(id);
            }
          }}
        />
      ) : null}
      {statusOverlayLabel ? (
        <div className="screen-overlay">{statusOverlayLabel}</div>
      ) : null}
      {isBooted && !hasFrame && !isStreamError && !statusOverlayLabel ? (
        <div className="screen-overlay">Waiting for first frame…</div>
      ) : null}
      {!isBooted && !statusOverlayLabel ? (
        <div className="screen-overlay">Boot simulator to start streaming</div>
      ) : null}
    </div>
  );
}

function TouchInteractionOverlay({
  indicators,
}: {
  indicators: TouchIndicator[];
}) {
  return (
    <div className="touch-interaction-overlay" aria-hidden="true">
      {indicators.map((indicator) => (
        <span
          className={`touch-indicator touch-indicator-${indicator.phase}`}
          key={indicator.id}
          style={{
            left: `${indicator.x * 100}%`,
            top: `${indicator.y * 100}%`,
          }}
        />
      ))}
    </div>
  );
}

function hitTestAccessibilityId(
  event: React.PointerEvent<HTMLElement>,
  roots: AccessibilityNode[],
  rotationQuarterTurns: number,
): string | null {
  const point = normalizedPointerCoordinatesForOrientation(
    event,
    rotationQuarterTurns,
  );
  if (!point) {
    return null;
  }
  return findAccessibilityItemAtPoint(roots, point)?.id ?? null;
}
