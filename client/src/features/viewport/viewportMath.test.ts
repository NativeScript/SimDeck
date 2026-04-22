import { describe, expect, it } from "vitest";

import {
  clampPan,
  clampZoom,
  computeChromeScreenRect,
  computeFitScale
} from "./viewportMath";

describe("viewportMath", () => {
  it("clamps zoom between fit and max multiplier", () => {
    expect(clampZoom(0.1, 0.5)).toBe(0.5);
    expect(clampZoom(10, 1)).toBe(4);
  });

  it("keeps pan inside computed bounds", () => {
    const clamped = clampPan(
      { x: 500, y: -500 },
      2,
      { width: 300, height: 600 },
      { width: 300, height: 600 },
      null
    );
    expect(clamped.x).toBeLessThan(500);
    expect(clamped.y).toBeGreaterThan(-500);
  });

  it("fits device aspect inside chrome screen rect", () => {
    const rect = computeChromeScreenRect(
      {
        cornerRadius: 40,
        screenHeight: 600,
        screenWidth: 300,
        screenX: 50,
        screenY: 25,
        totalHeight: 900,
        totalWidth: 450
      },
      { width: 300, height: 650 }
    );

    expect(rect).not.toBeNull();
    expect(rect?.x).toBeGreaterThanOrEqual(50);
    expect(rect?.y).toBeGreaterThanOrEqual(25);
  });

  it("reduces fit scale when bottom space is reserved for controls", () => {
    const withoutDock = computeFitScale(
      { width: 900, height: 900 },
      { width: 300, height: 650 },
      null
    );
    const withDock = computeFitScale(
      { width: 900, height: 900 },
      { width: 300, height: 650 },
      null,
      120
    );

    expect(withDock).toBeLessThan(withoutDock);
  });
});
