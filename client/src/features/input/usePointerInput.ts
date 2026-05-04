import { useRef, useState } from "react";

import type { ChromeProfile, TouchPhase } from "../../api/types";
import { normalizedPointerCoordinatesForOrientation } from "./gestureMath";
import { clampPan } from "../viewport/viewportMath";
import type { Point, Size } from "../viewport/types";

interface UsePointerInputOptions {
  canvasSize: Size | null;
  chromeProfile: ChromeProfile | null;
  deviceNaturalSize: Size | null;
  effectiveZoom: number;
  fitScale: number;
  isBooted: boolean;
  pan: Point;
  rotationQuarterTurns: number;
  setPan: React.Dispatch<React.SetStateAction<Point>>;
  onTouch: (phase: TouchPhase, coords: Point) => void;
  onTouchPreview?: (phase: TouchPhase, coords: Point) => void;
}

export function usePointerInput({
  canvasSize,
  chromeProfile,
  deviceNaturalSize,
  effectiveZoom,
  fitScale,
  isBooted,
  pan,
  rotationQuarterTurns,
  setPan,
  onTouch,
  onTouchPreview,
}: UsePointerInputOptions) {
  const activePointerRef = useRef<number | null>(null);
  const panningRef = useRef<{
    startX: number;
    startY: number;
    startPanX: number;
    startPanY: number;
  } | null>(null);
  const [isPanning, setIsPanning] = useState(false);

  function startPanning(event: React.PointerEvent<HTMLElement>) {
    if (event.pointerType !== "mouse") {
      return;
    }
    if (event.button !== 0 && event.button !== 1) {
      return;
    }
    if (effectiveZoom <= fitScale + 0.001) {
      return;
    }

    panningRef.current = {
      startX: event.clientX,
      startY: event.clientY,
      startPanX: pan.x,
      startPanY: pan.y,
    };
    setIsPanning(true);
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function handlePanPointerMove(event: React.PointerEvent<HTMLElement>) {
    if (!panningRef.current) {
      return;
    }

    setPan(
      clampPan(
        {
          x:
            panningRef.current.startPanX +
            (event.clientX - panningRef.current.startX),
          y:
            panningRef.current.startPanY +
            (event.clientY - panningRef.current.startY),
        },
        effectiveZoom,
        canvasSize,
        deviceNaturalSize,
        chromeProfile,
        rotationQuarterTurns,
      ),
    );
  }

  function handlePanPointerUp() {
    panningRef.current = null;
    setIsPanning(false);
  }

  function handleScreenPointerDown(event: React.PointerEvent<HTMLElement>) {
    if (event.button !== 0 || !isBooted) {
      return;
    }
    event.stopPropagation();
    const coords = normalizedPointerCoordinatesForOrientation(
      event,
      rotationQuarterTurns,
    );
    if (!coords) {
      return;
    }
    activePointerRef.current = event.pointerId;
    event.currentTarget.setPointerCapture(event.pointerId);
    onTouchPreview?.("began", coords);
    onTouch("began", coords);
  }

  function handleScreenPointerMove(event: React.PointerEvent<HTMLElement>) {
    event.stopPropagation();
    if (activePointerRef.current !== event.pointerId) {
      return;
    }
    const coords = normalizedPointerCoordinatesForOrientation(
      event,
      rotationQuarterTurns,
    );
    if (coords) {
      onTouchPreview?.("moved", coords);
      onTouch("moved", coords);
    }
  }

  function finishTouch(
    event: React.PointerEvent<HTMLElement>,
    phase: Exclude<TouchPhase, "moved" | "began">,
  ) {
    event.stopPropagation();
    if (activePointerRef.current !== event.pointerId) {
      return;
    }
    activePointerRef.current = null;
    const coords = normalizedPointerCoordinatesForOrientation(
      event,
      rotationQuarterTurns,
    );
    if (coords) {
      onTouchPreview?.(phase, coords);
      onTouch(phase, coords);
    }
  }

  return {
    isPanning,
    startPanning,
    handlePanPointerMove,
    handlePanPointerUp,
    handleScreenPointerDown,
    handleScreenPointerMove,
    handleScreenPointerUp: (event: React.PointerEvent<HTMLElement>) =>
      finishTouch(event, "ended"),
    handleScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) =>
      finishTouch(event, "cancelled"),
  };
}
