import type { AriaRole, CSSProperties } from "react";

import type { AccessibilityNode } from "../../api/types";
import {
  accessibilityKind,
  accessibilityIdentifier,
  accessibilityRootFrame,
  buildAccessibilityTree,
  findAccessibilityItem,
  flattenAccessibilityTree,
  primaryAccessibilityText,
  validFrame,
} from "./accessibilityTree";

interface AccessibilityOverlayProps {
  hoveredId: string | null;
  roots: AccessibilityNode[];
  selectedId: string;
}

export function AccessibilityOverlay({
  hoveredId,
  roots,
  selectedId,
}: AccessibilityOverlayProps) {
  const rootFrame = accessibilityRootFrame(roots);
  const tree = buildAccessibilityTree(roots);
  const overlayItems = rootFrame
    ? flattenAccessibilityTree(tree).filter((item) =>
        validFrame(item.node.frame),
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
  variant: "hovered" | "selected";
}) {
  if (!validFrame(node.frame)) {
    return null;
  }

  const left = ((node.frame.x - rootFrame.x) / rootFrame.width) * 100;
  const top = ((node.frame.y - rootFrame.y) / rootFrame.height) * 100;
  const width = (node.frame.width / rootFrame.width) * 100;
  const height = (node.frame.height / rootFrame.height) * 100;
  const label = primaryAccessibilityText(node) || accessibilityKind(node);

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
      <span>{label}</span>
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
  const kind = accessibilityKind(node);
  const role = accessibilityDomRole(kind);

  return (
    <div
      aria-checked={
        role === "checkbox" || role === "switch"
          ? (node.checked ?? undefined)
          : undefined
      }
      aria-disabled={node.enabled === false ? true : undefined}
      aria-label={label}
      aria-level={depth + 1}
      aria-selected={node.selected ?? undefined}
      className="accessibility-dom-node"
      data-simdeck-accessibility-id={id}
      data-simdeck-accessibility-identifier={
        accessibilityIdentifier(node) || undefined
      }
      data-simdeck-accessibility-kind={kind}
      data-simdeck-accessibility-source={node.source || undefined}
      role={role}
      style={frameStyle(node.frame, rootFrame)}
    />
  );
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
  if (text && identifier && text !== identifier) {
    return `${kind}: ${text} (${identifier})`;
  }
  return text || identifier || kind;
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
