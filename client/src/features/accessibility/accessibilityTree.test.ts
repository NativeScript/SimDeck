import { describe, expect, it } from "vitest";

import type { AccessibilityNode } from "../../api/types";
import {
  buildAccessibilityTree,
  findAccessibilityItemAtPoint,
} from "./accessibilityTree";

describe("buildAccessibilityTree", () => {
  it("compacts framed React Native wrapper chains until meaningful children", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "react-native",
        type: "RCTScrollContentView",
        title: "RCTScrollContentView",
        frame: { x: 0, y: 0, width: 400, height: 800 },
        children: [
          {
            source: "react-native",
            type: "RCTScrollView",
            title: "RCTScrollView",
            frame: { x: 0, y: 0, width: 400, height: 800 },
            children: [
              {
                source: "react-native",
                type: "ScrollViewContext",
                frame: { x: 0, y: 0, width: 400, height: 800 },
                children: [
                  { source: "react-native", type: "Text", title: "Today" },
                  { source: "react-native", type: "Text", title: "7d" },
                ],
              },
            ],
          },
        ],
      },
    ];

    const tree = buildAccessibilityTree(roots);

    expect(tree[0].node.type).toBe("ScrollViewContext");
    expect(tree[0].chain.map((node) => node.type)).toEqual([
      "RCTScrollContentView",
      "RCTScrollView",
    ]);
    expect(tree[0].children).toHaveLength(2);
  });

  it("keeps React Native source-location nodes visible", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "react-native",
        type: "View",
        children: [
          {
            source: "react-native",
            type: "RangeAndFilterBar",
            sourceLocation: {
              file: "/app/src/components/RangeAndFilterBar.tsx",
            },
            children: [{ source: "react-native", type: "RCTView" }],
          },
        ],
      },
    ];

    const tree = buildAccessibilityTree(roots);

    expect(tree[0].node.type).toBe("RangeAndFilterBar");
    expect(tree[0].chain.map((node) => node.type)).toEqual(["View"]);
  });

  it("keeps Expo route display names visible", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "react-native",
        type: "RCTView",
        children: [
          {
            source: "react-native",
            type: "HomeLayout(./(tabs)/(home)/_layout.tsx)",
            title: "HomeLayout(./(tabs)/(home)/_layout.tsx)",
            children: [{ source: "react-native", type: "RCTView" }],
          },
        ],
      },
    ];

    const tree = buildAccessibilityTree(roots);

    expect(tree[0].node.type).toBe("HomeLayout(./(tabs)/(home)/_layout.tsx)");
    expect(tree[0].chain.map((node) => node.type)).toEqual(["RCTView"]);
  });

  it("compacts generated numeric React Native wrapper titles", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "react-native",
        type: "Wrap",
        title: "1",
        children: [
          {
            source: "react-native",
            type: "RCTView",
            title: "2",
            children: [{ source: "react-native", type: "Text", title: "7d" }],
          },
        ],
      },
    ];

    const tree = buildAccessibilityTree(roots);

    expect(tree[0].node.type).toBe("Text");
    expect(tree[0].chain.map((node) => node.type)).toEqual(["Wrap", "RCTView"]);
  });
});

describe("findAccessibilityItemAtPoint", () => {
  it("descends through frameless wrapper nodes", () => {
    const roots: AccessibilityNode[] = [
      {
        type: "GridLayout",
        frame: { x: 0, y: 0, width: 400, height: 800 },
        children: [
          {
            type: "ProxyViewContainer",
            children: [
              {
                type: "Label",
                title: "Grace Hopper",
                frame: { x: 0, y: 200, width: 400, height: 50 },
              },
            ],
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, { x: 0.5, y: 0.275 });

    expect(item?.node.type).toBe("Label");
    expect(item?.id).toBe("0.0.0");
  });

  it("ignores transparent leaf UIViews that cover selectable content", () => {
    const roots: AccessibilityNode[] = [
      {
        type: "UIWindow",
        frame: { x: 0, y: 0, width: 400, height: 800 },
        children: [
          {
            type: "Button",
            title: "Continue",
            frame: { x: 100, y: 300, width: 200, height: 60 },
            source: "in-app-inspector",
          },
          {
            className: "UIView",
            source: "in-app-inspector",
            title: "UIView",
            type: "UIView",
            frame: { x: 0, y: 0, width: 400, height: 800 },
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, { x: 0.5, y: 0.4125 });

    expect(item?.node.type).toBe("Button");
    expect(item?.id).toBe("0.0");
  });

  it("ignores private transparent UIKit containers even when they have children", () => {
    const roots: AccessibilityNode[] = [
      {
        type: "UIWindow",
        frame: { x: 0, y: 0, width: 400, height: 800 },
        children: [
          {
            type: "Label",
            title: "Real row",
            frame: { x: 20, y: 200, width: 360, height: 44 },
            source: "in-app-inspector",
          },
          {
            className: "_UITouchPassthroughView",
            source: "in-app-inspector",
            title: "_UITouchPassthroughView",
            type: "_UITouchPassthroughView",
            frame: { x: 0, y: 0, width: 400, height: 800 },
            children: [
              {
                className: "UIView",
                source: "in-app-inspector",
                title: "UIView",
                type: "UIView",
                frame: { x: 0, y: 0, width: 400, height: 800 },
              },
            ],
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, { x: 0.5, y: 0.265 });

    expect(item?.node.type).toBe("Label");
    expect(item?.id).toBe("0.0");
  });
});
