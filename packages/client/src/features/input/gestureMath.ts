import type { PointerEvent as ReactPointerEvent } from "react";

import type { Point } from "../viewport/types";
import { mapDisplayedPointToNaturalOrientation } from "../viewport/viewportMath";

type QuadPoint = { x: number; y: number };
type BoxQuad = {
  p1: QuadPoint;
  p2: QuadPoint;
  p4: QuadPoint;
};
type ElementWithBoxQuads = HTMLElement & {
  getBoxQuads?: () => BoxQuad[];
};

function clampNormalized(point: Point): Point {
  return {
    x: Math.min(Math.max(point.x, 0), 1),
    y: Math.min(Math.max(point.y, 0), 1),
  };
}

function pointInTransformedBox(point: QuadPoint, quad: BoxQuad): Point | null {
  const xAxis = {
    x: quad.p2.x - quad.p1.x,
    y: quad.p2.y - quad.p1.y,
  };
  const yAxis = {
    x: quad.p4.x - quad.p1.x,
    y: quad.p4.y - quad.p1.y,
  };
  const relative = {
    x: point.x - quad.p1.x,
    y: point.y - quad.p1.y,
  };
  const determinant = xAxis.x * yAxis.y - xAxis.y * yAxis.x;
  if (Math.abs(determinant) < 0.000001) {
    return null;
  }

  return {
    x: (relative.x * yAxis.y - relative.y * yAxis.x) / determinant,
    y: (xAxis.x * relative.y - xAxis.y * relative.x) / determinant,
  };
}

export function normalizedClientCoordinates(
  target: HTMLElement,
  clientX: number,
  clientY: number,
  { clamp = true }: { clamp?: boolean } = {},
): Point | null {
  const quad = (() => {
    try {
      return (target as ElementWithBoxQuads).getBoxQuads?.()[0] ?? null;
    } catch {
      return null;
    }
  })();
  if (quad) {
    const point = pointInTransformedBox({ x: clientX, y: clientY }, quad);
    return point ? (clamp ? clampNormalized(point) : point) : null;
  }

  const rect = target.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) {
    return null;
  }
  const point = {
    x: (clientX - rect.left) / rect.width,
    y: (clientY - rect.top) / rect.height,
  };
  return clamp ? clampNormalized(point) : point;
}

export function normalizedPointerCoordinates(
  event: PointerEvent | ReactPointerEvent<HTMLElement>,
): Point | null {
  const currentTarget = event.currentTarget as HTMLElement | null;
  if (!currentTarget) {
    return null;
  }
  return normalizedClientCoordinates(
    currentTarget,
    event.clientX,
    event.clientY,
  );
}

export function normalizedClientCoordinatesForOrientation(
  target: HTMLElement,
  clientX: number,
  clientY: number,
  rotationQuarterTurns: number,
): Point | null {
  const coords = normalizedClientCoordinates(target, clientX, clientY);
  return coords
    ? mapDisplayedPointToNaturalOrientation(coords, rotationQuarterTurns)
    : null;
}

export function normalizedPointerCoordinatesForOrientation(
  event: PointerEvent | ReactPointerEvent<HTMLElement>,
  rotationQuarterTurns: number,
): Point | null {
  const currentTarget = event.currentTarget as HTMLElement | null;
  return currentTarget
    ? normalizedClientCoordinatesForOrientation(
        currentTarget,
        event.clientX,
        event.clientY,
        rotationQuarterTurns,
      )
    : null;
}
