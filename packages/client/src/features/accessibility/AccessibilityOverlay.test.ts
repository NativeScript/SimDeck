import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  AccessibilityOverlay,
  accessibilityDomTagName,
} from "./AccessibilityOverlay";

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

  it("falls back when component metadata is not string-like", () => {
    expect(
      accessibilityDomTagName({
        source: "in-app-inspector",
        type: { kind: "Button" } as unknown as string,
      }),
    ).toBe("simdeck-element");
  });
});

describe("AccessibilityOverlay", () => {
  it("does not attach browser-native hover tooltips to selection nodes", () => {
    const markup = renderToStaticMarkup(
      createElement(AccessibilityOverlay, {
        hoveredId: null,
        roots: [
          {
            frame: { height: 844, width: 390, x: 0, y: 0 },
            role: "application",
            children: [
              {
                AXLabel: "Continue",
                frame: { height: 48, width: 180, x: 105, y: 720 },
                type: "Button",
              },
            ],
          },
        ],
        selectedId: "",
      }),
    );

    expect(markup).toContain('aria-label="SimDeck accessibility element');
    expect(markup).not.toContain(" title=");
  });

  it("marks overlay nodes as app representations without bare disabled wording", () => {
    const markup = renderToStaticMarkup(
      createElement(AccessibilityOverlay, {
        hoveredId: null,
        roots: [
          {
            frame: { height: 844, width: 390, x: 0, y: 0 },
            role: "application",
            children: [
              {
                AXLabel: "Upgrade",
                enabled: false,
                frame: { height: 48, width: 180, x: 105, y: 720 },
                nativeScript: {
                  testID: "upgrade-button",
                  type: "Button",
                },
                source: "nativescript",
                sourceLocation: {
                  file: "/Users/dj/Developer/app/src/app.component.ts",
                  line: 12,
                  column: 8,
                },
                type: "Button",
              },
            ],
          },
        ],
        selectedId: "",
      }),
    );

    expect(markup).toContain(
      'data-simdeck-overlay-node="accessibility-representation"',
    );
    expect(markup).toContain(
      "SimDeck overlay node representing a simulator app element",
    );
    expect(markup).toContain('data-test-id="upgrade-button"');
    expect(markup).toContain(
      'data-simdeck-accessibility-source-location="/Users/dj/Developer/app/src/app.component.ts:12:8"',
    );
    expect(markup).toContain("simulator accessibility state enabled=false");
    expect(markup).not.toContain(">disabled<");
    expect(markup).not.toContain("; disabled");
    expect(markup).not.toContain(" title=");
  });

  it("renders object-shaped accessibility metadata without crashing", () => {
    const markup = renderToStaticMarkup(
      createElement(AccessibilityOverlay, {
        hoveredId: null,
        roots: [
          {
            frame: { height: 844, width: 390, x: 0, y: 0 },
            role: "application",
            children: [
              {
                AXLabel: { localized: "Continue" } as unknown as string,
                AXValue: 42 as unknown as string,
                frame: { height: 48, width: 180, x: 105, y: 720 },
                nativeScript: {
                  testID: 123,
                  type: { kind: "Button" },
                },
                placeholder: false as unknown as string,
                source: "nativescript",
                sourceLocation: {
                  file: { path: "/app/app.component.ts" } as unknown as string,
                  line: 12,
                },
                type: { kind: "Button" } as unknown as string,
              },
            ],
          },
        ],
        selectedId: "",
      }),
    );

    expect(markup).toContain("<simdeck-element");
    expect(markup).toContain('data-test-id="123"');
    expect(markup).toContain('data-simdeck-accessibility-value="42"');
    expect(markup).not.toContain("[object Object]");
  });
});
