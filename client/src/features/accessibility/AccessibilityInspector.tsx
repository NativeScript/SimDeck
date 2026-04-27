import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";

import { sendInspectorRequest } from "../../api/simulators";
import type {
  AccessibilityNode,
  AccessibilitySource,
  AccessibilityTreeResponse,
  SimulatorMetadata,
  UIKitScriptResult,
} from "../../api/types";
import { ConsolePanel } from "./ConsolePanel";
import {
  ancestorAccessibilityIds,
  accessibilityIdentifier,
  accessibilityKind,
  buildAccessibilityTree,
  defaultExpandedAccessibilityIds,
  findAccessibilityItem,
  primaryAccessibilityText,
  validFrame,
  visibleAccessibilityTreeItems,
} from "./accessibilityTree";

interface AccessibilityInspectorProps {
  availableSources: AccessibilitySource[];
  error: string;
  isLoading: boolean;
  onHover: (id: string | null) => void;
  onPickerToggle: () => void;
  onSelect: (id: string) => void;
  onSourceChange: (source: AccessibilitySource) => void;
  roots: AccessibilityNode[];
  pickerActive: boolean;
  selectedId: string;
  selectedSimulator: SimulatorMetadata | null;
  source: AccessibilityTreeResponse["source"] | "";
  visible: boolean;
}

type InspectorTab = "console" | "inspector";

export function AccessibilityInspector({
  availableSources,
  error,
  isLoading,
  onHover,
  onPickerToggle,
  onSelect,
  onSourceChange,
  pickerActive,
  roots,
  selectedId,
  selectedSimulator,
  source,
  visible,
}: AccessibilityInspectorProps) {
  const [panelWidth, setPanelWidth] = useState(() =>
    readStoredNumber("xcw-hierarchy-panel-width", 320),
  );
  const [detailsHeight, setDetailsHeight] = useState(() =>
    readStoredNumber("xcw-hierarchy-details-height", 240),
  );
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  const [activeTab, setActiveTab] = useState<InspectorTab>(() =>
    readStoredTab(),
  );
  const rowRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const expandedInitializedKeyRef = useRef("");
  const resizeStateRef = useRef<ResizeState | null>(null);
  const panelWidthRef = useRef(panelWidth);
  const detailsHeightRef = useRef(detailsHeight);

  useEffect(() => {
    panelWidthRef.current = panelWidth;
  }, [panelWidth]);

  useEffect(() => {
    detailsHeightRef.current = detailsHeight;
  }, [detailsHeight]);

  useEffect(() => {
    function handlePointerMove(event: PointerEvent) {
      const resizeState = resizeStateRef.current;
      if (!resizeState) {
        return;
      }

      event.preventDefault();
      if (resizeState.kind === "width") {
        const nextWidth = clamp(
          resizeState.startValue + event.clientX - resizeState.startPointer,
          240,
          Math.max(280, window.innerWidth * 0.55),
        );
        setPanelWidth(nextWidth);
      } else {
        const nextHeight = clamp(
          resizeState.startValue - (event.clientY - resizeState.startPointer),
          132,
          Math.max(180, window.innerHeight * 0.6),
        );
        setDetailsHeight(nextHeight);
      }
    }

    function handlePointerUp() {
      const resizeState = resizeStateRef.current;
      resizeStateRef.current = null;
      document.body.classList.remove("is-resizing");
      if (!resizeState) {
        return;
      }
      window.localStorage.setItem(
        resizeState.kind === "width"
          ? "xcw-hierarchy-panel-width"
          : "xcw-hierarchy-details-height",
        String(
          resizeState.kind === "width"
            ? panelWidthRef.current
            : detailsHeightRef.current,
        ),
      );
    }

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
    };
  }, []);

  useEffect(() => {
    const udid = selectedSimulator?.udid ?? "";
    const expansionKey = `${udid}:${source || "unknown"}`;
    if (!udid || roots.length === 0) {
      expandedInitializedKeyRef.current = "";
      setExpandedIds(new Set());
      return;
    }

    if (expandedInitializedKeyRef.current === expansionKey) {
      return;
    }

    const tree = buildAccessibilityTree(roots);
    const storedExpandedIds =
      source === "react-native"
        ? []
        : readStoredStringArray(expandedStorageKey(udid));
    setExpandedIds(
      storedExpandedIds.length > 0
        ? new Set(storedExpandedIds)
        : defaultExpandedAccessibilityIds(tree),
    );
    expandedInitializedKeyRef.current = expansionKey;
  }, [roots, selectedSimulator?.udid, source]);

  useEffect(() => {
    const udid = selectedSimulator?.udid ?? "";
    if (!udid || !expandedInitializedKeyRef.current.startsWith(`${udid}:`)) {
      return;
    }

    window.localStorage.setItem(
      expandedStorageKey(udid),
      JSON.stringify([...expandedIds]),
    );
  }, [expandedIds, selectedSimulator?.udid]);

  useEffect(() => {
    window.localStorage.setItem("xcw-hierarchy-active-tab", activeTab);
  }, [activeTab]);

  useEffect(() => {
    if (!selectedId) {
      return;
    }

    setExpandedIds((current) => {
      const next = new Set(current);
      for (const id of ancestorAccessibilityIds(selectedId)) {
        next.add(id);
      }
      return next;
    });

    window.requestAnimationFrame(() => {
      rowRefs.current[selectedId]?.scrollIntoView({
        block: "nearest",
        inline: "nearest",
      });
    });
  }, [selectedId]);

  if (!visible) {
    return null;
  }

  const tree = buildAccessibilityTree(roots);
  const visibleItems = visibleAccessibilityTreeItems(tree, expandedIds);
  const selectedItem = selectedId
    ? findAccessibilityItem(tree, selectedId)
    : null;
  const sourceOptions = hierarchySourceOptions(availableSources, source);

  return (
    <aside className="hierarchy-panel" style={{ width: `${panelWidth}px` }}>
      <div className="hierarchy-tools">
        <button
          aria-label="Pick element from simulator"
          className={`tbtn icon-btn ${pickerActive ? "active" : ""}`}
          disabled={!selectedSimulator?.isBooted}
          onClick={onPickerToggle}
          title="Pick element from simulator"
          type="button"
        >
          <PickerIcon />
        </button>
        <button
          aria-label="Inspector"
          className={`tbtn icon-btn ${activeTab === "inspector" ? "active" : ""}`}
          onClick={() => setActiveTab("inspector")}
          title="Inspector"
          type="button"
        >
          <InspectorIcon />
        </button>
        <button
          aria-label="Console"
          className={`tbtn icon-btn ${activeTab === "console" ? "active" : ""}`}
          onClick={() => setActiveTab("console")}
          title="Console"
          type="button"
        >
          <ConsoleIcon />
        </button>
      </div>
      {activeTab === "console" ? (
        <ConsolePanel
          accessibilityRoots={roots}
          selectedSimulator={selectedSimulator}
          visible={visible && activeTab === "console"}
        />
      ) : (
        <div className="hierarchy-tree">
          {sourceOptions.length > 0 ? (
            <div className="hierarchy-source">
              <div
                className="hierarchy-source-switcher"
                aria-label="Hierarchy source"
              >
                {sourceOptions.map((option) => (
                  <button
                    className={`hierarchy-source-pill source-${option} ${option === source ? "active" : ""}`}
                    disabled={option === source || !selectedSimulator?.isBooted}
                    key={option}
                    onClick={() => onSourceChange(option)}
                    title={`Show ${sourceLabel(option)} hierarchy`}
                    type="button"
                  >
                    {sourceLabel(option)}
                  </button>
                ))}
              </div>
            </div>
          ) : null}
          {!selectedSimulator ? (
            <div className="hierarchy-empty">Select a simulator.</div>
          ) : !selectedSimulator.isBooted ? (
            <div className="hierarchy-empty">
              Boot the simulator to inspect UI.
            </div>
          ) : error ? (
            <div className="hierarchy-empty error">{error}</div>
          ) : visibleItems.length === 0 && isLoading ? (
            <div className="hierarchy-empty">Reading accessibility tree...</div>
          ) : visibleItems.length === 0 ? (
            <div className="hierarchy-empty">
              No accessibility snapshot yet.
            </div>
          ) : (
            visibleItems.map((item) => {
              const kind = accessibilityKind(item.node);
              const label = hierarchyNodeLabel(item.node, kind);
              const sourceBadge = sourceLocationBadgeText(item.node);
              return (
                <div
                  className={`hierarchy-node ${item.id === selectedItem?.id ? "selected" : ""}`}
                  key={item.id}
                  onPointerEnter={() => onHover(item.id)}
                  onPointerLeave={() => onHover(null)}
                  style={{ paddingLeft: `${10 + item.depth * 14}px` }}
                >
                  <button
                    aria-label={
                      expandedIds.has(item.id) ? "Collapse node" : "Expand node"
                    }
                    className={`hierarchy-disclosure ${item.children.length === 0 ? "empty" : ""}`}
                    disabled={item.children.length === 0}
                    onClick={() =>
                      setExpandedIds((current) => {
                        const next = new Set(current);
                        if (next.has(item.id)) {
                          next.delete(item.id);
                        } else {
                          next.add(item.id);
                        }
                        return next;
                      })
                    }
                    type="button"
                  >
                    {expandedIds.has(item.id) ? "▾" : "▸"}
                  </button>
                  <button
                    className="hierarchy-node-main"
                    onClick={() => onSelect(item.id)}
                    ref={(node) => {
                      rowRefs.current[item.id] = node;
                    }}
                    type="button"
                  >
                    <span className="hierarchy-node-kind">{kind}</span>
                    {label ? (
                      <span className="hierarchy-node-text">{label}</span>
                    ) : null}
                    {sourceBadge ? (
                      <span className="hierarchy-node-source">
                        {sourceBadge}
                      </span>
                    ) : null}
                  </button>
                </div>
              );
            })
          )}
        </div>
      )}

      {activeTab === "inspector" && selectedItem ? (
        <div
          className="hierarchy-details-wrap"
          style={{ height: `${detailsHeight}px` }}
        >
          <div
            className="hierarchy-resize-y"
            onPointerDown={(event) => {
              event.preventDefault();
              resizeStateRef.current = {
                kind: "height",
                startPointer: event.clientY,
                startValue: detailsHeight,
              };
              document.body.classList.add("is-resizing");
            }}
          />
          <NodeDetails
            node={selectedItem.node}
            selectedSimulator={selectedSimulator}
          />
        </div>
      ) : null}
      <div
        className="hierarchy-resize-x"
        onPointerDown={(event) => {
          event.preventDefault();
          resizeStateRef.current = {
            kind: "width",
            startPointer: event.clientX,
            startValue: panelWidth,
          };
          document.body.classList.add("is-resizing");
        }}
      />
    </aside>
  );
}

function PickerIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M3.2 2.7 12.6 6l-4.1 1.4-1.7 4.4z"
        stroke="currentColor"
        strokeLinejoin="round"
        strokeWidth="1.4"
      />
      <path
        d="m8.1 7.5 3 3"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="1.4"
      />
    </svg>
  );
}

function InspectorIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M3 3h10v10H3zM5.5 6h5M5.5 8h5M5.5 10h3"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.35"
      />
    </svg>
  );
}

function ConsoleIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="m4 5 3 3-3 3M8.5 11h3.5"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.5"
      />
    </svg>
  );
}

function NodeDetails({
  node,
  selectedSimulator,
}: {
  node: AccessibilityNode;
  selectedSimulator: SimulatorMetadata | null;
}) {
  const details = [
    ["Type", accessibilityKind(node)],
    ["Label", primaryAccessibilityText(node)],
    ["Source", sourceLocationText(node)],
    ["Identifier", accessibilityIdentifier(node)],
    ["Inspector ID", node.inspectorId ?? ""],
    ["Module", node.moduleName ?? ""],
    ["NativeScript", nativeScriptDescription(node.nativeScript)],
    ["React Native", reactNativeDescription(node.reactNative)],
    ["UIKit Class", node.className ?? ""],
    ["Last JS", lastUIKitScriptText(node)],
    ["Value", node.AXValue ?? ""],
    ["Role", node.role ?? ""],
    ["Role Description", node.role_description ?? ""],
    ["View Controller", objectClassName(node.viewController)],
    ["SwiftUI", swiftUIDescription(node.swiftUI)],
    ["Enabled", node.enabled == null ? "" : node.enabled ? "true" : "false"],
    ["Hidden", node.isHidden == null ? "" : node.isHidden ? "true" : "false"],
    ["Alpha", node.alpha == null ? "" : String(round(node.alpha))],
    ["Frame", validFrame(node.frame) ? frameText(node.frame) : ""],
    ["PID", node.pid == null ? "" : String(node.pid)],
    ["Actions", node.custom_actions?.join(", ") ?? ""],
    ["Help", node.help ?? ""],
  ].filter(([, value]) => value);

  return (
    <div className="hierarchy-details">
      <div className="hierarchy-details-title">Properties</div>
      {details.map(([label, value]) => (
        <div className="hierarchy-detail-row" key={label}>
          <span className="hierarchy-detail-label">{label}</span>
          <span className="hierarchy-detail-value">{value}</span>
        </div>
      ))}
      <UIKitScriptEditor node={node} selectedSimulator={selectedSimulator} />
    </div>
  );
}

function UIKitScriptEditor({
  node,
  selectedSimulator,
}: {
  node: AccessibilityNode;
  selectedSimulator: SimulatorMetadata | null;
}) {
  const targetId = dynamicInspectorTargetId(node);
  const lastScript = lastUIKitScriptText(node);
  const isReactNativeTarget = targetId.startsWith("rn:");
  const scriptTitle = isReactNativeTarget ? "React Native JS" : "UIKit JS";
  const udid = selectedSimulator?.udid ?? "";
  const [script, setScript] = useState("");
  const [result, setResult] = useState("");
  const [error, setError] = useState("");
  const [isLoading, setIsLoading] = useState(false);

  useEffect(() => {
    setScript(lastScript || (isReactNativeTarget ? "props" : ""));
    setResult("");
    setError("");
  }, [isReactNativeTarget, lastScript, targetId, udid]);

  if (!targetId) {
    return null;
  }

  async function runScript(event: FormEvent) {
    event.preventDefault();
    if (!udid || !targetId) {
      return;
    }
    if (!script.trim()) {
      setError("Enter JavaScript to run against the selected view.");
      return;
    }

    setIsLoading(true);
    setError("");
    setResult("");
    try {
      const response = await sendInspectorRequest<UIKitScriptResult>(
        udid,
        "View.evaluateScript",
        {
          id: targetId,
          script,
        },
      );
      setResult(jsonPreview(response.result.result ?? null));
    } catch (runError) {
      setError(errorMessage(runError));
    } finally {
      setIsLoading(false);
    }
  }

  return (
    <section className="uikit-script">
      <div className="uikit-script-header">
        <div>
          <div className="uikit-script-title">{scriptTitle}</div>
          <div className="uikit-script-target">{targetId}</div>
        </div>
      </div>
      <form className="uikit-script-form" onSubmit={runScript}>
        <textarea
          className="uikit-script-input"
          onChange={(event) => setScript(event.target.value)}
          placeholder={
            isReactNativeTarget ? "props" : "view.textColor = UIColor.redColor"
          }
          spellCheck={false}
          value={script}
        />
        <button
          className="uikit-script-run"
          disabled={isLoading || !udid}
          type="submit"
        >
          Run
        </button>
      </form>
      {result ? <div className="uikit-script-result">{result}</div> : null}
      {error ? <div className="uikit-script-error">{error}</div> : null}
    </section>
  );
}

function sourceLocationText(node: AccessibilityNode): string {
  const location = primarySourceLocation(node);
  if (!location?.file) {
    return "";
  }

  const line = finiteNumber(location.line);
  const column = finiteNumber(location.column);
  if (line == null) {
    return location.file;
  }
  if (column == null) {
    return `${location.file}:${line}`;
  }
  return `${location.file}:${line}:${column}`;
}

function sourceLocationBadgeText(node: AccessibilityNode): string {
  const location = primarySourceLocation(node);
  if (!location?.file) {
    return "";
  }

  const line = finiteNumber(location.line);
  const fileName = location.file.split(/[\\/]/).pop() ?? location.file;
  return line == null ? fileName : `${fileName}:${line}`;
}

function primarySourceLocation(
  node: AccessibilityNode,
): AccessibilityNode["sourceLocation"] {
  if (node.sourceLocation?.file) {
    return node.sourceLocation;
  }
  const firstLocation = node.sourceLocations?.find(
    (location) => location?.file,
  );
  if (firstLocation) {
    return firstLocation;
  }
  if (node.sourceFile) {
    return {
      column: node.sourceColumn,
      file: node.sourceFile,
      line: node.sourceLine,
    };
  }
  return null;
}

function dynamicInspectorTargetId(node: AccessibilityNode): string {
  const inspectorId = cleanString(node.inspectorId);
  if (inspectorId.startsWith("rn:")) {
    return inspectorId;
  }

  const linkedUIKitId = cleanString(node.uikitId);
  if (linkedUIKitId) {
    return linkedUIKitId;
  }

  if (isUIKitInspectorId(inspectorId)) {
    return inspectorId;
  }

  const uniqueId = cleanString(node.AXUniqueId);
  return isUIKitInspectorId(uniqueId) ? uniqueId : "";
}

function isUIKitInspectorId(value: string): boolean {
  return value.startsWith("view:");
}

function cleanString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function jsonPreview(value: unknown): string {
  try {
    const preview = JSON.stringify(value);
    return preview == null ? String(value) : preview;
  } catch {
    return String(value);
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

const HIERARCHY_SOURCE_ORDER: AccessibilitySource[] = [
  "nativescript",
  "react-native",
  "in-app-inspector",
  "native-ax",
];

function hierarchySourceOptions(
  availableSources: AccessibilitySource[],
  _activeSource: AccessibilityTreeResponse["source"] | "",
): AccessibilitySource[] {
  const sourceSet = new Set(availableSources);
  return HIERARCHY_SOURCE_ORDER.filter((option) => sourceSet.has(option));
}

function hierarchyNodeLabel(node: AccessibilityNode, kind: string): string {
  const label = primaryAccessibilityText(node);
  return sameHierarchyText(label, kind) ? "" : label;
}

function sameHierarchyText(left: string, right: string): boolean {
  return left.trim().toLowerCase() === right.trim().toLowerCase();
}

function sourceLabel(source: AccessibilitySource): string {
  if (source === "nativescript") {
    return "NativeScript";
  }
  if (source === "react-native") {
    return "React Native";
  }
  return source === "in-app-inspector" ? "UIKit" : "Native AX";
}

function objectClassName(value: Record<string, unknown> | null | undefined) {
  const className = value?.className;
  return typeof className === "string" ? className : "";
}

function nativeScriptDescription(
  value: Record<string, unknown> | null | undefined,
) {
  if (!value) {
    return "";
  }
  const type = typeof value.type === "string" ? value.type : "";
  const id = typeof value.id === "string" ? value.id : "";
  const className = typeof value.className === "string" ? value.className : "";
  const suffix = [id ? `#${id}` : "", className ? `.${className}` : ""].join(
    "",
  );
  return type ? `${type}${suffix}` : suffix;
}

function reactNativeDescription(
  value: Record<string, unknown> | null | undefined,
) {
  if (!value) {
    return "";
  }
  const tag = typeof value.tag === "number" ? `tag ${value.tag}` : "";
  const testID = typeof value.testID === "string" ? `#${value.testID}` : "";
  const nativeID =
    typeof value.nativeID === "string" ? `nativeID ${value.nativeID}` : "";
  return [tag, testID, nativeID].filter(Boolean).join(" / ");
}

function lastUIKitScriptText(node: AccessibilityNode): string {
  const direct = stringRecordValue(node.uikitScript, "script");
  if (direct) {
    return direct;
  }
  return stringRecordValue(node.uikit, "script");
}

function stringRecordValue(
  value: Record<string, unknown> | null | undefined,
  key: string,
): string {
  const field = value?.[key];
  return typeof field === "string" ? field : "";
}

function swiftUIDescription(value: Record<string, unknown> | null | undefined) {
  if (!value) {
    return "";
  }
  const tag = typeof value.tag === "string" ? value.tag : "";
  const tagId = typeof value.tagId === "string" ? value.tagId : "";
  const flags = [
    value.isHost === true ? "host" : "",
    value.isProbe === true ? "probe" : "",
  ].filter(Boolean);
  return [tag, tagId, flags.join(", ")].filter(Boolean).join(" / ");
}

function frameText(frame: {
  height: number;
  width: number;
  x: number;
  y: number;
}) {
  return `${round(frame.x)}, ${round(frame.y)}  ${round(frame.width)} x ${round(frame.height)}`;
}

function round(value: number): number {
  return Math.round(value * 10) / 10;
}

function finiteNumber(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

interface ResizeState {
  kind: "height" | "width";
  startPointer: number;
  startValue: number;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function readStoredNumber(key: string, fallback: number): number {
  if (typeof window === "undefined") {
    return fallback;
  }
  const value = Number.parseFloat(window.localStorage.getItem(key) ?? "");
  return Number.isFinite(value) ? value : fallback;
}

function readStoredStringArray(key: string): string[] {
  if (typeof window === "undefined") {
    return [];
  }

  try {
    const parsed = JSON.parse(window.localStorage.getItem(key) ?? "[]");
    return Array.isArray(parsed)
      ? parsed.filter((value): value is string => typeof value === "string")
      : [];
  } catch {
    return [];
  }
}

function expandedStorageKey(udid: string): string {
  return `xcw-hierarchy-expanded-ids:${udid}`;
}

function readStoredTab(): InspectorTab {
  if (typeof window === "undefined") {
    return "inspector";
  }
  const tab = window.localStorage.getItem("xcw-hierarchy-active-tab");
  return tab === "console" ? "console" : "inspector";
}
