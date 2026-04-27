import type { AccessibilityFrame, AccessibilityNode } from "../../api/types";

export interface AccessibilityTreeItem {
  children: AccessibilityTreeItem[];
  depth: number;
  id: string;
  node: AccessibilityNode;
}

export function buildAccessibilityTree(
  roots: AccessibilityNode[],
): AccessibilityTreeItem[] {
  return roots.map((node, index) => buildItem(node, `${index}`, 0));
}

export function flattenAccessibilityTree(
  items: AccessibilityTreeItem[],
): AccessibilityTreeItem[] {
  return items.flatMap((item) => [
    item,
    ...flattenAccessibilityTree(item.children),
  ]);
}

export function visibleAccessibilityTreeItems(
  items: AccessibilityTreeItem[],
  expandedIds: Set<string>,
): AccessibilityTreeItem[] {
  return items.flatMap((item) => [
    item,
    ...(expandedIds.has(item.id)
      ? visibleAccessibilityTreeItems(item.children, expandedIds)
      : []),
  ]);
}

export function findAccessibilityItem(
  items: AccessibilityTreeItem[],
  id: string,
): AccessibilityTreeItem | null {
  for (const item of items) {
    if (item.id === id) {
      return item;
    }
    const match = findAccessibilityItem(item.children, id);
    if (match) {
      return match;
    }
  }
  return null;
}

export function findAccessibilityItemAtPoint(
  roots: AccessibilityNode[],
  normalizedPoint: { x: number; y: number },
): AccessibilityTreeItem | null {
  const rootFrame = accessibilityRootFrame(roots);
  if (!rootFrame) {
    return null;
  }

  const point = {
    x: rootFrame.x + normalizedPoint.x * rootFrame.width,
    y: rootFrame.y + normalizedPoint.y * rootFrame.height,
  };
  return findContainingItem(buildAccessibilityTree(roots), point);
}

export function defaultExpandedAccessibilityIds(
  items: AccessibilityTreeItem[],
  maxOpenDepth = 10,
): Set<string> {
  const ids = new Set<string>();
  for (const item of flattenAccessibilityTree(items)) {
    if (item.children.length > 0 && item.depth < maxOpenDepth) {
      ids.add(item.id);
    }
  }
  return ids;
}

export function ancestorAccessibilityIds(id: string): string[] {
  const parts = id.split(".");
  return parts
    .slice(0, -1)
    .map((_, index) => parts.slice(0, index + 1).join("."));
}

export function primaryAccessibilityText(node: AccessibilityNode): string {
  return (
    cleanText(node.AXLabel) ??
    cleanText(node.title) ??
    cleanText(node.AXUniqueId) ??
    cleanText(node.AXIdentifier) ??
    cleanText(node.AXValue) ??
    ""
  );
}

export function accessibilityIdentifier(node: AccessibilityNode): string {
  return cleanText(node.AXUniqueId) ?? cleanText(node.AXIdentifier) ?? "";
}

export function accessibilityKind(node: AccessibilityNode): string {
  return cleanText(node.type) ?? cleanText(node.role) ?? "Element";
}

export function accessibilityRootFrame(
  roots: AccessibilityNode[],
): AccessibilityFrame | null {
  return roots.find((node) => validFrame(node.frame))?.frame ?? null;
}

export function validFrame(
  frame: AccessibilityFrame | null | undefined,
): frame is AccessibilityFrame {
  return Boolean(
    frame &&
    Number.isFinite(frame.x) &&
    Number.isFinite(frame.y) &&
    Number.isFinite(frame.width) &&
    Number.isFinite(frame.height) &&
    frame.width > 0 &&
    frame.height > 0,
  );
}

function buildItem(
  node: AccessibilityNode,
  id: string,
  depth: number,
): AccessibilityTreeItem {
  return {
    children: (node.children ?? []).map((child, index) =>
      buildItem(child, `${id}.${index}`, depth + 1),
    ),
    depth,
    id,
    node,
  };
}

function findContainingItem(
  items: AccessibilityTreeItem[],
  point: { x: number; y: number },
): AccessibilityTreeItem | null {
  for (const item of [...items].reverse()) {
    const childMatch = findContainingItem(item.children, point);
    if (childMatch) {
      return childMatch;
    }

    if (
      frameContainsPoint(item.node.frame, point) &&
      !isTransparentHitTestBlocker(item)
    ) {
      return item;
    }
  }
  return null;
}

function isTransparentHitTestBlocker(item: AccessibilityTreeItem): boolean {
  const node = item.node;
  if (node.source !== "in-app-inspector" || node.nativeScript) {
    return false;
  }

  const rawClassName = cleanText(node.className);
  const type = cleanText(node.type);
  if (!isTransparentContainerClass(rawClassName) && type !== "UIView") {
    return false;
  }

  return !hasMeaningfulNodeContent(node);
}

function isTransparentContainerClass(value: string | null): boolean {
  return (
    value === "UIView" ||
    value === "UITransitionView" ||
    value === "UIDropShadowView" ||
    value === "UIViewControllerWrapperView" ||
    value === "UINavigationTransitionView" ||
    value === "_UITouchPassthroughView" ||
    value === "_UIFloatingBarContainerView" ||
    Boolean(value?.includes("FloatingBarHostingView"))
  );
}

function hasMeaningfulNodeContent(node: AccessibilityNode): boolean {
  const generatedNames = new Set(
    [node.type, node.className, node.role, "UIView", "UIKit View"]
      .map(cleanText)
      .filter(Boolean),
  );
  return [
    node.AXLabel,
    node.text,
    node.AXIdentifier,
    node.AXValue,
    node.placeholder,
    node.imageName,
    node.title,
  ].some((value) => {
    const text = cleanText(value);
    return Boolean(text && !generatedNames.has(text));
  });
}

function frameContainsPoint(
  frame: AccessibilityFrame | null | undefined,
  point: { x: number; y: number },
): boolean {
  return (
    validFrame(frame) &&
    point.x >= frame.x &&
    point.y >= frame.y &&
    point.x <= frame.x + frame.width &&
    point.y <= frame.y + frame.height
  );
}

function cleanText(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}
