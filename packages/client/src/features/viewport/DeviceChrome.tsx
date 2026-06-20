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
  accessibilitySkeletonVisible: boolean;
  chromeProfile: ChromeProfile | null;
  chromeButtonsRenderedInChrome: boolean;
  chromeScreenBackingStyle: CSSProperties | null;
  chromeScreenStyle: CSSProperties | null;
  chromeUrl: string;
  chromeButtonUrl: (button: string, pressed?: boolean) => string;
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
  onBottomBezelPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onBottomBezelPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onBottomBezelPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onBottomBezelPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onBottomBezelTouchCancel: (event: React.TouchEvent<HTMLElement>) => void;
  onBottomBezelTouchEnd: (event: React.TouchEvent<HTMLElement>) => void;
  onBottomBezelTouchMove: (event: React.TouchEvent<HTMLElement>) => void;
  onBottomBezelTouchStart: (event: React.TouchEvent<HTMLElement>) => void;
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
  onScreenTouchCancel: (event: React.TouchEvent<HTMLElement>) => void;
  onScreenTouchEnd: (event: React.TouchEvent<HTMLElement>) => void;
  onScreenTouchMove: (event: React.TouchEvent<HTMLElement>) => void;
  onScreenTouchStart: (event: React.TouchEvent<HTMLElement>) => void;
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
  accessibilitySkeletonVisible,
  chromeProfile,
  chromeButtonsRenderedInChrome,
  chromeScreenBackingStyle,
  chromeScreenStyle,
  chromeUrl,
  chromeButtonUrl,
  hasFrame,
  isBooted,
  isLoadingStream,
  isStreamError,
  onChromeButtonEvent,
  onBottomBezelPointerCancel,
  onBottomBezelPointerDown,
  onBottomBezelPointerMove,
  onBottomBezelPointerUp,
  onBottomBezelTouchCancel,
  onBottomBezelTouchEnd,
  onBottomBezelTouchMove,
  onBottomBezelTouchStart,
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
  onScreenTouchCancel,
  onScreenTouchEnd,
  onScreenTouchMove,
  onScreenTouchStart,
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
          renderImages={!chromeButtonsRenderedInChrome}
        />
        <img
          alt=""
          aria-hidden="true"
          className="device-chrome"
          draggable={false}
          src={chromeUrl}
        />
        {chromeScreenBackingStyle ? (
          <div
            aria-hidden="true"
            className="device-screen-backing"
            style={chromeScreenBackingStyle}
          />
        ) : null}
        <ChromeButtonOverlay
          chromeButtonUrl={chromeButtonUrl}
          chromeProfile={chromeProfile}
          layer="over"
          onEvent={onChromeButtonEvent}
          renderImages={!chromeButtonsRenderedInChrome}
        />
        <BottomEdgeCatcher
          chromeScreenStyle={chromeScreenStyle}
          onPointerCancel={onBottomBezelPointerCancel}
          onPointerDown={onBottomBezelPointerDown}
          onPointerMove={onBottomBezelPointerMove}
          onPointerUp={onBottomBezelPointerUp}
          onSimulatorInteraction={onSimulatorInteraction}
          onTouchCancel={onBottomBezelTouchCancel}
          onTouchEnd={onBottomBezelTouchEnd}
          onTouchMove={onBottomBezelTouchMove}
          onTouchStart={onBottomBezelTouchStart}
        />
        <ScreenLayer
          accessibilityHoveredId={accessibilityHoveredId}
          accessibilityPickerActive={accessibilityPickerActive}
          accessibilityRoots={accessibilityRoots}
          accessibilitySelectedId={accessibilitySelectedId}
          accessibilitySkeletonVisible={accessibilitySkeletonVisible}
          chromeScreenStyle={chromeScreenStyle}
          hasFrame={hasFrame}
          isBooted={isBooted}
          isLoadingStream={isLoadingStream}
          isStreamError={isStreamError}
          onScreenPointerCancel={onScreenPointerCancel}
          onScreenPointerDown={onScreenPointerDown}
          onScreenPointerMove={onScreenPointerMove}
          onScreenPointerUp={onScreenPointerUp}
          onScreenTouchCancel={onScreenTouchCancel}
          onScreenTouchEnd={onScreenTouchEnd}
          onScreenTouchMove={onScreenTouchMove}
          onScreenTouchStart={onScreenTouchStart}
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
        accessibilitySkeletonVisible={accessibilitySkeletonVisible}
        chromeScreenStyle={{
          aspectRatio: screenAspect,
          ...(chromeScreenStyle ?? {}),
        }}
        hasFrame={hasFrame}
        isBooted={isBooted}
        isLoadingStream={isLoadingStream}
        isStreamError={isStreamError}
        onScreenPointerCancel={onScreenPointerCancel}
        onScreenPointerDown={onScreenPointerDown}
        onScreenPointerMove={onScreenPointerMove}
        onScreenPointerUp={onScreenPointerUp}
        onScreenTouchCancel={onScreenTouchCancel}
        onScreenTouchEnd={onScreenTouchEnd}
        onScreenTouchMove={onScreenTouchMove}
        onScreenTouchStart={onScreenTouchStart}
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

function screenEdgeCatcherStyle(
  chromeScreenStyle: CSSProperties | null,
): CSSProperties | null {
  const left =
    typeof chromeScreenStyle?.left === "string" ? chromeScreenStyle.left : null;
  const top =
    typeof chromeScreenStyle?.top === "string" ? chromeScreenStyle.top : null;
  const width =
    typeof chromeScreenStyle?.width === "string"
      ? chromeScreenStyle.width
      : null;
  const height =
    typeof chromeScreenStyle?.height === "string"
      ? chromeScreenStyle.height
      : null;
  if (!left || !top || !width || !height) {
    return null;
  }
  return {
    left,
    top: `calc(${top} + ${height} - 2px)`,
    width,
  };
}

function BottomEdgeCatcher({
  chromeScreenStyle,
  onPointerCancel,
  onPointerDown,
  onPointerMove,
  onPointerUp,
  onSimulatorInteraction,
  onTouchCancel,
  onTouchEnd,
  onTouchMove,
  onTouchStart,
}: {
  chromeScreenStyle: CSSProperties | null;
  onPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onSimulatorInteraction: () => void;
  onTouchCancel: (event: React.TouchEvent<HTMLElement>) => void;
  onTouchEnd: (event: React.TouchEvent<HTMLElement>) => void;
  onTouchMove: (event: React.TouchEvent<HTMLElement>) => void;
  onTouchStart: (event: React.TouchEvent<HTMLElement>) => void;
}) {
  const style = screenEdgeCatcherStyle(chromeScreenStyle);
  if (!style) {
    return null;
  }
  return (
    <div
      aria-hidden="true"
      className="device-bottom-edge-catcher"
      onPointerCancel={onPointerCancel}
      onPointerDown={(event) => {
        onSimulatorInteraction();
        onPointerDown(event);
      }}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onTouchCancel={onTouchCancel}
      onTouchEnd={onTouchEnd}
      onTouchMove={onTouchMove}
      onTouchStart={(event) => {
        onSimulatorInteraction();
        onTouchStart(event);
      }}
      style={style}
    />
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
const CHROME_BUTTON_REST_INSET_RATIO = 0.5;
const CHROME_BUTTON_PRESSED_INSET_RATIO = 0.85;

export function chromeButtonMotionVariables(button: ChromeButtonProfile) {
  const normalOffset = button.normalOffset ?? { x: 0, y: 0 };
  const rolloverOffset = button.rolloverOffset ?? normalOffset;
  const inwardDelta = {
    x: normalOffset.x - rolloverOffset.x,
    y: normalOffset.y - rolloverOffset.y,
  };
  const restOffset = {
    x: inwardDelta.x * CHROME_BUTTON_REST_INSET_RATIO,
    y: inwardDelta.y * CHROME_BUTTON_REST_INSET_RATIO,
  };
  const pressedOffset = {
    x: inwardDelta.x * CHROME_BUTTON_PRESSED_INSET_RATIO,
    y: inwardDelta.y * CHROME_BUTTON_PRESSED_INSET_RATIO,
  };
  const width = Math.max(button.width, 1);
  const height = Math.max(button.height, 1);

  return {
    "--button-rest-x": `${(restOffset.x / width) * 100}%`,
    "--button-rest-y": `${(restOffset.y / height) * 100}%`,
    "--button-hover-x": "0%",
    "--button-hover-y": "0%",
    "--button-pressed-x": `${(pressedOffset.x / width) * 100}%`,
    "--button-pressed-y": `${(pressedOffset.y / height) * 100}%`,
  } as Record<
    | "--button-rest-x"
    | "--button-rest-y"
    | "--button-hover-x"
    | "--button-hover-y"
    | "--button-pressed-x"
    | "--button-pressed-y",
    string
  >;
}

function ChromeButtonOverlay({
  chromeButtonUrl,
  chromeProfile,
  layer,
  onEvent,
  renderImages,
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
  renderImages: boolean;
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
            renderImages={renderImages}
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
  renderImages,
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
  renderImages: boolean;
  totalHeight: number;
  totalWidth: number;
  wireName: string;
}) {
  const pressedRef = useRef(false);
  const [pressed, setPressed] = useState(false);
  const label = button.label || humanizeChromeButtonName(button.name);
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
    ...chromeButtonMotionVariables(button),
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
      {renderImages && downCompositeUnder ? (
        <img
          alt=""
          aria-hidden="true"
          className="device-chrome-button-image-under"
          draggable={false}
          src={pressedImageUrl}
        />
      ) : null}
      {renderImages ? (
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
      ) : null}
      {renderImages && !pressed && pressedImageUrl ? (
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
  accessibilitySkeletonVisible: boolean;
  chromeScreenStyle: CSSProperties | null;
  hasFrame: boolean;
  isBooted: boolean;
  isLoadingStream: boolean;
  isStreamError: boolean;
  onScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenTouchCancel: (event: React.TouchEvent<HTMLElement>) => void;
  onScreenTouchEnd: (event: React.TouchEvent<HTMLElement>) => void;
  onScreenTouchMove: (event: React.TouchEvent<HTMLElement>) => void;
  onScreenTouchStart: (event: React.TouchEvent<HTMLElement>) => void;
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
  accessibilitySkeletonVisible,
  chromeScreenStyle,
  hasFrame,
  isBooted,
  isLoadingStream,
  isStreamError,
  onScreenPointerCancel,
  onScreenPointerDown,
  onScreenPointerMove,
  onScreenPointerUp,
  onScreenTouchCancel,
  onScreenTouchEnd,
  onScreenTouchMove,
  onScreenTouchStart,
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
      onTouchCancel={onScreenTouchCancel}
      onTouchEnd={onScreenTouchEnd}
      onTouchMove={onScreenTouchMove}
      onTouchStart={(event) => {
        onSimulatorInteraction();
        onScreenTouchStart(event);
      }}
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
        skeletonVisible={accessibilitySkeletonVisible}
      />
      {touchOverlayVisible ? (
        <TouchInteractionOverlay
          indicators={touchIndicators.filter(
            (indicator) => (indicator.space ?? "screen") === "screen",
          )}
        />
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
        <div className="screen-overlay">Waiting for first frame...</div>
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
