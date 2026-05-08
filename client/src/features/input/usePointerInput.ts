import { useRef, useState } from "react";

import type { ChromeProfile, TouchPhase } from "../../api/types";
import {
  normalizedPointerCoordinates,
  normalizedPointerCoordinatesForOrientation,
} from "./gestureMath";
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
  onEdgeTouch?: (
    phase: TouchPhase,
    coords: Point,
    edge: "left" | "top" | "bottom" | "right" | "none",
  ) => void;
  onMultiTouch?: (phase: TouchPhase, first: Point, second: Point) => void;
  onTouchPreview?: (phase: TouchPhase, coords: Point) => void;
  onMultiTouchPreview?: (phase: TouchPhase, coords: Point[]) => void;
}

type ActiveGesture =
  | { kind: "single"; pointerId: number }
  | { kind: "edgeBottom"; pointerId: number }
  | { kind: "pinch"; pointerId: number; first: Point; second: Point }
  | {
      kind: "twoFingerPan";
      pointerId: number;
      start: Point;
      first: Point;
      second: Point;
    };

const TWO_FINGER_SPREAD = 0.16;
const BOTTOM_EDGE_GESTURE_START_Y = 0.93;

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
  onEdgeTouch,
  onMultiTouch,
  onTouchPreview,
  onMultiTouchPreview,
}: UsePointerInputOptions) {
  const activeGestureRef = useRef<ActiveGesture | null>(null);
  const panningRef = useRef<{
    startX: number;
    startY: number;
    startPanX: number;
    startPanY: number;
  } | null>(null);
  const [isPanning, setIsPanning] = useState(false);

  function clampPoint(point: Point): Point {
    return {
      x: Math.min(Math.max(point.x, 0), 1),
      y: Math.min(Math.max(point.y, 0), 1),
    };
  }

  function mirrorAroundCenter(point: Point): Point {
    return clampPoint({ x: 1 - point.x, y: 1 - point.y });
  }

  function previewMultiTouch(phase: TouchPhase, first: Point, second: Point) {
    if (onMultiTouchPreview) {
      onMultiTouchPreview(phase, [first, second]);
      return;
    }
    onTouchPreview?.(phase, first);
  }

  function sendMultiTouch(phase: TouchPhase, first: Point, second: Point) {
    previewMultiTouch(phase, first, second);
    onMultiTouch?.(phase, first, second);
  }

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
    event.preventDefault();
    const coords = normalizedPointerCoordinatesForOrientation(
      event,
      rotationQuarterTurns,
    );
    if (!coords) {
      return;
    }
    event.currentTarget.setPointerCapture(event.pointerId);

    const displayedCoords = normalizedPointerCoordinates(event);
    if (
      displayedCoords &&
      displayedCoords.y >= BOTTOM_EDGE_GESTURE_START_Y &&
      !event.altKey &&
      onEdgeTouch
    ) {
      activeGestureRef.current = {
        kind: "edgeBottom",
        pointerId: event.pointerId,
      };
      onTouchPreview?.("began", coords);
      onEdgeTouch("began", coords, "bottom");
      return;
    }

    if (event.altKey && onMultiTouch) {
      if (event.shiftKey) {
        const first = { x: 0.5 + TWO_FINGER_SPREAD / 2, y: 0.5 };
        const second = { x: 0.5 - TWO_FINGER_SPREAD / 2, y: 0.5 };
        activeGestureRef.current = {
          kind: "twoFingerPan",
          pointerId: event.pointerId,
          start: coords,
          first,
          second,
        };
        sendMultiTouch("began", first, second);
        return;
      }

      const first = clampPoint(coords);
      const second = mirrorAroundCenter(first);
      activeGestureRef.current = {
        kind: "pinch",
        pointerId: event.pointerId,
        first,
        second,
      };
      sendMultiTouch("began", first, second);
      return;
    }

    activeGestureRef.current = { kind: "single", pointerId: event.pointerId };
    onTouchPreview?.("began", coords);
    onTouch("began", coords);
  }

  function handleScreenPointerMove(event: React.PointerEvent<HTMLElement>) {
    event.stopPropagation();
    const active = activeGestureRef.current;
    if (!active || active.pointerId !== event.pointerId) {
      return;
    }
    event.preventDefault();
    const coords = normalizedPointerCoordinatesForOrientation(
      event,
      rotationQuarterTurns,
    );
    if (!coords) {
      return;
    }

    if (active.kind === "pinch") {
      const first = clampPoint(coords);
      const second = mirrorAroundCenter(first);
      activeGestureRef.current = { ...active, first, second };
      sendMultiTouch("moved", first, second);
      return;
    }

    if (active.kind === "twoFingerPan") {
      const delta = {
        x: coords.x - active.start.x,
        y: coords.y - active.start.y,
      };
      const first = clampPoint({
        x: 0.5 + TWO_FINGER_SPREAD / 2 + delta.x,
        y: 0.5 + delta.y,
      });
      const second = clampPoint({
        x: 0.5 - TWO_FINGER_SPREAD / 2 + delta.x,
        y: 0.5 + delta.y,
      });
      activeGestureRef.current = { ...active, first, second };
      sendMultiTouch("moved", first, second);
      return;
    }

    if (active.kind === "single") {
      onTouchPreview?.("moved", coords);
      onTouch("moved", coords);
      return;
    }

    if (active.kind === "edgeBottom") {
      onTouchPreview?.("moved", coords);
      onEdgeTouch?.("moved", coords, "bottom");
    }
  }

  function finishTouch(
    event: React.PointerEvent<HTMLElement>,
    phase: Exclude<TouchPhase, "moved" | "began">,
  ) {
    event.stopPropagation();
    const active = activeGestureRef.current;
    if (!active || active.pointerId !== event.pointerId) {
      return;
    }
    event.preventDefault();
    activeGestureRef.current = null;
    const coords = normalizedPointerCoordinatesForOrientation(
      event,
      rotationQuarterTurns,
    );

    if (active.kind === "pinch") {
      const first = coords ? clampPoint(coords) : active.first;
      const second = coords ? mirrorAroundCenter(first) : active.second;
      sendMultiTouch(phase, first, second);
      return;
    }

    if (active.kind === "twoFingerPan") {
      sendMultiTouch(phase, active.first, active.second);
      return;
    }

    if (active.kind === "edgeBottom") {
      if (coords) {
        onTouchPreview?.(phase, coords);
        onEdgeTouch?.(phase, coords, "bottom");
      }
      return;
    }

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
