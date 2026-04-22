import type { CSSProperties, Ref } from "react";

interface DeviceChromeProps {
  chromeScreenStyle: CSSProperties | null;
  chromeUrl: string;
  hasFrame: boolean;
  isBooted: boolean;
  isStreamError: boolean;
  onPanPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onPanPointerUp: () => void;
  onScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onStartPanning: (event: React.PointerEvent<HTMLElement>) => void;
  screenAspect: string;
  shellStyle: CSSProperties | null;
  simulatorName: string;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  useChromeProfile: boolean;
}

export function DeviceChrome({
  chromeScreenStyle,
  chromeUrl,
  hasFrame,
  isBooted,
  isStreamError,
  onPanPointerCancel,
  onPanPointerMove,
  onPanPointerUp,
  onScreenPointerCancel,
  onScreenPointerDown,
  onScreenPointerMove,
  onScreenPointerUp,
  onStartPanning,
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
          chromeScreenStyle={chromeScreenStyle}
          hasFrame={hasFrame}
          isBooted={isBooted}
          isStreamError={isStreamError}
          onScreenPointerCancel={onScreenPointerCancel}
          onScreenPointerDown={onScreenPointerDown}
          onScreenPointerMove={onScreenPointerMove}
          onScreenPointerUp={onScreenPointerUp}
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
        chromeScreenStyle={{ aspectRatio: screenAspect }}
        hasFrame={hasFrame}
        isBooted={isBooted}
        isStreamError={isStreamError}
        onScreenPointerCancel={onScreenPointerCancel}
        onScreenPointerDown={onScreenPointerDown}
        onScreenPointerMove={onScreenPointerMove}
        onScreenPointerUp={onScreenPointerUp}
        simulatorName={simulatorName}
        streamCanvasRef={streamCanvasRef}
        useChromeProfile={false}
      />
    </div>
  );
}

interface ScreenLayerProps {
  chromeScreenStyle: CSSProperties | null;
  hasFrame: boolean;
  isBooted: boolean;
  isStreamError: boolean;
  onScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onScreenPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  simulatorName: string;
  streamCanvasRef: Ref<HTMLCanvasElement | null>;
  useChromeProfile: boolean;
}

function ScreenLayer({
  chromeScreenStyle,
  hasFrame,
  isBooted,
  isStreamError,
  onScreenPointerCancel,
  onScreenPointerDown,
  onScreenPointerMove,
  onScreenPointerUp,
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
      {isBooted && !hasFrame && !isStreamError ? (
        <div className="screen-overlay">Waiting for first frame…</div>
      ) : null}
      {!isBooted ? (
        <div className="screen-overlay">Boot simulator to start streaming</div>
      ) : null}
    </div>
  );
}
