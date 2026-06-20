import { describe, expect, it } from "vitest";

import { isMoveControlMessage } from "./controlMessages";

describe("controlMessages", () => {
  it("marks single-touch moves as coalescible", () => {
    expect(
      isMoveControlMessage({ type: "touch", x: 0.4, y: 0.5, phase: "moved" }),
    ).toBe(true);
    expect(
      isMoveControlMessage({
        type: "edgeTouch",
        x: 0.4,
        y: 0.95,
        phase: "moved",
        edge: "bottom",
      }),
    ).toBe(true);
  });

  it("does not coalesce multi-touch, gesture boundaries, or discrete controls", () => {
    expect(
      isMoveControlMessage({ type: "touch", x: 0.4, y: 0.5, phase: "began" }),
    ).toBe(false);
    expect(
      isMoveControlMessage({
        type: "multiTouch",
        x1: 0.35,
        y1: 0.5,
        x2: 0.65,
        y2: 0.5,
        phase: "moved",
      }),
    ).toBe(false);
    expect(
      isMoveControlMessage({
        type: "multiTouch",
        x1: 0.35,
        y1: 0.5,
        x2: 0.65,
        y2: 0.5,
        phase: "ended",
      }),
    ).toBe(false);
    expect(
      isMoveControlMessage({ type: "scroll", deltaX: 0, deltaY: 42 }),
    ).toBe(false);
    expect(isMoveControlMessage({ type: "home" })).toBe(false);
  });
});
