import { useEffect, useRef, useState } from "react";

import type { ChromeProfile, TouchPhase } from "../../api/types";
import {
  normalizedClientCoordinates,
  normalizedPointerCoordinates,
} from "./gestureMath";
import {
  clampPan,
  mapDisplayedPointToNaturalOrientation,
  mapNaturalPointToDisplayedOrientation,
} from "../viewport/viewportMath";
import type { Point, Size, TouchPreviewPoint } from "../viewport/types";

interface UsePointerInputOptions {
  canvasSize: Size | null;
  chromeProfile: ChromeProfile | null;
  deviceNaturalSize: Size | null;
  effectiveZoom: number;
  fitScale: number;
  isBooted: boolean;
  pan: Point;
  reservedBottomInset: number;
  rotationQuarterTurns: number;
  setPan: React.Dispatch<React.SetStateAction<Point>>;
  onTouch: (phase: TouchPhase, coords: Point) => void;
  onEdgeTouch?: (
    phase: TouchPhase,
    coords: Point,
    edge: "left" | "top" | "bottom" | "right" | "none",
  ) => void;
  onMultiTouch?: (phase: TouchPhase, first: Point, second: Point) => void;
  onTouchPreview?: (phase: TouchPhase, coords: TouchPreviewPoint) => void;
  onMultiTouchPreview?: (
    phase: TouchPhase,
    coords: TouchPreviewPoint[],
  ) => void;
}

type ActiveGesture =
  | { kind: "single"; pointerId: number }
  | { kind: "edgeBottom"; pointerId: number; targetElement: HTMLElement }
  | { kind: "pinch"; pointerId: number; first: Point; second: Point }
  | {
      kind: "twoFingerPan";
      pointerId: number;
      start: Point;
      first: Point;
      second: Point;
    }
  | {
      kind: "directTouch";
      primaryPointerId: number;
      pointers: Map<number, TouchSample>;
      mode: "single" | "multi";
    }
  | {
      kind: "touchMulti";
      touchIds: [number, number];
      targetElement: HTMLElement;
      first: TouchSample;
      second: TouchSample;
    }
  | {
      kind: "touchPendingSingle";
      touchId: number;
      targetElement: HTMLElement;
      sample: TouchSample;
      timer: number;
    }
  | {
      kind: "touchSingle";
      touchId: number;
      targetElement: HTMLElement;
      sample: TouchSample;
    }
  | {
      kind: "touchEdgeBottom";
      touchId: number;
      targetElement: HTMLElement;
      sample: TouchSample;
    };

type TouchSample = {
  displayed: Point;
  natural: Point;
  preview: TouchPreviewPoint;
};

const TWO_FINGER_SPREAD = 0.16;
const PINCH_MINIMUM_SPREAD = 0.16;
const BOTTOM_EDGE_GESTURE_START_Y = 0.93;
const TOUCH_TARGET_TOLERANCE = 0.08;
const TOUCH_SINGLE_BEGIN_DELAY_MS = 70;

export function usePointerInput({
  canvasSize,
  chromeProfile,
  deviceNaturalSize,
  effectiveZoom,
  fitScale,
  isBooted,
  pan,
  reservedBottomInset,
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

  useEffect(() => {
    return () => {
      const active = activeGestureRef.current;
      if (active?.kind === "touchPendingSingle") {
        window.clearTimeout(active.timer);
      }
    };
  }, []);

  function clampPoint(point: Point): Point {
    return {
      x: Math.min(Math.max(point.x, 0), 1),
      y: Math.min(Math.max(point.y, 0), 1),
    };
  }

  function pinchPointsAroundCenter(
    anchor: Point,
    previousFirst?: Point,
  ): [Point, Point] {
    let dx = anchor.x - 0.5;
    let dy = anchor.y - 0.5;
    const halfMinimumSpread = PINCH_MINIMUM_SPREAD / 2;
    const distance = Math.hypot(dx, dy);
    if (distance < halfMinimumSpread) {
      const fallbackDx = previousFirst ? previousFirst.x - 0.5 : 1;
      const fallbackDy = previousFirst ? previousFirst.y - 0.5 : 0;
      const fallbackDistance = Math.hypot(fallbackDx, fallbackDy) || 1;
      dx = (fallbackDx / fallbackDistance) * halfMinimumSpread;
      dy = (fallbackDy / fallbackDistance) * halfMinimumSpread;
    }
    return [
      clampPoint({ x: 0.5 + dx, y: 0.5 + dy }),
      clampPoint({ x: 0.5 - dx, y: 0.5 - dy }),
    ];
  }

  function sampleFromDisplayedPoint(
    displayed: Point,
    browser?: {
      clientX: number;
      clientY: number;
      pageX: number;
      pageY: number;
    },
  ): TouchSample {
    return {
      displayed,
      natural: mapDisplayedPointToNaturalOrientation(
        displayed,
        rotationQuarterTurns,
      ),
      preview: browser
        ? {
            ...displayed,
            clientX: browser.clientX,
            clientY: browser.clientY,
            pageX: browser.pageX,
            pageY: browser.pageY,
          }
        : displayed,
    };
  }

  function sampleFromNaturalPoint(natural: Point): TouchSample {
    const displayed = mapNaturalPointToDisplayedOrientation(
      natural,
      rotationQuarterTurns,
    );
    return {
      displayed,
      natural,
      preview: displayed,
    };
  }

  function previewMultiTouch(
    phase: TouchPhase,
    first: TouchPreviewPoint,
    second: TouchPreviewPoint,
  ) {
    if (onMultiTouchPreview) {
      onMultiTouchPreview(phase, [first, second]);
      return;
    }
    onTouchPreview?.(phase, first);
  }

  function sendMultiTouch(
    phase: TouchPhase,
    first: Point,
    second: Point,
    firstPreview = sampleFromNaturalPoint(first).preview,
    secondPreview = sampleFromNaturalPoint(second).preview,
  ) {
    previewMultiTouch(phase, firstPreview, secondPreview);
    onMultiTouch?.(phase, first, second);
  }

  function sendMultiTouchSamples(
    phase: TouchPhase,
    first: TouchSample,
    second: TouchSample,
  ) {
    sendMultiTouch(
      phase,
      first.natural,
      second.natural,
      first.preview,
      second.preview,
    );
  }

  function firstTwoTouchPoints(
    pointers: Map<number, TouchSample>,
  ): [TouchSample, TouchSample] | null {
    const points = Array.from(pointers.values());
    return points.length >= 2 ? [points[0], points[1]] : null;
  }

  function clientTouchSample(
    target: HTMLElement,
    clientX: number,
    clientY: number,
    pageX = clientX + window.scrollX,
    pageY = clientY + window.scrollY,
    clamp = true,
  ): TouchSample | null {
    const displayed = displayedClientPoint(target, clientX, clientY, clamp);
    return displayed
      ? sampleFromDisplayedPoint(displayed, {
          clientX,
          clientY,
          pageX,
          pageY,
        })
      : null;
  }

  function touchSample(
    touch: React.Touch,
    target: HTMLElement,
    clamp = true,
  ): TouchSample | null {
    return clientTouchSample(
      target,
      touch.clientX,
      touch.clientY,
      touch.pageX,
      touch.pageY,
      clamp,
    );
  }

  function displayedClientPoint(
    target: HTMLElement,
    clientX: number,
    clientY: number,
    clamp = true,
  ): Point | null {
    return normalizedClientCoordinates(target, clientX, clientY, { clamp });
  }

  function touchById(
    touches: React.TouchList,
    identifier: number,
  ): React.Touch | null {
    for (let index = 0; index < touches.length; index += 1) {
      const touch = touches.item(index);
      if (touch?.identifier === identifier) {
        return touch;
      }
    }
    return null;
  }

  function touchesForTarget(
    touches: React.TouchList,
    target: HTMLElement,
  ): React.Touch[] {
    const matchingTouches: React.Touch[] = [];
    for (let index = 0; index < touches.length; index += 1) {
      const touch = touches.item(index);
      if (!touch) {
        continue;
      }
      const displayed = displayedClientPoint(
        target,
        touch.clientX,
        touch.clientY,
        false,
      );
      if (
        displayed &&
        displayed.x >= -TOUCH_TARGET_TOLERANCE &&
        displayed.x <= 1 + TOUCH_TARGET_TOLERANCE &&
        displayed.y >= -TOUCH_TARGET_TOLERANCE &&
        displayed.y <= 1 + TOUCH_TARGET_TOLERANCE
      ) {
        matchingTouches.push(touch);
      }
    }
    return matchingTouches;
  }

  function touchPairForIds(
    target: HTMLElement,
    touches: React.TouchList,
    touchIds: [number, number],
    clamp = true,
  ): [TouchSample, TouchSample] | null {
    const firstTouch = touchById(touches, touchIds[0]);
    const secondTouch = touchById(touches, touchIds[1]);
    if (!firstTouch || !secondTouch) {
      return null;
    }
    const first = touchSample(firstTouch, target, clamp);
    const second = touchSample(secondTouch, target, clamp);
    return first && second ? [first, second] : null;
  }

  function touchPairForEvent(
    event: React.TouchEvent<HTMLElement>,
    touchIds: [number, number],
    clamp = true,
  ): [TouchSample, TouchSample] | null {
    const firstTouch =
      touchById(event.touches, touchIds[0]) ??
      touchById(event.changedTouches, touchIds[0]);
    const secondTouch =
      touchById(event.touches, touchIds[1]) ??
      touchById(event.changedTouches, touchIds[1]);
    if (!firstTouch || !secondTouch) {
      return null;
    }
    const first = touchSample(firstTouch, event.currentTarget, clamp);
    const second = touchSample(secondTouch, event.currentTarget, clamp);
    return first && second ? [first, second] : null;
  }

  function clearPendingTouchSingle(active: ActiveGesture | null) {
    if (active?.kind === "touchPendingSingle") {
      window.clearTimeout(active.timer);
    }
  }

  function flushPendingTouchSingle(touchId: number) {
    const active = activeGestureRef.current;
    if (active?.kind !== "touchPendingSingle" || active.touchId !== touchId) {
      return;
    }
    window.clearTimeout(active.timer);
    activeGestureRef.current = {
      kind: "touchSingle",
      touchId: active.touchId,
      targetElement: active.targetElement,
      sample: active.sample,
    };
    onTouch("began", active.sample.natural);
  }

  function beginPendingTouchSingle(
    touchId: number,
    targetElement: HTMLElement,
    sample: TouchSample,
  ) {
    const timer = window.setTimeout(
      () => flushPendingTouchSingle(touchId),
      TOUCH_SINGLE_BEGIN_DELAY_MS,
    );
    activeGestureRef.current = {
      kind: "touchPendingSingle",
      touchId,
      targetElement,
      sample,
      timer,
    };
    onTouchPreview?.("began", sample.preview);
  }

  function cancelActiveGestureForTouchMulti() {
    const active = activeGestureRef.current;
    if (!active) {
      return;
    }
    activeGestureRef.current = null;
    clearPendingTouchSingle(active);

    if (active.kind === "directTouch") {
      if (active.mode === "multi") {
        const pair = firstTwoTouchPoints(active.pointers);
        if (pair) {
          sendMultiTouchSamples("cancelled", pair[0], pair[1]);
        }
        return;
      }
      const primary = active.pointers.get(active.primaryPointerId);
      if (primary) {
        onTouchPreview?.("cancelled", primary.preview);
        onTouch("cancelled", primary.natural);
      }
      return;
    }

    if (active.kind === "pinch") {
      sendMultiTouch("cancelled", active.first, active.second);
      return;
    }

    if (active.kind === "twoFingerPan") {
      sendMultiTouch("cancelled", active.first, active.second);
      return;
    }

    if (active.kind === "touchMulti") {
      sendMultiTouchSamples("cancelled", active.first, active.second);
      return;
    }

    if (active.kind === "touchPendingSingle") {
      return;
    }

    if (active.kind === "touchSingle") {
      onTouchPreview?.("cancelled", active.sample.preview);
      onTouch("cancelled", active.sample.natural);
      return;
    }

    if (active.kind === "touchEdgeBottom") {
      onTouchPreview?.("cancelled", active.sample.preview);
      onEdgeTouch?.("cancelled", active.sample.natural, "bottom");
    }
  }

  function handleDirectTouchPointerDown(
    event: React.PointerEvent<HTMLElement>,
    sample: TouchSample,
  ): boolean {
    if (event.pointerType === "mouse") {
      return false;
    }

    event.currentTarget.setPointerCapture(event.pointerId);
    const active = activeGestureRef.current;
    if (active?.kind === "directTouch") {
      active.pointers.set(event.pointerId, sample);
      const pair = firstTwoTouchPoints(active.pointers);
      if (pair && onMultiTouch) {
        if (active.mode === "single") {
          const primary = active.pointers.get(active.primaryPointerId);
          if (primary) {
            onTouchPreview?.("cancelled", primary.preview);
            onTouch("cancelled", primary.natural);
          }
          active.mode = "multi";
          sendMultiTouchSamples("began", pair[0], pair[1]);
        } else {
          sendMultiTouchSamples("moved", pair[0], pair[1]);
        }
      }
      return true;
    }

    const pointers = new Map<number, TouchSample>();
    pointers.set(event.pointerId, sample);
    activeGestureRef.current = {
      kind: "directTouch",
      primaryPointerId: event.pointerId,
      pointers,
      mode: "single",
    };
    onTouchPreview?.("began", sample.preview);
    onTouch("began", sample.natural);
    return true;
  }

  function pointerEventShouldDeferToTouchEvents(
    event: React.PointerEvent<HTMLElement>,
  ): boolean {
    return false;
  }

  function screenElementFromBottomBezelTarget(
    target: HTMLElement,
  ): HTMLElement | null {
    return (target.parentElement?.querySelector(".device-screen") ??
      null) as HTMLElement | null;
  }

  function bottomEdgeCoordsFromClientPoint(
    screenElement: HTMLElement,
    clientX: number,
    clientY: number,
  ): TouchSample | null {
    return clientTouchSample(screenElement, clientX, clientY);
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
        reservedBottomInset,
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
    if (pointerEventShouldDeferToTouchEvents(event)) {
      return;
    }
    event.preventDefault();
    const sample = clientTouchSample(
      event.currentTarget,
      event.clientX,
      event.clientY,
      event.pageX,
      event.pageY,
    );
    if (!sample) {
      return;
    }
    if (activeGestureRef.current?.kind === "touchMulti") {
      return;
    }

    if (handleDirectTouchPointerDown(event, sample)) {
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
        targetElement: event.currentTarget,
      };
      onTouchPreview?.("began", sample.preview);
      onEdgeTouch("began", sample.natural, "bottom");
      return;
    }

    if (event.altKey && onMultiTouch) {
      if (event.shiftKey) {
        const first = { x: 0.5 + TWO_FINGER_SPREAD / 2, y: 0.5 };
        const second = { x: 0.5 - TWO_FINGER_SPREAD / 2, y: 0.5 };
        activeGestureRef.current = {
          kind: "twoFingerPan",
          pointerId: event.pointerId,
          start: sample.natural,
          first,
          second,
        };
        sendMultiTouch("began", first, second);
        return;
      }

      const [first, second] = pinchPointsAroundCenter(sample.natural);
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
    onTouchPreview?.("began", sample.preview);
    onTouch("began", sample.natural);
  }

  function handleScreenPointerMove(event: React.PointerEvent<HTMLElement>) {
    event.stopPropagation();
    const active = activeGestureRef.current;
    if (!active) {
      return;
    }
    event.preventDefault();
    if (active.kind === "touchMulti" || active.kind === "touchPendingSingle") {
      return;
    }
    if (active.kind === "touchSingle" || active.kind === "touchEdgeBottom") {
      return;
    }

    if (active.kind === "edgeBottom") {
      if (active.pointerId !== event.pointerId) {
        return;
      }
      const sample = clientTouchSample(
        active.targetElement,
        event.clientX,
        event.clientY,
        event.pageX,
        event.pageY,
      );
      if (sample) {
        onTouchPreview?.("moved", sample.preview);
        onEdgeTouch?.("moved", sample.natural, "bottom");
      }
      return;
    }

    const sample = clientTouchSample(
      event.currentTarget,
      event.clientX,
      event.clientY,
      event.pageX,
      event.pageY,
      active.kind === "directTouch" && active.mode === "multi" ? false : true,
    );
    if (!sample) {
      return;
    }

    if (active.kind === "directTouch") {
      if (!active.pointers.has(event.pointerId)) {
        return;
      }
      active.pointers.set(event.pointerId, sample);
      if (active.mode === "multi") {
        const pair = firstTwoTouchPoints(active.pointers);
        if (pair) {
          sendMultiTouchSamples("moved", pair[0], pair[1]);
        }
        return;
      }
      if (active.primaryPointerId === event.pointerId) {
        onTouchPreview?.("moved", sample.preview);
        onTouch("moved", sample.natural);
      }
      return;
    }

    if (active.pointerId !== event.pointerId) {
      return;
    }

    if (active.kind === "pinch") {
      const [first, second] = pinchPointsAroundCenter(
        sample.natural,
        active.first,
      );
      activeGestureRef.current = { ...active, first, second };
      sendMultiTouch("moved", first, second);
      return;
    }

    if (active.kind === "twoFingerPan") {
      const delta = {
        x: sample.natural.x - active.start.x,
        y: sample.natural.y - active.start.y,
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
      onTouchPreview?.("moved", sample.preview);
      onTouch("moved", sample.natural);
      return;
    }
  }

  function finishTouch(
    event: React.PointerEvent<HTMLElement>,
    phase: Exclude<TouchPhase, "moved" | "began">,
  ) {
    event.stopPropagation();
    const active = activeGestureRef.current;
    if (!active) {
      return;
    }
    event.preventDefault();
    if (active.kind === "touchMulti" || active.kind === "touchPendingSingle") {
      return;
    }
    if (active.kind === "touchSingle" || active.kind === "touchEdgeBottom") {
      return;
    }
    if (active.kind === "edgeBottom") {
      if (active.pointerId !== event.pointerId) {
        return;
      }
      activeGestureRef.current = null;
      const sample = clientTouchSample(
        active.targetElement,
        event.clientX,
        event.clientY,
        event.pageX,
        event.pageY,
      );
      if (sample) {
        onTouchPreview?.(phase, sample.preview);
        onEdgeTouch?.(phase, sample.natural, "bottom");
      }
      return;
    }
    const sample = clientTouchSample(
      event.currentTarget,
      event.clientX,
      event.clientY,
      event.pageX,
      event.pageY,
    );

    if (active.kind === "directTouch") {
      if (!active.pointers.has(event.pointerId)) {
        return;
      }
      if (sample) {
        active.pointers.set(event.pointerId, sample);
      }
      if (active.mode === "multi") {
        const pair = firstTwoTouchPoints(active.pointers);
        if (pair) {
          sendMultiTouchSamples(phase, pair[0], pair[1]);
        }
      } else {
        const finalSample =
          sample ?? active.pointers.get(active.primaryPointerId);
        if (finalSample) {
          onTouchPreview?.(phase, finalSample.preview);
          onTouch(phase, finalSample.natural);
        }
      }
      activeGestureRef.current = null;
      return;
    }

    if (active.pointerId !== event.pointerId) {
      return;
    }
    activeGestureRef.current = null;

    if (active.kind === "pinch") {
      const [first, second] = sample
        ? pinchPointsAroundCenter(sample.natural, active.first)
        : [active.first, active.second];
      sendMultiTouch(phase, first, second);
      return;
    }

    if (active.kind === "twoFingerPan") {
      sendMultiTouch(phase, active.first, active.second);
      return;
    }

    if (sample) {
      onTouchPreview?.(phase, sample.preview);
      onTouch(phase, sample.natural);
    }
  }

  function handleScreenTouchStart(event: React.TouchEvent<HTMLElement>) {
    if (!isBooted) {
      return;
    }

    event.stopPropagation();
    event.preventDefault();

    const active = activeGestureRef.current;
    if (active?.kind === "directTouch") {
      return;
    }
    const targetTouches = touchesForTarget(event.touches, event.currentTarget);
    if (targetTouches.length === 0) {
      return;
    }

    if (targetTouches.length >= 2 && onMultiTouch) {
      const touches: [React.Touch, React.Touch] = [
        targetTouches[0],
        targetTouches[1],
      ];
      const first = touchSample(touches[0], event.currentTarget);
      const second = touchSample(touches[1], event.currentTarget);
      if (!first || !second) {
        return;
      }

      if (active?.kind !== "touchMulti") {
        cancelActiveGestureForTouchMulti();
        activeGestureRef.current = {
          kind: "touchMulti",
          touchIds: [touches[0].identifier, touches[1].identifier],
          targetElement: event.currentTarget,
          first,
          second,
        };
        sendMultiTouchSamples("began", first, second);
        return;
      }

      const pair = touchPairForIds(
        active.targetElement,
        event.touches,
        active.touchIds,
        false,
      ) ?? [first, second];
      active.first = pair[0];
      active.second = pair[1];
      sendMultiTouchSamples("moved", pair[0], pair[1]);
      return;
    }

    if (active?.kind === "touchMulti") {
      return;
    }

    const touch = targetTouches[0];
    const first = touchSample(touch, event.currentTarget);
    if (!first) {
      return;
    }

    cancelActiveGestureForTouchMulti();
    if (first.displayed.y >= BOTTOM_EDGE_GESTURE_START_Y && onEdgeTouch) {
      activeGestureRef.current = {
        kind: "touchEdgeBottom",
        touchId: touch.identifier,
        targetElement: event.currentTarget,
        sample: first,
      };
      onTouchPreview?.("began", first.preview);
      onEdgeTouch("began", first.natural, "bottom");
      return;
    }

    beginPendingTouchSingle(touch.identifier, event.currentTarget, first);
  }

  function handleScreenTouchMove(event: React.TouchEvent<HTMLElement>) {
    const active = activeGestureRef.current;
    if (
      active?.kind !== "touchMulti" &&
      active?.kind !== "touchPendingSingle" &&
      active?.kind !== "touchSingle" &&
      active?.kind !== "touchEdgeBottom"
    ) {
      return;
    }

    event.stopPropagation();
    event.preventDefault();

    if (active.kind === "touchPendingSingle") {
      const touch = touchById(event.touches, active.touchId);
      if (!touch) {
        return;
      }
      const sample = touchSample(touch, active.targetElement);
      if (!sample) {
        return;
      }
      active.sample = sample;
      onTouchPreview?.("moved", sample.preview);
      return;
    }

    if (active.kind === "touchSingle" || active.kind === "touchEdgeBottom") {
      const touch = touchById(event.touches, active.touchId);
      if (!touch) {
        return;
      }
      const sample = touchSample(touch, active.targetElement);
      if (!sample) {
        return;
      }
      active.sample = sample;
      onTouchPreview?.("moved", sample.preview);
      if (active.kind === "touchEdgeBottom") {
        onEdgeTouch?.("moved", sample.natural, "bottom");
      } else {
        onTouch("moved", sample.natural);
      }
      return;
    }

    const pair = touchPairForIds(
      active.targetElement,
      event.touches,
      active.touchIds,
      false,
    );
    if (!pair) {
      return;
    }
    active.first = pair[0];
    active.second = pair[1];
    sendMultiTouchSamples("moved", pair[0], pair[1]);
  }

  function finishScreenTouch(
    event: React.TouchEvent<HTMLElement>,
    phase: Exclude<TouchPhase, "moved" | "began">,
  ) {
    const active = activeGestureRef.current;
    if (
      active?.kind !== "touchMulti" &&
      active?.kind !== "touchPendingSingle" &&
      active?.kind !== "touchSingle" &&
      active?.kind !== "touchEdgeBottom"
    ) {
      return;
    }

    event.stopPropagation();
    event.preventDefault();

    if (active.kind === "touchPendingSingle") {
      const liveTouch = touchById(event.touches, active.touchId);
      if (liveTouch) {
        const sample = touchSample(liveTouch, active.targetElement);
        if (sample) {
          active.sample = sample;
        }
        return;
      }

      const finalTouch = touchById(event.changedTouches, active.touchId);
      const finalSample = finalTouch
        ? touchSample(finalTouch, active.targetElement)
        : active.sample;
      window.clearTimeout(active.timer);
      activeGestureRef.current = null;
      if (finalSample) {
        onTouchPreview?.(phase, finalSample.preview);
        if (phase === "ended") {
          onTouch("began", active.sample.natural);
          onTouch("ended", finalSample.natural);
        }
      }
      return;
    }

    if (active.kind === "touchSingle" || active.kind === "touchEdgeBottom") {
      const liveTouch = touchById(event.touches, active.touchId);
      if (liveTouch) {
        const sample = touchSample(liveTouch, active.targetElement);
        if (sample) {
          active.sample = sample;
        }
        return;
      }

      const finalTouch = touchById(event.changedTouches, active.touchId);
      const finalSample = finalTouch
        ? touchSample(finalTouch, active.targetElement)
        : active.sample;
      activeGestureRef.current = null;
      if (finalSample) {
        onTouchPreview?.(phase, finalSample.preview);
        if (active.kind === "touchEdgeBottom") {
          onEdgeTouch?.(phase, finalSample.natural, "bottom");
        } else {
          onTouch(phase, finalSample.natural);
        }
      }
      return;
    }

    const livePair = touchPairForIds(
      active.targetElement,
      event.touches,
      active.touchIds,
      false,
    );
    if (livePair) {
      active.first = livePair[0];
      active.second = livePair[1];
      return;
    }

    const finalPair = touchPairForEvent(event, active.touchIds, false) ?? [
      active.first,
      active.second,
    ];
    activeGestureRef.current = null;
    sendMultiTouchSamples(phase, finalPair[0], finalPair[1]);
  }

  function handleBottomBezelPointerDown(
    event: React.PointerEvent<HTMLElement>,
  ) {
    if (
      event.button !== 0 ||
      !isBooted ||
      !onEdgeTouch ||
      pointerEventShouldDeferToTouchEvents(event)
    ) {
      return;
    }
    const screenElement = screenElementFromBottomBezelTarget(
      event.currentTarget,
    );
    if (!screenElement) {
      return;
    }
    const sample = bottomEdgeCoordsFromClientPoint(
      screenElement,
      event.clientX,
      event.clientY,
    );
    if (!sample) {
      return;
    }

    event.stopPropagation();
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    activeGestureRef.current = {
      kind: "edgeBottom",
      pointerId: event.pointerId,
      targetElement: screenElement,
    };
    onTouchPreview?.("began", sample.preview);
    onEdgeTouch("began", sample.natural, "bottom");
  }

  function handleBottomBezelTouchStart(event: React.TouchEvent<HTMLElement>) {
    if (!isBooted || !onEdgeTouch || event.touches.length === 0) {
      return;
    }
    const screenElement = screenElementFromBottomBezelTarget(
      event.currentTarget,
    );
    const touch = event.touches.item(0);
    if (!screenElement || !touch) {
      return;
    }
    const sample = bottomEdgeCoordsFromClientPoint(
      screenElement,
      touch.clientX,
      touch.clientY,
    );
    if (!sample) {
      return;
    }

    event.stopPropagation();
    event.preventDefault();
    cancelActiveGestureForTouchMulti();
    activeGestureRef.current = {
      kind: "touchEdgeBottom",
      touchId: touch.identifier,
      targetElement: screenElement,
      sample,
    };
    onTouchPreview?.("began", sample.preview);
    onEdgeTouch("began", sample.natural, "bottom");
  }

  return {
    isPanning,
    cancelActiveGestureForExternalMultiTouch: cancelActiveGestureForTouchMulti,
    startPanning,
    handlePanPointerMove,
    handlePanPointerUp,
    handleScreenPointerDown,
    handleScreenPointerMove,
    handleScreenPointerUp: (event: React.PointerEvent<HTMLElement>) =>
      finishTouch(event, "ended"),
    handleScreenPointerCancel: (event: React.PointerEvent<HTMLElement>) =>
      finishTouch(event, "cancelled"),
    handleScreenTouchStart,
    handleScreenTouchMove,
    handleScreenTouchEnd: (event: React.TouchEvent<HTMLElement>) =>
      finishScreenTouch(event, "ended"),
    handleScreenTouchCancel: (event: React.TouchEvent<HTMLElement>) =>
      finishScreenTouch(event, "cancelled"),
    handleBottomBezelPointerDown,
    handleBottomBezelPointerMove: handleScreenPointerMove,
    handleBottomBezelPointerUp: (event: React.PointerEvent<HTMLElement>) =>
      finishTouch(event, "ended"),
    handleBottomBezelPointerCancel: (event: React.PointerEvent<HTMLElement>) =>
      finishTouch(event, "cancelled"),
    handleBottomBezelTouchStart,
    handleBottomBezelTouchMove: handleScreenTouchMove,
    handleBottomBezelTouchEnd: (event: React.TouchEvent<HTMLElement>) =>
      finishScreenTouch(event, "ended"),
    handleBottomBezelTouchCancel: (event: React.TouchEvent<HTMLElement>) =>
      finishScreenTouch(event, "cancelled"),
  };
}
