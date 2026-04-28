import type { AccessibilityNode } from "../../api/types";
import {
  accessibilityKind,
  accessibilityRootFrame,
  buildAccessibilityTree,
  findAccessibilityItem,
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
  const selected = selectedId
    ? framedNode(findAccessibilityItem(tree, selectedId)?.node)
    : null;
  const hovered =
    hoveredId && hoveredId !== selectedId
      ? framedNode(findAccessibilityItem(tree, hoveredId)?.node)
      : null;

  if (!rootFrame || (!selected && !hovered)) {
    return null;
  }

  return (
    <div className="accessibility-overlay" aria-hidden="true">
      {hovered ? (
        <NodeRect node={hovered} rootFrame={rootFrame} variant="hovered" />
      ) : null}
      {selected ? (
        <NodeRect node={selected} rootFrame={rootFrame} variant="selected" />
      ) : null}
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
