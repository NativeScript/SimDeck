import { describe, expect, it } from "vitest";

import type { ChromeProfile } from "../../api/types";
import { isUsableChromeProfile } from "./chromeProfile";

function profile(overrides: Partial<ChromeProfile> = {}): ChromeProfile {
  return {
    buttons: [],
    chromeStyle: "asset",
    contentHeight: 844,
    contentWidth: 390,
    contentX: 31,
    contentY: 22,
    cornerRadius: 46,
    hasScreenMask: false,
    screenHeight: 844,
    screenWidth: 390,
    screenX: 31,
    screenY: 22,
    totalHeight: 888,
    totalWidth: 452,
    ...overrides,
  };
}

describe("isUsableChromeProfile", () => {
  it("accepts a profile whose screen geometry fits inside the chrome", () => {
    expect(isUsableChromeProfile(profile())).toBe(true);
  });

  it("rejects tiny screen geometry produced by missing display metadata", () => {
    expect(isUsableChromeProfile(profile({ screenHeight: 1 }))).toBe(false);
    expect(isUsableChromeProfile(profile({ screenWidth: 1 }))).toBe(false);
  });

  it("rejects screen geometry outside the chrome bounds", () => {
    expect(
      isUsableChromeProfile(profile({ screenX: 80, screenWidth: 390 })),
    ).toBe(false);
    expect(
      isUsableChromeProfile(profile({ screenY: 60, screenHeight: 844 })),
    ).toBe(false);
  });

  it("rejects partial content geometry", () => {
    expect(
      isUsableChromeProfile(
        profile({
          contentHeight: undefined,
          contentWidth: 390,
          contentX: 31,
          contentY: 22,
        }),
      ),
    ).toBe(false);
  });
});
