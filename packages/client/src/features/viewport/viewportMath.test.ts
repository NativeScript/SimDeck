import { describe, expect, it } from "vitest";

import {
  buildShellRotationTransform,
  clampPan,
  clampZoom,
  computeChromeBackingRect,
  computeChromeScreenBorderRadius,
  computeChromeScreenRect,
  computeFitScale,
  mapDisplayedPointToNaturalOrientation,
  mapNaturalPointToDisplayedOrientation,
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

  it("centers pan in the visible area when bottom controls reserve canvas space", () => {
    const clamped = clampPan(
      { x: 0, y: 0 },
      0.5,
      { width: 600, height: 800 },
      { width: 300, height: 600 },
      null,
      0,
      120,
    );

    expect(clamped).toEqual({ x: 0, y: -60 });
  });

  it("uses the exact chrome screen rect even when stream aspect differs", () => {
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
    expect(rect).toEqual({
      height: 600,
      width: 300,
      x: 50,
      y: 25,
    });
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

  it("keeps watch display backing separate from stream content", () => {
    const profile = {
      contentHeight: 464.062,
      contentWidth: 381,
      contentX: 88.5,
      contentY: 58.469,
      cornerRadius: 135,
      screenHeight: 513,
      screenWidth: 422,
      screenX: 68,
      screenY: 34,
      totalHeight: 581,
      totalWidth: 573,
    };

    expect(computeChromeBackingRect(profile)).toEqual({
      height: 513,
      width: 422,
      x: 68,
      y: 34,
    });
    expect(
      computeChromeScreenRect(profile, { width: 422, height: 514 }),
    ).toEqual({
      height: 464.062,
      width: 381,
      x: 88.5,
      y: 58.469,
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

  it("maps natural stream points back to displayed overlay coordinates", () => {
    const displayed = { x: 0.2, y: 0.75 };
    const natural = mapDisplayedPointToNaturalOrientation(displayed, 1);
    const remapped = mapNaturalPointToDisplayedOrientation(natural, 1);

    expect(remapped.x).toBeCloseTo(displayed.x);
    expect(remapped.y).toBeCloseTo(displayed.y);

    const remappedCounterClockwise = mapNaturalPointToDisplayedOrientation(
      mapDisplayedPointToNaturalOrientation(displayed, 3),
      3,
    );
    expect(remappedCounterClockwise.x).toBeCloseTo(displayed.x);
    expect(remappedCounterClockwise.y).toBeCloseTo(displayed.y);
  });

  it("builds a quarter-turn transform around the shell origin", () => {
    const portrait = shellSize({ width: 300, height: 650 }, null, 0);
    expect(
      buildShellRotationTransform({ width: 300, height: 650 }, null, 1),
    ).toBe(`translate(${portrait.height}px, 0px) rotate(90deg)`);
  });
});
