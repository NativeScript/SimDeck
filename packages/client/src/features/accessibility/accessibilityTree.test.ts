import { describe, expect, it } from "vitest";

import type { AccessibilityNode } from "../../api/types";
import {
  accessibilityIdentifier,
  accessibilityKind,
  buildAccessibilityTree,
  findAccessibilityItemAtPoint,
  isAccessibilityHitTestCandidate,
  paintOrderedAccessibilityItems,
  primaryAccessibilityText,
} from "./accessibilityTree";

describe("buildAccessibilityTree", () => {
  it("renders React Native RCT-prefixed nodes without the prefix", () => {
    expect(accessibilityKind({ source: "react-native", type: "RCTView" })).toBe(
      "View",
    );
    expect(
      accessibilityKind({ source: "react-native", type: "RCTScrollView" }),
    ).toBe("ScrollView");
    expect(accessibilityKind({ source: "react-native", type: "RCTText" })).toBe(
      "Text",
    );
  });

  it("hides internal React Native text children when the Text node has text", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "react-native",
        type: "Text",
        title: "Welcome",
        children: [
          {
            source: "react-native",
            type: "RCTText",
            title: "Welcome",
            children: [
              { source: "react-native", type: "Text", title: "Wel" },
              { source: "react-native", type: "Text", title: "come" },
            ],
          },
        ],
      },
    ];

    const tree = buildAccessibilityTree(roots);

    expect(accessibilityKind(tree[0].node)).toBe("Text");
    expect(primaryAccessibilityText(tree[0].node)).toBe("Welcome");
    expect(tree[0].children).toHaveLength(0);
  });

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

  it("compacts one-child Flutter layout wrappers but keeps app components", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "flutter",
        type: "InspectorDemoHome",
        title: "InspectorDemoHome",
        sourceLocation: { file: "/tmp/demo/lib/main.dart" },
        children: [
          {
            source: "flutter",
            type: "Padding",
            title: "Padding",
            sourceLocation: { file: "/tmp/demo/lib/main.dart" },
            flutter: { transparent: true },
            children: [
              {
                source: "flutter",
                type: "Center",
                title: "Center",
                sourceLocation: { file: "/tmp/demo/lib/main.dart" },
                flutter: { transparent: true },
                children: [
                  {
                    source: "flutter",
                    type: "Text",
                    title: "Continue",
                    AXLabel: "Continue",
                  },
                ],
              },
            ],
          },
        ],
      },
    ];

    const tree = buildAccessibilityTree(roots);

    expect(tree[0].node.type).toBe("InspectorDemoHome");
    expect(tree[0].children[0].node.type).toBe("Text");
    expect(tree[0].children[0].chain.map((node) => node.type)).toEqual([
      "Padding",
      "Center",
    ]);
  });
});

describe("primaryAccessibilityText", () => {
  it("uses image source fallback instead of generated NativeScript class names", () => {
    expect(
      primaryAccessibilityText({
        source: "nativescript",
        type: "_ImageCacheIt",
        title: "_ImageCacheIt",
        imageName: "~/assets/album-midnight.jpg",
      }),
    ).toBe("~/assets/album-midnight.jpg");
  });

  it("ignores object-shaped accessibility text fields without crashing", () => {
    const node: AccessibilityNode = {
      AXIdentifier: { id: "continue-button" } as unknown as string,
      AXLabel: { localized: "Continue" } as unknown as string,
      AXUniqueId: { value: "unique" } as unknown as string,
      AXValue: 42 as unknown as string,
      role: { name: "button" } as unknown as string,
      source: "in-app-inspector",
      title: { value: "Continue" } as unknown as string,
      type: { kind: "Button" } as unknown as string,
    };

    expect(() => buildAccessibilityTree([node])).not.toThrow();
    expect(primaryAccessibilityText(node)).toBe("42");
    expect(accessibilityIdentifier(node)).toBe("");
    expect(accessibilityKind(node)).toBe("Element");
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

  it("ignores module-qualified transparent UIKit tab bar containers", () => {
    const roots: AccessibilityNode[] = [
      {
        type: "UIWindow",
        frame: { x: 0, y: 0, width: 402, height: 874 },
        children: [
          {
            type: "Label",
            title: "Dashboard",
            frame: { x: 24, y: 160, width: 220, height: 44 },
            source: "in-app-inspector",
          },
          {
            className: "UIKit._UITabBarContainerView",
            source: "in-app-inspector",
            title: "UIKit._UITabBarContainerView",
            type: "UIKit._UITabBarContainerView",
            frame: { x: 0, y: 0, width: 402, height: 874 },
            children: [
              {
                className: "UIKit.UIView",
                source: "in-app-inspector",
                title: "UIKit.UIView",
                type: "UIKit.UIView",
                frame: { x: 0, y: 825, width: 402, height: 49 },
              },
            ],
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, { x: 0.12, y: 0.208 });

    expect(item?.node.type).toBe("Label");
    expect(item?.id).toBe("0.0");
  });

  it("ignores transparent Flutter overlays that cover selectable content", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "flutter",
        type: "Stack",
        frame: { x: 0, y: 0, width: 400, height: 800 },
        flutter: { transparent: true },
        children: [
          {
            source: "flutter",
            type: "FilledButton",
            title: "Continue",
            AXLabel: "Continue",
            frame: { x: 100, y: 300, width: 200, height: 60 },
          },
          {
            source: "flutter",
            type: "Listener",
            title: "Listener",
            frame: { x: 0, y: 0, width: 400, height: 800 },
            flutter: { transparent: true },
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, { x: 0.5, y: 0.4125 });

    expect(item?.node.type).toBe("FilledButton");
    expect(item?.id).toBe("0.0");
  });

  it("ignores Flutter composited followers that cover selectable content", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "flutter",
        type: "View",
        frame: { x: 0, y: 0, width: 402, height: 874 },
        children: [
          {
            source: "flutter",
            type: "Text",
            title: "Daily budget",
            AXLabel: "Daily budget",
            frame: { x: 24, y: 180, width: 180, height: 32 },
          },
          {
            source: "flutter",
            type: "CompositedTransformFollower",
            title: "CompositedTransformFollower",
            frame: { x: 0, y: 0, width: 402, height: 874 },
            sourceLocation: {
              file: "file:///Users/dj/Developer/flutter/packages/flutter/lib/src/material/slider.dart",
              line: 1057,
              column: 35,
            },
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, { x: 0.2, y: 0.224 });

    expect(item?.node.type).toBe("Text");
    expect(item?.id).toBe("0.0");
  });

  it("prefers the first application root over stale overlapping roots", () => {
    const roots: AccessibilityNode[] = [
      {
        type: "CurrentApp",
        frame: { x: 0, y: 0, width: 400, height: 800 },
        children: [
          {
            type: "Button",
            AXLabel: "Current continue",
            frame: { x: 100, y: 300, width: 200, height: 60 },
          },
        ],
      },
      {
        type: "PreviousApp",
        frame: { x: 0, y: 0, width: 400, height: 800 },
        children: [
          {
            type: "Button",
            AXLabel: "Previous continue",
            frame: { x: 100, y: 300, width: 200, height: 60 },
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, { x: 0.5, y: 0.4125 });

    expect(item?.node.AXLabel).toBe("Current continue");
    expect(item?.id).toBe("0.0");
  });

  it("does not select full-screen NativeScript containers as fallback targets", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "nativescript",
        type: "Frame",
        title: "Frame",
        frame: { x: 0, y: 0, width: 402, height: 874 },
        children: [
          {
            source: "nativescript",
            type: "Label",
            title: "Now Playing",
            frame: { x: 24, y: 120, width: 180, height: 36 },
          },
        ],
      },
    ];

    expect(findAccessibilityItemAtPoint(roots, { x: 0.5, y: 0.5 })).toBeNull();
    expect(
      findAccessibilityItemAtPoint(roots, { x: 0.2, y: 0.158 })?.node.type,
    ).toBe("Label");
  });

  it("selects synthetic NativeScript tab items over content under the tab bar", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "nativescript",
        type: "TabView",
        title: "TabView",
        frame: { x: 0, y: 0, width: 402, height: 874 },
        children: [
          {
            source: "nativescript",
            type: "Label",
            title: "Album title underneath",
            frame: { x: 0, y: 810, width: 402, height: 44 },
          },
          {
            source: "nativescript",
            type: "TabItem",
            title: "Home",
            frame: { x: 20, y: 791, width: 70, height: 83 },
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, { x: 0.14, y: 0.94 });

    expect(item?.node.type).toBe("TabItem");
    expect(item?.node.title).toBe("Home");
  });

  it("descends through NativeScript tab accessory wrappers", () => {
    const roots: AccessibilityNode[] = [
      {
        source: "nativescript",
        type: "TabView",
        frame: { x: 0, y: 0, width: 402, height: 874 },
        children: [
          {
            source: "nativescript",
            type: "TabAccessory",
            title: "Tab accessory",
            frame: { x: 21, y: 735, width: 360, height: 48 },
            children: [
              {
                source: "nativescript",
                type: "Label",
                title: "Neon Pulse",
                frame: { x: 85, y: 742, width: 212, height: 18 },
              },
            ],
          },
        ],
      },
    ];

    const item = findAccessibilityItemAtPoint(roots, {
      x: 85 / 402,
      y: 742 / 874,
    });

    expect(item?.node.type).toBe("Label");
    expect(item?.node.title).toBe("Neon Pulse");
  });
});

describe("paintOrderedAccessibilityItems", () => {
  it("paints later roots first so the preferred root is hit-tested on top", () => {
    const tree = buildAccessibilityTree([
      {
        type: "CurrentApp",
        frame: { x: 0, y: 0, width: 400, height: 800 },
        children: [
          {
            type: "Button",
            AXLabel: "Current continue",
            frame: { x: 100, y: 300, width: 200, height: 60 },
          },
        ],
      },
      {
        type: "PreviousApp",
        frame: { x: 0, y: 0, width: 400, height: 800 },
      },
    ]);

    expect(paintOrderedAccessibilityItems(tree).map((item) => item.id)).toEqual(
      ["1", "0", "0.0"],
    );
  });

  it("keeps transparent NativeScript containers out of annotatable DOM targets", () => {
    const tree = buildAccessibilityTree([
      {
        source: "nativescript",
        type: "Frame",
        title: "Frame",
        frame: { x: 0, y: 0, width: 402, height: 874 },
        children: [
          {
            source: "nativescript",
            type: "Label",
            title: "Albums",
            frame: { x: 24, y: 120, width: 120, height: 36 },
          },
        ],
      },
    ]);

    expect(
      paintOrderedAccessibilityItems(tree)
        .filter(isAccessibilityHitTestCandidate)
        .map((item) => item.node.type),
    ).toEqual(["Label"]);
  });
});
