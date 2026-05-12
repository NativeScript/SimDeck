import { useRef, useState, type CSSProperties, type Ref } from "react";

import type {
  AccessibilityNode,
  ChromeButtonProfile,
  ChromeProfile,
} from "../../api/types";
import { AccessibilityOverlay } from "../accessibility/AccessibilityOverlay";
import { findAccessibilityItemAtPoint } from "../accessibility/accessibilityTree";
import { normalizedPointerCoordinatesForOrientation } from "../input/gestureMath";
import type { TouchIndicator } from "./types";

interface DeviceChromeProps {
  accessibilityHoveredId: string | null;
  accessibilityPickerActive: boolean;
  accessibilityRoots: AccessibilityNode[];
  accessibilitySelectedId: string;
  chromeProfile: ChromeProfile | null;
  chromeScreenStyle: CSSProperties | null;
  chromeUrl: string;
  chromeButtonUrl: (button: string, pressed?: boolean) => string;
  browserFramePending: boolean;
  hasFrame: boolean;
  isBooted: boolean;
  isLoadingStream: boolean;
  isStreamError: boolean;
  onChromeButtonEvent: (
    button: string,
    phase: "down" | "up",
    usagePage?: number,
    usage?: number,
  ) => void;
  onChromeLoad: () => void;
  onPanPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerUp: () => void;
  onPickerHover: (id: string | null) => void;
  onPickerSelect: (id: string) => void;
  onSimulatorInteraction: () => void;
  onScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onStartPanning: (event: React.PointerEvent<HTMLElement>) => void;
  rotationQuarterTurns: number;
  screenAspect: string;
  screenClassName?: string;
  shellStyle: CSSProperties | null;
  simulatorName: string;
  streamBackend: string;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  streamCanvasKey: string;
  streamStatusLabel: string;
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
  chromeProfile,
  chromeScreenStyle,
  chromeUrl,
  chromeButtonUrl,
  browserFramePending,
  hasFrame,
  isBooted,
  isLoadingStream,
  isStreamError,
  onChromeButtonEvent,
  onChromeLoad,
  onPanPointerCancel,
  onPanPointerMove,
  onPanPointerUp,
  onPickerHover,
  onPickerSelect,
  onSimulatorInteraction,
  onScreenPointerCancel,
  onScreenPointerDown,
  onScreenPointerMove,
  onScreenPointerUp,
  onStartPanning,
  rotationQuarterTurns,
  screenAspect,
  screenClassName,
  shellStyle,
  simulatorName,
  streamBackend,
  streamCanvasRef,
  streamCanvasKey,
  streamStatusLabel,
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
        <ChromeButtonOverlay
          chromeButtonUrl={chromeButtonUrl}
          chromeProfile={chromeProfile}
          layer="under"
          onEvent={onChromeButtonEvent}
        />
        <img
          alt=""
          aria-hidden="true"
          className="device-chrome"
          draggable={false}
          onLoad={onChromeLoad}
          src={chromeUrl}
        />
        <ChromeButtonOverlay
          chromeButtonUrl={chromeButtonUrl}
          chromeProfile={chromeProfile}
          layer="over"
          onEvent={onChromeButtonEvent}
        />
        <ScreenLayer
          accessibilityHoveredId={accessibilityHoveredId}
          accessibilityPickerActive={accessibilityPickerActive}
          accessibilityRoots={accessibilityRoots}
          accessibilitySelectedId={accessibilitySelectedId}
          chromeScreenStyle={chromeScreenStyle}
          browserFramePending={browserFramePending}
          hasFrame={hasFrame}
          isBooted={isBooted}
          isLoadingStream={isLoadingStream}
          isStreamError={isStreamError}
          onScreenPointerCancel={onScreenPointerCancel}
          onScreenPointerDown={onScreenPointerDown}
          onScreenPointerMove={onScreenPointerMove}
          onScreenPointerUp={onScreenPointerUp}
          onPickerHover={onPickerHover}
          onPickerSelect={onPickerSelect}
          onSimulatorInteraction={onSimulatorInteraction}
          rotationQuarterTurns={rotationQuarterTurns}
          simulatorName={simulatorName}
          screenClassName={screenClassName}
          streamBackend={streamBackend}
          streamCanvasRef={streamCanvasRef}
          streamCanvasKey={streamCanvasKey}
          streamStatusLabel={streamStatusLabel}
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
        browserFramePending={browserFramePending}
        hasFrame={hasFrame}
        isBooted={isBooted}
        isLoadingStream={isLoadingStream}
        isStreamError={isStreamError}
        onScreenPointerCancel={onScreenPointerCancel}
        onScreenPointerDown={onScreenPointerDown}
        onScreenPointerMove={onScreenPointerMove}
        onScreenPointerUp={onScreenPointerUp}
        onPickerHover={onPickerHover}
        onPickerSelect={onPickerSelect}
        onSimulatorInteraction={onSimulatorInteraction}
        rotationQuarterTurns={rotationQuarterTurns}
        simulatorName={simulatorName}
        screenClassName={screenClassName}
        streamBackend={streamBackend}
        streamCanvasRef={streamCanvasRef}
        streamCanvasKey={streamCanvasKey}
        streamStatusLabel={streamStatusLabel}
        statusOverlayLabel={statusOverlayLabel}
        touchIndicators={touchIndicators}
        touchOverlayVisible={touchOverlayVisible}
        useChromeProfile={false}
      />
    </div>
  );
}

const CHROME_BUTTON_WIRE_NAMES: Record<string, string> = {
  action: "action",
  "digital-crown": "digital-crown",
  home: "home",
  "left-side-button": "left-side-button",
  lock: "power",
  mute: "mute",
  power: "power",
  "side-button": "side-button",
  "volume-down": "volume-down",
  "volume-up": "volume-up",
};

function ChromeButtonOverlay({
  chromeButtonUrl,
  chromeProfile,
  layer,
  onEvent,
}: {
  chromeButtonUrl: (button: string, pressed?: boolean) => string;
  chromeProfile: ChromeProfile | null;
  layer: "under" | "over";
  onEvent: (
    button: string,
    phase: "down" | "up",
    usagePage?: number,
    usage?: number,
  ) => void;
}) {
  const buttons = chromeProfile?.buttons ?? [];
  if (!chromeProfile || buttons.length === 0) {
    return null;
  }

  return (
    <div
      className={`device-chrome-buttons device-chrome-buttons-${layer}`}
      aria-hidden={false}
    >
      {buttons.map((button) => {
        const onTop = Boolean(button.onTop);
        if ((layer === "over") !== onTop) {
          return null;
        }
        const wireName = wireButtonName(button);
        if (!wireName) {
          return null;
        }
        return (
          <ChromeButtonHitTarget
            button={button}
            chromeButtonUrl={chromeButtonUrl}
            key={`${button.name}-${button.x}-${button.y}`}
            onEvent={onEvent}
            totalHeight={chromeProfile.totalHeight}
            totalWidth={chromeProfile.totalWidth}
            wireName={wireName}
          />
        );
      })}
    </div>
  );
}

function ChromeButtonHitTarget({
  button,
  chromeButtonUrl,
  onEvent,
  totalHeight,
  totalWidth,
  wireName,
}: {
  button: ChromeButtonProfile;
  chromeButtonUrl: (button: string, pressed?: boolean) => string;
  onEvent: (
    button: string,
    phase: "down" | "up",
    usagePage?: number,
    usage?: number,
  ) => void;
  totalHeight: number;
  totalWidth: number;
  wireName: string;
}) {
  const pressedRef = useRef(false);
  const [pressed, setPressed] = useState(false);
  const label = button.label || humanizeChromeButtonName(button.name);
  const rolloverDelta = button.rolloverOffset
    ? {
        x: button.rolloverOffset.x - (button.normalOffset?.x ?? 0),
        y: button.rolloverOffset.y - (button.normalOffset?.y ?? 0),
      }
    : { x: 0, y: 0 };
  const imageUrl = chromeButtonUrl(button.name, false);
  const pressedImageUrl = button.imageDownName
    ? chromeButtonUrl(button.name, true)
    : "";
  const downCompositeUnder =
    pressed &&
    pressedImageUrl &&
    button.imageDownDrawMode?.toLowerCase() === "compositeunder";
  const style = {
    height: `${(button.height / totalHeight) * 100}%`,
    left: `${(button.x / totalWidth) * 100}%`,
    top: `${(button.y / totalHeight) * 100}%`,
    width: `${(button.width / totalWidth) * 100}%`,
    "--button-rest-x": "0%",
    "--button-rest-y": "0%",
    "--button-hover-x": `${(rolloverDelta.x / Math.max(button.width, 1)) * 100}%`,
    "--button-hover-y": `${(rolloverDelta.y / Math.max(button.height, 1)) * 100}%`,
    "--button-pressed-x": `${(-rolloverDelta.x / Math.max(button.width, 1)) * 100}%`,
    "--button-pressed-y": `${(-rolloverDelta.y / Math.max(button.height, 1)) * 100}%`,
  } as CSSProperties &
    Record<
      | "--button-rest-x"
      | "--button-rest-y"
      | "--button-hover-x"
      | "--button-hover-y"
      | "--button-pressed-x"
      | "--button-pressed-y",
      string
    >;

  function sendPhase(phase: "down" | "up") {
    onEvent(wireName, phase, button.usagePage, button.usage);
  }

  function endPress() {
    if (!pressedRef.current) {
      return;
    }
    pressedRef.current = false;
    setPressed(false);
    sendPhase("up");
  }

  return (
    <button
      aria-label={label}
      className={`device-chrome-button device-chrome-button-${button.anchor ?? "edge"} ${
        button.onTop ? "device-chrome-button-on-top" : ""
      } ${pressed ? "is-pressed" : ""}`}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
      onPointerCancel={(event) => {
        event.preventDefault();
        event.stopPropagation();
        endPress();
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget.setPointerCapture(event.pointerId);
        pressedRef.current = true;
        setPressed(true);
        sendPhase("down");
      }}
      onPointerUp={(event) => {
        event.preventDefault();
        event.stopPropagation();
        endPress();
      }}
      onLostPointerCapture={endPress}
      style={style}
      title={label}
      type="button"
    >
      {downCompositeUnder ? (
        <img
          alt=""
          aria-hidden="true"
          className="device-chrome-button-image-under"
          draggable={false}
          src={pressedImageUrl}
        />
      ) : null}
      <img
        alt=""
        aria-hidden="true"
        className="device-chrome-button-image"
        draggable={false}
        src={
          pressed && pressedImageUrl && !downCompositeUnder
            ? pressedImageUrl
            : imageUrl
        }
      />
      {!pressed && pressedImageUrl ? (
        <img
          alt=""
          aria-hidden="true"
          className="device-chrome-button-preload"
          draggable={false}
          src={pressedImageUrl}
        />
      ) : null}
    </button>
  );
}

function wireButtonName(button: ChromeButtonProfile): string | null {
  return CHROME_BUTTON_WIRE_NAMES[button.name.toLowerCase()] ?? null;
}

function humanizeChromeButtonName(name: string) {
  return name
    .split(/[-_]/)
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

interface ScreenLayerProps {
  accessibilityHoveredId: string | null;
  accessibilityPickerActive: boolean;
  accessibilityRoots: AccessibilityNode[];
  accessibilitySelectedId: string;
  chromeScreenStyle: CSSProperties | null;
  browserFramePending: boolean;
  hasFrame: boolean;
  isBooted: boolean;
  isLoadingStream: boolean;
  isStreamError: boolean;
  onScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onPickerHover: (id: string | null) => void;
  onPickerSelect: (id: string) => void;
  onSimulatorInteraction: () => void;
  rotationQuarterTurns: number;
  screenClassName?: string;
  simulatorName: string;
  streamBackend: string;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  streamCanvasKey: string;
  streamStatusLabel: string;
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
  browserFramePending,
  hasFrame,
  isBooted,
  isLoadingStream,
  isStreamError,
  onScreenPointerCancel,
  onScreenPointerDown,
  onScreenPointerMove,
  onScreenPointerUp,
  onPickerHover,
  onPickerSelect,
  onSimulatorInteraction,
  rotationQuarterTurns,
  screenClassName,
  simulatorName,
  streamBackend,
  streamCanvasRef,
  streamCanvasKey,
  streamStatusLabel,
  statusOverlayLabel,
  touchIndicators,
  touchOverlayVisible,
  useChromeProfile,
}: ScreenLayerProps) {
  return (
    <div
      className={[
        "device-screen",
        useChromeProfile ? "chrome-screen" : "",
        screenClassName ?? "",
      ]
        .filter(Boolean)
        .join(" ")}
      onPointerCancel={onScreenPointerCancel}
      onPointerDown={(event) => {
        onSimulatorInteraction();
        onScreenPointerDown(event);
      }}
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
      <div
        aria-live="polite"
        className="stream-status-agent"
        data-testid="stream-status"
        role="status"
      >
        {streamStatusLabel}
      </div>
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
      {isLoadingStream && !statusOverlayLabel ? (
        <div
          aria-label="Loading simulator"
          className="screen-overlay screen-loading"
          role="status"
        >
          <span className="loading-spinner" aria-hidden="true" />
        </div>
      ) : null}
      {isBooted &&
      !hasFrame &&
      !isStreamError &&
      !isLoadingStream &&
      !statusOverlayLabel ? (
        <div className="screen-overlay">
          {browserFramePending
            ? "Stream connected, browser frame pending"
            : "Waiting for first frame..."}
        </div>
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
