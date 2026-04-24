import { describe, expect, it } from "vitest";

import {
  DEFAULT_VIEWPORT_STATE,
  sanitizeAccessibilitySources,
  sanitizePersistedUiState,
  viewportStateForUDID,
  type PersistedViewportState,
} from "./uiState";

describe("uiState", () => {
  it("orders and filters accessibility sources", () => {
    expect(
      sanitizeAccessibilitySources([
        "native-ax",
        "unknown",
        "nativescript",
        "native-ax",
        "in-app-inspector",
      ]),
    ).toEqual(["nativescript", "in-app-inspector", "native-ax"]);
  });

  it("sanitizes persisted viewport state and falls back to defaults", () => {
    const sanitized = sanitizePersistedUiState({
      bundleIDValue: 123 as unknown as string,
      openURLValue: "https://example.com",
      selectedUDID: "sim-1",
      viewportByUDID: {
        "sim-1": {
          pan: { x: 10, y: 20 },
          rotationQuarterTurns: 3,
          viewMode: "manual",
          zoom: 1.5,
        },
        "sim-2": {
          pan: {
            x: Number.NaN,
            y: 20,
          } as unknown as PersistedViewportState["pan"],
          rotationQuarterTurns: Number.NaN,
          viewMode: "sideways" as unknown as PersistedViewportState["viewMode"],
          zoom: Number.POSITIVE_INFINITY,
        },
      },
    });

    expect(sanitized.bundleIDValue).toBeUndefined();
    expect(sanitized.openURLValue).toBe("https://example.com");
    expect(sanitized.viewportByUDID?.["sim-1"]).toEqual({
      pan: { x: 10, y: 20 },
      rotationQuarterTurns: 3,
      viewMode: "manual",
      zoom: 1.5,
    });
    expect(sanitized.viewportByUDID?.["sim-2"]).toEqual({
      pan: DEFAULT_VIEWPORT_STATE.pan,
      rotationQuarterTurns: DEFAULT_VIEWPORT_STATE.rotationQuarterTurns,
      viewMode: DEFAULT_VIEWPORT_STATE.viewMode,
      zoom: null,
    });
  });

  it("returns the default viewport state for unknown simulators", () => {
    expect(viewportStateForUDID({}, "missing")).toEqual(DEFAULT_VIEWPORT_STATE);
  });
});
