import type { CSSProperties, Ref } from "react";

import type { AccessibilityNode } from "../../api/types";
import { AccessibilityOverlay } from "../accessibility/AccessibilityOverlay";
import { findAccessibilityItemAtPoint } from "../accessibility/accessibilityTree";
import { normalizedPointerCoordinatesForOrientation } from "../input/gestureMath";

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
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
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
  streamCanvasRef,
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
          streamCanvasRef={streamCanvasRef}
          useChromeProfile
        />
      </div>
    );
  }

  return (
    <div
      className="device-bezel"
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
        chromeScreenStyle={{ aspectRatio: screenAspect }}
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
        streamCanvasRef={streamCanvasRef}
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
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
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
  streamCanvasRef,
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
        ref={streamCanvasRef}
      />
      <AccessibilityOverlay
        hoveredId={accessibilityHoveredId}
        roots={accessibilityRoots}
        selectedId={accessibilitySelectedId}
      />
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
      {isBooted && !hasFrame && !isStreamError ? (
        <div className="screen-overlay">Waiting for first frame…</div>
      ) : null}
      {!isBooted ? (
        <div className="screen-overlay">Boot simulator to start streaming</div>
      ) : null}
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
