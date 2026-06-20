import { describe, expect, it } from "vitest";

import {
  DEFAULT_VIEWPORT_STATE,
  nextAccessibilitySourcePreference,
  preferredRicherAccessibilitySource,
  readStoredAccessibilitySkeletonMode,
  readStoredFlag,
  sanitizeAccessibilitySources,
  sanitizePersistedUiState,
  shouldRetainAccessibilityTreeDuringRefresh,
  viewportStateForUDID,
  type PersistedViewportState,
} from "./uiState";

describe("uiState", () => {
  it("orders and filters accessibility sources", () => {
    expect(
      sanitizeAccessibilitySources([
        "native-ax",
        "unknown",
        "swiftui",
        "nativescript",
        "native-ax",
        "in-app-inspector",
      ]),
    ).toEqual(["nativescript", "swiftui", "in-app-inspector", "native-ax"]);
  });

  it("prefers a richer hierarchy source over native accessibility", () => {
    expect(
      preferredRicherAccessibilitySource([
        "native-ax",
        "in-app-inspector",
        "react-native",
      ]),
    ).toBe("react-native");
    expect(preferredRicherAccessibilitySource(["native-ax"])).toBeNull();
  });

  it("moves away from native accessibility when a richer source is available", () => {
    expect(
      nextAccessibilitySourcePreference("auto", "native-ax", [
        "react-native",
        "native-ax",
      ]),
    ).toBe("react-native");
    expect(
      nextAccessibilitySourcePreference("native-ax", "native-ax", [
        "react-native",
        "native-ax",
      ]),
    ).toBeNull();
  });

  it("uses a richer fallback when the selected source disappears", () => {
    expect(
      nextAccessibilitySourcePreference("swiftui", "native-ax", [
        "in-app-inspector",
        "native-ax",
      ]),
    ).toBe("in-app-inspector");
  });

  it("keeps an explicit Flutter preference through transient native fallback", () => {
    expect(
      nextAccessibilitySourcePreference("flutter", "native-ax", [
        "in-app-inspector",
        "native-ax",
      ]),
    ).toBeNull();
  });

  it("retains the current Flutter tree during empty or fallback refreshes", () => {
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "auto",
        "flutter",
        "flutter",
        ["flutter", "native-ax"],
        0,
        3,
      ),
    ).toBe(true);
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "flutter",
        "flutter",
        "native-ax",
        ["flutter", "native-ax"],
        12,
        3,
      ),
    ).toBe(true);
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "auto",
        "flutter",
        "flutter",
        ["flutter", "native-ax"],
        4,
        3,
      ),
    ).toBe(false);
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "native-ax",
        "native-ax",
        "flutter",
        ["flutter", "native-ax"],
        4,
        3,
      ),
    ).toBe(false);
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "flutter",
        "native-ax",
        "native-ax",
        ["native-ax"],
        4,
        3,
      ),
    ).toBe(false);
  });

  it("retains the current NativeScript tree during empty or fallback refreshes", () => {
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "auto",
        "nativescript",
        "nativescript",
        ["nativescript", "native-ax"],
        0,
        3,
      ),
    ).toBe(true);
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "nativescript",
        "nativescript",
        "native-ax",
        ["nativescript", "native-ax"],
        12,
        3,
      ),
    ).toBe(true);
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "auto",
        "nativescript",
        "nativescript",
        ["nativescript", "native-ax"],
        4,
        3,
      ),
    ).toBe(false);
  });

  it("does not retain a rich tree after that source disappears", () => {
    expect(
      shouldRetainAccessibilityTreeDuringRefresh(
        "react-native",
        "react-native",
        "nativescript",
        ["nativescript", "native-ax"],
        4,
        3,
      ),
    ).toBe(false);
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

  it("can force a persisted viewport back to fit mode", () => {
    expect(
      viewportStateForUDID(
        {
          viewportByUDID: {
            "sim-1": {
              pan: { x: 20, y: 30 },
              rotationQuarterTurns: 1,
              viewMode: "manual",
              zoom: 1.7,
            },
          },
        },
        "sim-1",
        { forceFit: true },
      ),
    ).toEqual({
      pan: DEFAULT_VIEWPORT_STATE.pan,
      rotationQuarterTurns: 1,
      viewMode: "fit",
      zoom: null,
    });
  });

  it("uses the supplied stored-flag default outside the browser", () => {
    expect(readStoredFlag("missing-flag", true)).toBe(true);
  });

  it("defaults accessibility skeleton frames to auto outside the browser", () => {
    expect(readStoredAccessibilitySkeletonMode()).toBe("auto");
  });
});
