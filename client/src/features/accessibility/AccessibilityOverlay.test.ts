import { describe, expect, it } from "vitest";

import { accessibilityDomTagName } from "./AccessibilityOverlay";

describe("accessibilityDomTagName", () => {
  it("uses source and component names for annotator-friendly custom tags", () => {
    expect(
      accessibilityDomTagName({
        source: "nativescript",
        type: "TabItem",
      }),
    ).toBe("simdeck-tab-item");
    expect(
      accessibilityDomTagName({
        source: "nativescript",
        type: "Label",
      }),
    ).toBe("simdeck-label");
    expect(
      accessibilityDomTagName({
        source: "react-native",
        type: "RangeAndFilterBar",
      }),
    ).toBe("simdeck-range-and-filter-bar");
  });
});
