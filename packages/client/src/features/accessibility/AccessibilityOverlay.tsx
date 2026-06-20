import { createElement, type AriaRole, type CSSProperties } from "react";

import type { AccessibilityNode } from "../../api/types";
import {
  accessibilityKind,
  accessibilityIdentifier,
  accessibilityRootFrame,
  buildAccessibilityTree,
  findAccessibilityItem,
  isAccessibilityHitTestCandidate,
  paintOrderedAccessibilityItems,
  primaryAccessibilityText,
  validFrame,
} from "./accessibilityTree";

interface AccessibilityOverlayProps {
  hoveredId: string | null;
  roots: AccessibilityNode[];
  selectedId: string;
  skeletonVisible?: boolean;
}

export function AccessibilityOverlay({
  hoveredId,
  roots,
  selectedId,
  skeletonVisible = false,
}: AccessibilityOverlayProps) {
  const rootFrame = accessibilityRootFrame(roots);
  const tree = buildAccessibilityTree(roots);
  const overlayItems = rootFrame
    ? paintOrderedAccessibilityItems(tree).filter(
        isAccessibilityHitTestCandidate,
      )
    : [];
  const selected = selectedId
    ? framedNode(findAccessibilityItem(tree, selectedId)?.node)
    : null;
  const hovered =
    hoveredId && hoveredId !== selectedId
      ? framedNode(findAccessibilityItem(tree, hoveredId)?.node)
      : null;

  if (!rootFrame) {
    return null;
  }
  if (overlayItems.length === 0 && !selected && !hovered) {
    return null;
  }

  return (
    <div
      aria-label="Simulator accessibility overlay"
      className="accessibility-overlay"
    >
      <div className="accessibility-dom-overlay">
        {overlayItems.map((item) => (
          <AccessibilityDomNode
            depth={item.depth}
            id={item.id}
            key={item.id}
            node={item.node}
            rootFrame={rootFrame}
          />
        ))}
      </div>
      <div className="accessibility-visual-overlay" aria-hidden="true">
        {skeletonVisible
          ? overlayItems.map((item) => (
              <NodeRect
                key={`skeleton-${item.id}`}
                node={item.node}
                rootFrame={rootFrame}
                variant="skeleton"
              />
            ))
          : null}
        {hovered ? (
          <NodeRect node={hovered} rootFrame={rootFrame} variant="hovered" />
        ) : null}
        {selected ? (
          <NodeRect node={selected} rootFrame={rootFrame} variant="selected" />
        ) : null}
      </div>
    </div>
  );
}

function framedNode(
  node: AccessibilityNode | null | undefined,
): AccessibilityNode | null {
  if (!node) {
    return null;
  }
  if (validFrame(node.frame)) {
    return node;
  }
  for (const child of node.children ?? []) {
    const framed = framedNode(child);
    if (framed) {
      return framed;
    }
  }
  return null;
}

function NodeRect({
  node,
  rootFrame,
  variant,
}: {
  node: AccessibilityNode;
  rootFrame: { height: number; width: number; x: number; y: number };
  variant: "hovered" | "selected" | "skeleton";
}) {
  if (!validFrame(node.frame)) {
    return null;
  }

  const left = ((node.frame.x - rootFrame.x) / rootFrame.width) * 100;
  const top = ((node.frame.y - rootFrame.y) / rootFrame.height) * 100;
  const width = (node.frame.width / rootFrame.width) * 100;
  const height = (node.frame.height / rootFrame.height) * 100;
  const label =
    variant === "skeleton"
      ? ""
      : primaryAccessibilityText(node) || accessibilityKind(node);

  return (
    <div
      className={`accessibility-rect ${variant}`}
      style={{
        height: `${height}%`,
        left: `${left}%`,
        top: `${top}%`,
        width: `${width}%`,
      }}
    >
      {label ? <span>{label}</span> : null}
    </div>
  );
}

function AccessibilityDomNode({
  depth,
  id,
  node,
  rootFrame,
}: {
  depth: number;
  id: string;
  node: AccessibilityNode;
  rootFrame: { height: number; width: number; x: number; y: number };
}) {
  if (!validFrame(node.frame)) {
    return null;
  }

  const label = accessibilityDomLabel(node);
  const metadata = accessibilityDomMetadata(node, id);
  const kind = accessibilityKind(node);
  const role = accessibilityDomRole(kind);
  const tagName = accessibilityDomTagName(node);
  const description = accessibilityDomDescription(node, metadata);

  return createElement(tagName, {
    "aria-checked":
      role === "checkbox" || role === "switch"
        ? (node.checked ?? undefined)
        : undefined,
    "aria-label": label,
    "aria-description": description,
    "aria-level": depth + 1,
    "aria-selected": node.selected ?? undefined,
    className: "accessibility-dom-node",
    "data-test-id": metadata.testId,
    "data-testid": `simdeck-accessibility-${id}`,
    "data-simdeck-accessibility-id": id,
    "data-simdeck-accessibility-component": kind,
    "data-simdeck-accessibility-description": description,
    "data-simdeck-accessibility-identifier":
      accessibilityIdentifier(node) || undefined,
    "data-simdeck-accessibility-kind": kind,
    "data-simdeck-accessibility-label": primaryAccessibilityText(node),
    "data-simdeck-accessibility-image": metadata.imageName,
    "data-simdeck-accessibility-source-location": metadata.sourceLocation,
    "data-simdeck-accessibility-source-file": metadata.sourceFile,
    "data-simdeck-accessibility-source-line": metadata.sourceLine,
    "data-simdeck-accessibility-source-column": metadata.sourceColumn,
    "data-simdeck-accessibility-source": node.source || undefined,
    "data-simdeck-accessibility-test-id": metadata.testId,
    "data-simdeck-accessibility-native-id": metadata.nativeId,
    "data-simdeck-accessibility-role": role,
    "data-simdeck-accessibility-state": metadata.state,
    "data-simdeck-accessibility-value": metadata.value,
    "data-simdeck-framework": metadata.framework,
    "data-simdeck-framework-id": metadata.frameworkId,
    "data-simdeck-framework-type": metadata.frameworkType,
    "data-simdeck-overlay-node": "accessibility-representation",
    "data-simdeck-inspector-id": node.inspectorId || undefined,
    "data-simdeck-uikit-id": node.uikitId || undefined,
    role,
    style: frameStyle(node.frame, rootFrame),
  });
}

function frameStyle(
  frame: { height: number; width: number; x: number; y: number },
  rootFrame: { height: number; width: number; x: number; y: number },
): CSSProperties {
  return {
    height: `${(frame.height / rootFrame.height) * 100}%`,
    left: `${((frame.x - rootFrame.x) / rootFrame.width) * 100}%`,
    top: `${((frame.y - rootFrame.y) / rootFrame.height) * 100}%`,
    width: `${(frame.width / rootFrame.width) * 100}%`,
  };
}

function accessibilityDomLabel(node: AccessibilityNode): string {
  const text = primaryAccessibilityText(node);
  const identifier = accessibilityIdentifier(node);
  const kind = accessibilityKind(node);
  const parts = [`SimDeck accessibility element`, kind];
  if (text) {
    parts.push(`label "${text}"`);
  }
  if (identifier && identifier !== text) {
    parts.push(`identifier ${identifier}`);
  }
  const metadata = accessibilityDomMetadata(node);
  if (metadata.value && metadata.value !== text) {
    parts.push(`value "${metadata.value}"`);
  }
  if (metadata.placeholder && metadata.placeholder !== text) {
    parts.push(`placeholder "${metadata.placeholder}"`);
  }
  if (metadata.imageName && metadata.imageName !== text) {
    parts.push(`image ${metadata.imageName}`);
  }
  if (node.source) {
    parts.push(`source ${node.source}`);
  }
  if (metadata.sourceLocation) {
    parts.push(`defined at ${metadata.sourceLocation}`);
  }
  if (metadata.state) {
    parts.push(metadata.state);
  }
  return parts.join("; ");
}

function accessibilityDomDescription(
  node: AccessibilityNode,
  metadata: ReturnType<typeof accessibilityDomMetadata>,
): string | undefined {
  const parts = [
    "SimDeck overlay node representing a simulator app element, not the browser page control",
    metadata.framework ? `framework ${metadata.framework}` : "",
    metadata.frameworkType ? `component ${metadata.frameworkType}` : "",
    metadata.testId ? `test id ${metadata.testId}` : "",
    metadata.nativeId ? `native id ${metadata.nativeId}` : "",
    metadata.sourceLocation ? `source ${metadata.sourceLocation}` : "",
    metadata.state ? `state ${metadata.state}` : "",
    node.help ? `hint ${node.help}` : "",
  ].filter(Boolean);
  return parts.length > 0 ? parts.join("; ") : undefined;
}

function accessibilityDomRole(kind: string): AriaRole {
  const normalized = kind.toLowerCase();
  if (normalized.includes("button")) {
    return "button";
  }
  if (normalized.includes("checkbox")) {
    return "checkbox";
  }
  if (normalized.includes("switch")) {
    return "switch";
  }
  if (
    normalized.includes("textfield") ||
    normalized.includes("text field") ||
    normalized.includes("textbox") ||
    normalized.includes("searchfield")
  ) {
    return "textbox";
  }
  if (normalized.includes("slider")) {
    return "slider";
  }
  if (normalized.includes("image") || normalized.includes("icon")) {
    return "img";
  }
  if (
    normalized.includes("text") ||
    normalized.includes("label") ||
    normalized.includes("static")
  ) {
    return "text";
  }
  return "group";
}

export function accessibilityDomTagName(node: AccessibilityNode): string {
  const kind = accessibilityKind(node);
  const component = cleanTagPart(kind) ?? "element";
  return `simdeck-${component}`;
}

function cleanTagPart(value: string | null | undefined): string | null {
  const kebab = value
    ?.trim()
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/[^A-Za-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase();
  return kebab || null;
}

function accessibilityDomMetadata(node: AccessibilityNode, id?: string) {
  const sourceLocation = primarySourceLocation(node);
  const framework = frameworkMetadata(node);
  return {
    framework: framework.framework,
    frameworkId: framework.id,
    frameworkType: framework.type,
    imageName: cleanAccessibilityText(node.imageName),
    nativeId: framework.nativeId,
    placeholder: cleanAccessibilityText(node.placeholder),
    sourceFile: sourceLocation.file || undefined,
    sourceColumn:
      typeof sourceLocation.column === "number"
        ? String(sourceLocation.column)
        : undefined,
    sourceLine:
      typeof sourceLocation.line === "number"
        ? String(sourceLocation.line)
        : undefined,
    sourceLocation: formatSourceLocation(sourceLocation),
    state: accessibilityStateSummary(node, id),
    testId: framework.testId,
    value: cleanAccessibilityText(node.AXValue),
  };
}

function frameworkMetadata(node: AccessibilityNode): {
  framework: string | undefined;
  id: string | undefined;
  nativeId: string | undefined;
  testId: string | undefined;
  type: string | undefined;
} {
  const nativeScript = node.nativeScript ?? {};
  const reactNative = node.reactNative ?? {};
  const flutter = node.flutter ?? {};
  const semantics = node.semantics ?? {};
  return {
    framework: node.source || undefined,
    id:
      stringRecordValue(nativeScript, "id") ??
      stringRecordValue(reactNative, "tag") ??
      stringRecordValue(flutter, "key") ??
      undefined,
    nativeId:
      stringRecordValue(reactNative, "nativeID") ??
      stringRecordValue(nativeScript, "id") ??
      stringRecordValue(semantics, "identifier") ??
      undefined,
    testId:
      stringRecordValue(nativeScript, "testID") ??
      stringRecordValue(reactNative, "testID") ??
      undefined,
    type:
      stringRecordValue(nativeScript, "type") ??
      stringRecordValue(reactNative, "displayName") ??
      stringRecordValue(flutter, "widgetType") ??
      undefined,
  };
}

function stringRecordValue(
  record: Record<string, unknown>,
  key: string,
): string | undefined {
  const value = record[key];
  if (typeof value === "string") {
    return value.trim() || undefined;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return undefined;
}

function primarySourceLocation(node: AccessibilityNode): {
  column: number | null;
  file: string;
  line: number | null;
} {
  const location =
    node.sourceLocation ??
    node.sourceLocations?.find((location) => location?.file) ??
    null;
  const file =
    cleanAccessibilityText(location?.file) ??
    cleanAccessibilityText(node.sourceFile) ??
    "";
  const line =
    typeof location?.line === "number"
      ? location.line
      : typeof node.sourceLine === "number"
        ? node.sourceLine
        : null;
  const column =
    typeof location?.column === "number"
      ? location.column
      : typeof node.sourceColumn === "number"
        ? node.sourceColumn
        : null;
  return { column, file, line };
}

function formatSourceLocation(location: {
  column: number | null;
  file: string;
  line: number | null;
}): string {
  if (!location.file) {
    return "";
  }
  return location.line == null
    ? location.file
    : location.column == null
      ? `${location.file}:${location.line}`
      : `${location.file}:${location.line}:${location.column}`;
}

function accessibilityStateSummary(
  node: AccessibilityNode,
  id: string | undefined,
): string {
  const state = [
    id ? `tree id ${id}` : "",
    node.enabled === false ? "simulator accessibility state enabled=false" : "",
    node.focused === true ? "simulator accessibility state focused=true" : "",
    node.selected === true ? "simulator accessibility state selected=true" : "",
    node.checked === true ? "simulator accessibility state checked=true" : "",
    node.checked === false ? "simulator accessibility state checked=false" : "",
    node.clickable === true
      ? "simulator accessibility action clickable=true"
      : "",
    node.scrollable === true
      ? "simulator accessibility action scrollable=true"
      : "",
  ].filter(Boolean);
  return state.join(", ");
}

function cleanAccessibilityText(
  value: string | null | undefined,
): string | null {
  return value?.trim() || null;
}
