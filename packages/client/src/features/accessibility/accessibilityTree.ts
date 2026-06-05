import type { AccessibilityFrame, AccessibilityNode } from "../../api/types";

export interface AccessibilityTreeItem {
  chain: AccessibilityNode[];
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

export function paintOrderedAccessibilityItems(
  items: AccessibilityTreeItem[],
): AccessibilityTreeItem[] {
  return [...items]
    .reverse()
    .flatMap((item) => flattenAccessibilityTree([item]));
}

export function isAccessibilityHitTestCandidate(
  item: AccessibilityTreeItem,
): boolean {
  return validFrame(item.node.frame) && !isTransparentHitTestBlocker(item);
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
  return findContainingRootItem(buildAccessibilityTree(roots), point);
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
  const generatedNames = generatedNodeNames(node);
  return (
    [
      node.AXLabel,
      node.text,
      node.title,
      node.AXUniqueId,
      node.AXIdentifier,
      node.AXValue,
      node.placeholder,
      node.imageName,
    ]
      .map(cleanText)
      .find((text) => text && !generatedNames.has(text)) ?? ""
  );
}

export function accessibilityIdentifier(node: AccessibilityNode): string {
  return cleanText(node.AXUniqueId) ?? cleanText(node.AXIdentifier) ?? "";
}

export function accessibilityKind(node: AccessibilityNode): string {
  return displayAccessibilityKind(
    node.source,
    cleanText(node.type) ?? cleanText(node.role) ?? "Element",
  );
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
  const compacted = compactInspectorChain(node, id);
  const children = visibleAccessibilityChildren(compacted.node);
  return {
    chain: compacted.chain,
    children: children.map((child, index) =>
      buildItem(child, `${compacted.id}.${index}`, depth + 1),
    ),
    depth,
    id: compacted.id,
    node: compacted.node,
  };
}

function compactInspectorChain(
  node: AccessibilityNode,
  id: string,
): {
  chain: AccessibilityNode[];
  id: string;
  node: AccessibilityNode;
} {
  const chain: AccessibilityNode[] = [];
  let current = node;
  let currentId = id;

  while (canCompactInspectorNode(current)) {
    const child = current.children?.[0];
    if (!child || child.source !== current.source) {
      break;
    }
    chain.push(current);
    current = child;
    currentId = `${currentId}.0`;
  }

  return { chain, id: currentId, node: current };
}

function canCompactInspectorNode(node: AccessibilityNode): boolean {
  if (node.children?.length !== 1) {
    return false;
  }
  if (node.source === "react-native") {
    return canCompactReactNativeNode(node);
  }
  if (node.source === "flutter") {
    return canCompactFlutterNode(node);
  }
  return false;
}

function canCompactReactNativeNode(node: AccessibilityNode): boolean {
  if (primarySourceLocationFile(node) || isRouteDisplayName(node)) {
    return false;
  }
  return !hasMeaningfulNodeContent(node);
}

function canCompactFlutterNode(node: AccessibilityNode): boolean {
  if (flutterBoolean(node, "transparent")) {
    return true;
  }
  if (hasMeaningfulNodeContent(node)) {
    return false;
  }
  const type = cleanText(node.type);
  if (!isFlutterTransparentContainerType(type)) {
    return false;
  }
  const sourceLocation = primarySourceLocationFile(node);
  return !sourceLocation || isFlutterFrameworkContainerType(type);
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
      isAccessibilityHitTestCandidate(item)
    ) {
      return item;
    }
  }
  return null;
}

function findContainingRootItem(
  items: AccessibilityTreeItem[],
  point: { x: number; y: number },
): AccessibilityTreeItem | null {
  for (const item of items) {
    const childMatch = findContainingItem(item.children, point);
    if (childMatch) {
      return childMatch;
    }

    if (
      frameContainsPoint(item.node.frame, point) &&
      isAccessibilityHitTestCandidate(item)
    ) {
      return item;
    }
  }
  return null;
}

function isTransparentHitTestBlocker(item: AccessibilityTreeItem): boolean {
  const node = item.node;
  if (node.source === "flutter") {
    if (flutterBoolean(node, "transparent")) {
      return true;
    }
    return (
      !hasMeaningfulNodeContent(node) &&
      isFlutterTransparentContainerType(cleanText(node.type))
    );
  }

  if (node.source === "nativescript") {
    return (
      !hasMeaningfulNodeContent(node) &&
      isNativeScriptTransparentContainerType(cleanText(node.type))
    );
  }

  if (node.source !== "in-app-inspector" || node.nativeScript) {
    return false;
  }

  const rawClassName = cleanText(node.className);
  const type = cleanText(node.type);
  if (
    !isTransparentContainerClass(rawClassName) &&
    !isTransparentContainerClass(type)
  ) {
    return false;
  }

  return !hasMeaningfulNodeContent(node);
}

function isNativeScriptTransparentContainerType(value: string | null): boolean {
  const type = unqualifiedClassName(value);
  return Boolean(type && nativeScriptTransparentContainerTypes.has(type));
}

function isTransparentContainerClass(value: string | null): boolean {
  const className = unqualifiedClassName(value);
  return (
    className === "UIView" ||
    className === "UITransitionView" ||
    className === "UIDropShadowView" ||
    className === "UIViewControllerWrapperView" ||
    className === "UINavigationTransitionView" ||
    className === "_UITabBarContainerView" ||
    className === "_UITouchPassthroughView" ||
    className === "_UIFloatingBarContainerView" ||
    Boolean(className?.includes("FloatingBarHostingView"))
  );
}

function unqualifiedClassName(value: string | null): string | null {
  return value?.split(".").pop()?.trim() || value;
}

function flutterBoolean(node: AccessibilityNode, key: string): boolean {
  return node.flutter?.[key] === true;
}

function isFlutterTransparentContainerType(value: string | null): boolean {
  const type = unqualifiedClassName(value);
  return Boolean(
    type &&
    (type.startsWith("_") ||
      flutterTransparentContainerTypes.has(type) ||
      flutterFrameworkContainerTypes.has(type)),
  );
}

function isFlutterFrameworkContainerType(value: string | null): boolean {
  const type = unqualifiedClassName(value);
  return Boolean(
    type &&
    (type.startsWith("_") ||
      flutterFrameworkContainerTypes.has(type) ||
      flutterTransparentContainerTypes.has(type)),
  );
}

const flutterTransparentContainerTypes = new Set([
  "AbsorbPointer",
  "Actions",
  "Align",
  "AnimatedBuilder",
  "AnimatedContainer",
  "AnimatedDefaultTextStyle",
  "AnimatedOpacity",
  "AnimatedPadding",
  "AnimatedPhysicalModel",
  "AnimatedPositioned",
  "AnimatedTheme",
  "AspectRatio",
  "AutomaticKeepAlive",
  "BlockSemantics",
  "Builder",
  "Center",
  "ClipPath",
  "ClipRRect",
  "ClipRect",
  "Column",
  "CompositedTransformFollower",
  "CompositedTransformTarget",
  "ConstrainedBox",
  "Container",
  "CustomMultiChildLayout",
  "CustomPaint",
  "CustomSingleChildLayout",
  "DecoratedBox",
  "DefaultSelectionStyle",
  "DefaultTextStyle",
  "ExcludeSemantics",
  "Directionality",
  "Expanded",
  "Flexible",
  "Focus",
  "FocusScope",
  "FocusTraversalGroup",
  "FractionalTranslation",
  "GestureDetector",
  "IconButtonTheme",
  "IgnorePointer",
  "Ink",
  "IndexedSemantics",
  "InputDecorator",
  "IntrinsicHeight",
  "IntrinsicWidth",
  "KeepAlive",
  "KeyedSubtree",
  "LayoutId",
  "LayoutBuilder",
  "LimitedBox",
  "Listener",
  "ListenableBuilder",
  "ListView",
  "Material",
  "MatrixTransition",
  "MediaQuery",
  "MouseRegion",
  "NotificationListener",
  "Offstage",
  "Opacity",
  "OverflowBox",
  "Padding",
  "PhysicalModel",
  "PhysicalShape",
  "Positioned",
  "PositionedDirectional",
  "PrimaryScrollController",
  "RawGestureDetector",
  "RepaintBoundary",
  "RestorationScope",
  "RootRestorationScope",
  "Row",
  "SafeArea",
  "Scaffold",
  "ScrollNotificationObserver",
  "Scrollable",
  "Semantics",
  "SharedAppData",
  "SizeChangedLayoutNotifier",
  "SizedBox",
  "SliverList",
  "SliverPadding",
  "Stack",
  "TapRegionSurface",
  "TextFieldTapRegion",
  "TextSelectionGestureDetector",
  "Theme",
  "TickerMode",
  "Transform",
  "UnmanagedRestorationScope",
  "UndoHistory",
  "ValueListenableBuilder",
  "Viewport",
]);

const flutterFrameworkContainerTypes = new Set([
  "CheckedModeBanner",
  "CupertinoPageTransition",
  "CupertinoTheme",
  "DecoratedBoxTransition",
  "DefaultTextEditingShortcuts",
  "HeroControllerScope",
  "IconTheme",
  "InheritedCupertinoTheme",
  "ShortcutRegistrar",
  "Localizations",
  "MaterialApp",
  "ModalBarrier",
  "Navigator",
  "Overlay",
  "PageStorage",
  "RawView",
  "RootWidget",
  "ScaffoldMessenger",
  "ScrollConfiguration",
  "Shortcuts",
  "SlideTransition",
  "Title",
  "View",
  "WidgetsApp",
]);

const nativeScriptTransparentContainerTypes = new Set([
  "AbsoluteLayout",
  "ActionBar",
  "ContentView",
  "DockLayout",
  "FlexboxLayout",
  "Frame",
  "GridLayout",
  "HtmlView",
  "Page",
  "Placeholder",
  "ProxyViewContainer",
  "RootLayout",
  "StackLayout",
  "TabAccessory",
  "TabView",
  "WrapLayout",
]);

function hasMeaningfulNodeContent(node: AccessibilityNode): boolean {
  const generatedNames = generatedNodeNames(node);
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
    if (text && isGeneratedReactNativeText(node, text)) {
      return false;
    }
    return Boolean(text && !generatedNames.has(text));
  });
}

function generatedNodeNames(node: AccessibilityNode): Set<string> {
  const names = [node.type, node.className, node.role, "UIView", "UIKit View"]
    .map(cleanText)
    .filter((text): text is string => Boolean(text));
  return new Set([...names, ...names.map((name) => `_${name}`)]);
}

function primarySourceLocationFile(node: AccessibilityNode): string {
  return (
    cleanText(node.sourceLocation?.file) ??
    cleanText(node.sourceLocations?.find((location) => location?.file)?.file) ??
    cleanText(node.sourceFile) ??
    ""
  );
}

function isRouteDisplayName(node: AccessibilityNode): boolean {
  return /\(\.{1,2}\/.+\.[cm]?[jt]sx?\)$/.test(cleanText(node.type) ?? "");
}

function isGeneratedReactNativeText(
  node: AccessibilityNode,
  text: string,
): boolean {
  if (node.source !== "react-native" || node.children?.length !== 1) {
    return false;
  }
  const type = cleanText(node.type);
  if (type === "Text" || type === "RCTText") {
    return false;
  }
  return /^\d+$/.test(text);
}

function visibleAccessibilityChildren(
  node: AccessibilityNode,
): AccessibilityNode[] {
  if (
    isReactNativeTextDisplayNode(node) &&
    Boolean(primaryAccessibilityText(node))
  ) {
    return [];
  }
  return node.children ?? [];
}

function isReactNativeTextDisplayNode(node: AccessibilityNode): boolean {
  if (node.source !== "react-native") {
    return false;
  }
  return stripReactNativePrefix(cleanText(node.type) ?? "") === "Text";
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

function cleanText(value: unknown): string | null {
  if (typeof value === "string") {
    return value.trim() || null;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return null;
}

function displayAccessibilityKind(
  source: AccessibilityNode["source"],
  kind: string,
): string {
  if (source === "react-native") {
    return stripReactNativePrefix(kind);
  }
  return kind;
}

function stripReactNativePrefix(kind: string): string {
  return kind.startsWith("RCT") && kind.length > 3 ? kind.slice(3) : kind;
}
