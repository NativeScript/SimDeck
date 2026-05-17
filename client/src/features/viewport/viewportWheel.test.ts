import { describe, expect, it } from "vitest";

import { nextViewportWheelPanState } from "./viewportWheel";

describe("nextViewportWheelPanState", () => {
  it("preserves fit mode and zoom when a fitted viewport cannot pan", () => {
    const pan = { x: 0, y: 0 };

    expect(
      nextViewportWheelPanState({
        canvasSize: { width: 400, height: 800 },
        chromeProfile: null,
        deltaX: 0,
        deltaY: 120,
        deviceNaturalSize: { width: 300, height: 600 },
        effectiveZoom: 1,
        fitScale: 1,
        pan,
        reservedBottomInset: 96,
        rotationQuarterTurns: 0,
        viewMode: "fit",
        zoom: null,
      }),
    ).toEqual({
      pan,
      viewMode: "fit",
      zoom: null,
    });
  });

  it("preserves center mode and zoom while panning", () => {
    expect(
      nextViewportWheelPanState({
        canvasSize: { width: 300, height: 300 },
        chromeProfile: null,
        deltaX: 10,
        deltaY: 20,
        deviceNaturalSize: { width: 600, height: 600 },
        effectiveZoom: 1,
        fitScale: 0.5,
        pan: { x: 0, y: 0 },
        reservedBottomInset: 96,
        rotationQuarterTurns: 0,
        viewMode: "center",
        zoom: null,
      }),
    ).toEqual({
      pan: { x: -10, y: -20 },
      viewMode: "center",
      zoom: null,
    });
  });

  it("keeps manual zoom during plain wheel panning", () => {
    expect(
      nextViewportWheelPanState({
        canvasSize: { width: 300, height: 300 },
        chromeProfile: null,
        deltaX: 10,
        deltaY: 20,
        deviceNaturalSize: { width: 600, height: 600 },
        effectiveZoom: 1.35,
        fitScale: 0.5,
        pan: { x: 0, y: 0 },
        reservedBottomInset: 96,
        rotationQuarterTurns: 0,
        viewMode: "manual",
        zoom: 1.35,
      }),
    ).toEqual({
      pan: { x: -10, y: -20 },
      viewMode: "manual",
      zoom: 1.35,
    });
  });
});
