import { describe, expect, it } from "vitest";

import {
  buildShellRotationTransform,
  clampPan,
  clampZoom,
  computeChromeScreenBorderRadius,
  computeChromeScreenRect,
  computeFitScale,
  mapDisplayedPointToNaturalOrientation,
  shellSize,
} from "./viewportMath";

describe("viewportMath", () => {
  it("clamps zoom between fit and max multiplier", () => {
    expect(clampZoom(0.1, 0.5)).toBe(0.325);
    expect(clampZoom(10, 1)).toBe(4);
  });

  it("keeps pan inside computed bounds", () => {
    const clamped = clampPan(
      { x: 500, y: -500 },
      2,
      { width: 300, height: 600 },
      { width: 300, height: 600 },
      null,
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
        totalWidth: 450,
      },
      { width: 300, height: 650 },
    );

    expect(rect).not.toBeNull();
    expect(rect?.x).toBeGreaterThanOrEqual(50);
    expect(rect?.y).toBeGreaterThanOrEqual(25);
  });

  it("uses the full chrome screen when stream and profile aspect nearly match", () => {
    const rect = computeChromeScreenRect(
      {
        cornerRadius: 62,
        screenHeight: 954,
        screenWidth: 438,
        screenX: 18,
        screenY: 18,
        totalHeight: 990,
        totalWidth: 474,
      },
      { width: 1320, height: 2868 },
    );

    expect(rect).toEqual({
      height: 954,
      width: 438,
      x: 18,
      y: 18,
    });
  });

  it("only rounds stream corners that touch the physical screen corners", () => {
    const profile = {
      cornerRadius: 40,
      screenHeight: 600,
      screenWidth: 300,
      screenX: 50,
      screenY: 25,
      totalHeight: 900,
      totalWidth: 450,
    };

    expect(
      computeChromeScreenBorderRadius(profile, {
        height: 600,
        width: 300,
        x: 50,
        y: 25,
      }),
    ).toBe("40px 40px 40px 40px");
    expect(
      computeChromeScreenBorderRadius(profile, {
        height: 220,
        width: 300,
        x: 50,
        y: 215,
      }),
    ).toBe("0px 0px 0px 0px");
  });

  it("reduces fit scale when bottom space is reserved for controls", () => {
    const withoutDock = computeFitScale(
      { width: 900, height: 900 },
      { width: 300, height: 650 },
      null,
    );
    const withDock = computeFitScale(
      { width: 900, height: 900 },
      { width: 300, height: 650 },
      null,
      120,
    );

    expect(withDock).toBeLessThan(withoutDock);
  });

  it("swaps shell dimensions for quarter-turn rotations", () => {
    const portrait = shellSize({ width: 300, height: 650 }, null, 0);
    expect(shellSize({ width: 300, height: 650 }, null, 1)).toEqual({
      height: portrait.width,
      width: portrait.height,
    });
  });

  it("maps rotated pointer coordinates back to the natural stream", () => {
    expect(
      mapDisplayedPointToNaturalOrientation({ x: 0.2, y: 0.75 }, 1),
    ).toEqual({
      x: 0.75,
      y: 0.8,
    });
    expect(
      mapDisplayedPointToNaturalOrientation({ x: 0.2, y: 0.75 }, 3),
    ).toEqual({
      x: 0.25,
      y: 0.2,
    });
  });

  it("builds a quarter-turn transform around the shell origin", () => {
    const portrait = shellSize({ width: 300, height: 650 }, null, 0);
    expect(
      buildShellRotationTransform({ width: 300, height: 650 }, null, 1),
    ).toBe(`translate(${portrait.height}px, 0px) rotate(90deg)`);
  });
});
