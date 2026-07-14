import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  AccessibilityOverlay,
  accessibilityDomTagName,
} from "./AccessibilityOverlay";
import type { AccessibilityNode } from "../../api/types";

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

  it("tolerates non-string accessibility metadata", () => {
    const markup = renderToStaticMarkup(
      createElement(AccessibilityOverlay, {
        hoveredId: null,
        roots: [
          {
            frame: { height: 844, width: 390, x: 0, y: 0 },
            role: "application",
            children: [
              {
                AXValue: 42,
                frame: { height: 48, width: 180, x: 105, y: 720 },
                placeholder: { text: "Email" },
                sourceFile: null,
                type: "TextField",
              } as unknown as AccessibilityNode,
            ],
          },
        ],
        selectedId: "",
      }),
    );

    expect(markup).toContain('data-simdeck-accessibility-value="42"');
    expect(markup).toContain('data-simdeck-accessibility-label="42"');
  });

  it("draws label-free skeleton frames when requested", () => {
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
        skeletonVisible: true,
      }),
    );

    expect(markup).toContain("accessibility-rect skeleton");
    expect(markup).not.toContain("<span>Continue</span>");
  });

  it("converts display-space frames into natural space for a rotated landscape display", () => {
    // Landscape display (1210x834). The child is the right half of the display
    // in display space; after the shell's 270deg rotation it must land in the
    // bottom half of the natural (portrait) presentation box.
    const roots = [
      {
        frame: { height: 834, width: 1210, x: 0, y: 0 },
        role: "application",
        children: [
          {
            AXLabel: "RightHalf",
            frame: { height: 834, width: 605, x: 605, y: 0 },
            type: "Button",
          },
        ],
      },
    ];

    const rotated = renderToStaticMarkup(
      createElement(AccessibilityOverlay, {
        hoveredId: null,
        roots,
        rotationQuarterTurns: 3,
        selectedId: "",
      }),
    );
    // display right-half -> natural bottom-half.
    expect(rotated).toContain("height:50%;left:0%;top:50%;width:100%");

    const unrotated = renderToStaticMarkup(
      createElement(AccessibilityOverlay, {
        hoveredId: null,
        roots,
        rotationQuarterTurns: 0,
        selectedId: "",
      }),
    );
    // With no rotation the frame keeps its display-space placement (right half).
    expect(unrotated).toContain("height:100%;left:50%;top:0%;width:50%");
  });
});
