import { describe, expect, it } from "vitest";

import type { AccessibilityNode } from "../../api/types";
import { sourceLocationHref } from "./AccessibilityInspector";

describe("sourceLocationHref", () => {
  it("builds VS Code file links with line and column", () => {
    const node: AccessibilityNode = {
      source: "react-native",
      sourceLocation: {
        column: 4,
        file: "/Users/dj/My App/src/App.tsx",
        line: 12,
      },
      type: "View",
    };

    expect(sourceLocationHref(node)).toBe(
      "vscode://file/Users/dj/My%20App/src/App.tsx:12:4",
    );
  });

  it("normalizes file URLs before building VS Code links", () => {
    const node: AccessibilityNode = {
      source: "react-native",
      sourceLocation: {
        file: "file:///Users/dj/My%20App/src/App.tsx",
        line: 12,
      },
      type: "View",
    };

    expect(sourceLocationHref(node)).toBe(
      "vscode://file/Users/dj/My%20App/src/App.tsx:12",
    );
  });

  it("ignores relative source paths", () => {
    const node: AccessibilityNode = {
      source: "react-native",
      sourceLocation: {
        file: "src/App.tsx",
        line: 12,
      },
      type: "View",
    };

    expect(sourceLocationHref(node)).toBe("");
  });
});
