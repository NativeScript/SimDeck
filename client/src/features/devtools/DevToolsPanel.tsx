import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  CSSProperties,
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
} from "react";

import { accessTokenFromLocation } from "../../api/client";
import { apiUrl } from "../../api/config";
import {
  fetchChromeDevToolsTargets,
  fetchWebKitTargets,
} from "../../api/simulators";
import type {
  ChromeDevToolsTarget,
  SimulatorMetadata,
  WebKitTarget,
} from "../../api/types";

const DEVTOOLS_TARGET_REFRESH_MS = 5000;
const CHROME_DEVTOOLS_REQUEST_TIMEOUT_MS = 6000;
const WEBKIT_DEVTOOLS_REQUEST_TIMEOUT_MS = 2500;
const DEVTOOLS_PANEL_WIDTH_STORAGE_KEY = "xcw-devtools-panel-width";
const LEGACY_PANEL_WIDTH_STORAGE_KEYS = [
  "xcw-chrome-devtools-panel-width",
  "xcw-webkit-panel-width",
];
const DEVTOOLS_PANEL_DEFAULT_WIDTH = 720;
const DEVTOOLS_PANEL_MIN_WIDTH = 420;
const DEVTOOLS_PANEL_MIN_VIEWPORT_WIDTH = 340;
const DEVTOOLS_PANEL_WIDTH_STEP = 40;

interface DevToolsPanelProps {
  onClose: () => void;
  selectedSimulator: SimulatorMetadata | null;
}

interface ResizeState {
  handle: HTMLDivElement;
  pointerId: number;
  startPointer: number;
  startValue: number;
}

interface DevToolsTarget {
  frameUrl: string;
  id: string;
  meta: string;
  source: string;
  title: string;
}

interface DevToolsDiscovery {
  targets: DevToolsTarget[];
  warnings: string[];
}

export function DevToolsPanel({
  onClose,
  selectedSimulator,
}: DevToolsPanelProps) {
  const [panelWidth, setPanelWidth] = useState(readStoredPanelWidth);
  const [isResizing, setIsResizing] = useState(false);
  const [discovery, setDiscovery] = useState<DevToolsDiscovery | null>(null);
  const [selectedTargetId, setSelectedTargetId] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState("");
  const [frameLoaded, setFrameLoaded] = useState(false);
  const discoveryRef = useRef<DevToolsDiscovery | null>(null);
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const panelWidthRef = useRef(panelWidth);
  const requestIdRef = useRef(0);
  const resizeStateRef = useRef<ResizeState | null>(null);
  const selectedTargetIdRef = useRef("");

  const targets = discovery?.targets ?? [];
  const selectedTarget = useMemo(() => {
    if (targets.length === 0) {
      return null;
    }
    return (
      targets.find((target) => target.id === selectedTargetId) ?? targets[0]
    );
  }, [selectedTargetId, targets]);
  const frameUrl = selectedTarget?.frameUrl ?? "";

  useEffect(() => {
    panelWidthRef.current = panelWidth;
  }, [panelWidth]);

  const applyDiscovery = useCallback(
    (nextDiscovery: DevToolsDiscovery | null) => {
      discoveryRef.current = nextDiscovery;
      setDiscovery(nextDiscovery);
    },
    [],
  );

  const applySelectedTargetId = useCallback((nextTargetId: string) => {
    selectedTargetIdRef.current = nextTargetId;
    setSelectedTargetId(nextTargetId);
  }, []);

  const loadTargets = useCallback(async () => {
    if (!selectedSimulator) {
      applyDiscovery(null);
      applySelectedTargetId("");
      setError("");
      setIsLoading(false);
      return;
    }

    const requestId = ++requestIdRef.current;
    setIsLoading(true);
    setError("");
    try {
      const chromeTargets = requestWithTimeout(
        (signal) =>
          fetchChromeDevToolsTargets(selectedSimulator.udid, { signal }),
        CHROME_DEVTOOLS_REQUEST_TIMEOUT_MS,
        "Timed out loading Chrome DevTools targets.",
      );
      const webKitTargets = selectedSimulator.isBooted
        ? requestWithTimeout(
            (signal) => fetchWebKitTargets(selectedSimulator.udid, { signal }),
            WEBKIT_DEVTOOLS_REQUEST_TIMEOUT_MS,
            "Timed out loading WebKit targets.",
          )
        : Promise.resolve({
            socketPath: null,
            targets: [],
            udid: selectedSimulator.udid,
            warnings: [],
          });
      const [chromeResult, webKitResult] = await Promise.allSettled([
        chromeTargets,
        webKitTargets,
      ]);
      if (requestId !== requestIdRef.current) {
        return;
      }

      const nextTargets: DevToolsTarget[] = [];
      const warnings: string[] = [];
      const errors: string[] = [];
      if (chromeResult.status === "fulfilled") {
        nextTargets.push(...chromeResult.value.targets.map(mapChromeTarget));
        warnings.push(...chromeResult.value.warnings);
      } else {
        errors.push(errorMessage(chromeResult.reason));
      }

      if (webKitResult.status === "fulfilled") {
        nextTargets.push(...webKitResult.value.targets.map(mapWebKitTarget));
        warnings.push(...webKitResult.value.warnings);
      } else {
        errors.push(errorMessage(webKitResult.reason));
      }

      const previousDiscovery = discoveryRef.current;
      if (
        nextTargets.length === 0 &&
        previousDiscovery &&
        previousDiscovery.targets.length > 0
      ) {
        applyDiscovery({
          ...previousDiscovery,
          warnings: mergeWarnings(
            warnings,
            errors,
            previousDiscovery.warnings,
            [
              "DevTools target discovery returned no targets; keeping the active target while debuggers reconnect.",
            ],
          ),
        });
        return;
      }

      const nextDiscovery = {
        targets: nextTargets,
        warnings: mergeWarnings(warnings, errors),
      };
      applyDiscovery(nextDiscovery);
      const current = selectedTargetIdRef.current;
      const nextTargetId =
        current && nextTargets.some((target) => target.id === current)
          ? current
          : (nextTargets[0]?.id ?? "");
      applySelectedTargetId(nextTargetId);
      if (nextTargets.length === 0 && errors.length > 0) {
        setError(errors.join(" "));
      }
    } catch (targetError) {
      if (requestId !== requestIdRef.current) {
        return;
      }
      const message = errorMessage(targetError);
      const previousDiscovery = discoveryRef.current;
      if (previousDiscovery && previousDiscovery.targets.length > 0) {
        applyDiscovery({
          ...previousDiscovery,
          warnings: mergeWarnings(previousDiscovery.warnings, [message]),
        });
        return;
      }
      applyDiscovery(null);
      applySelectedTargetId("");
      setError(message);
    } finally {
      if (requestId === requestIdRef.current) {
        setIsLoading(false);
      }
    }
  }, [
    applyDiscovery,
    applySelectedTargetId,
    selectedSimulator?.isBooted,
    selectedSimulator?.udid,
  ]);

  useEffect(() => {
    requestIdRef.current += 1;
    applyDiscovery(null);
    applySelectedTargetId("");
    setError("");
    setFrameLoaded(false);
  }, [applyDiscovery, applySelectedTargetId, selectedSimulator?.udid]);

  useEffect(() => {
    void loadTargets();
    const interval = window.setInterval(() => {
      if (!selectedTargetIdRef.current) {
        void loadTargets();
      }
    }, DEVTOOLS_TARGET_REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [loadTargets]);

  useEffect(() => {
    setFrameLoaded(false);
  }, [frameUrl]);

  useEffect(() => {
    function handlePointerMove(event: PointerEvent) {
      const resizeState = resizeStateRef.current;
      if (!resizeState) {
        return;
      }

      event.preventDefault();
      const nextWidth = clampPanelWidth(
        resizeState.startValue + resizeState.startPointer - event.clientX,
      );
      panelWidthRef.current = nextWidth;
      setPanelWidth(nextWidth);
    }

    function finishResize() {
      const resizeState = resizeStateRef.current;
      resizeStateRef.current = null;
      setIsResizing(false);
      document.body.classList.remove("is-resizing-devtools");
      if (!resizeState) {
        return;
      }
      if (resizeState.handle.hasPointerCapture(resizeState.pointerId)) {
        resizeState.handle.releasePointerCapture(resizeState.pointerId);
      }
      storePanelWidth(panelWidthRef.current);
    }

    function handleViewportResize() {
      setPanelWidth((currentWidth) => {
        const nextWidth = clampPanelWidth(currentWidth);
        panelWidthRef.current = nextWidth;
        return nextWidth;
      });
    }

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    window.addEventListener("resize", handleViewportResize);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
      window.removeEventListener("resize", handleViewportResize);
      document.body.classList.remove("is-resizing-devtools");
    };
  }, []);

  useEffect(() => {
    const frame = frameRef.current;
    if (!frame || typeof ResizeObserver === "undefined") {
      return;
    }

    const observer = new ResizeObserver(() => {
      frame.contentWindow?.dispatchEvent(new Event("resize"));
    });
    observer.observe(frame);
    return () => observer.disconnect();
  }, [frameUrl]);

  function beginResize(event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0) {
      return;
    }

    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    resizeStateRef.current = {
      handle: event.currentTarget,
      pointerId: event.pointerId,
      startPointer: event.clientX,
      startValue: panelWidthRef.current,
    };
    setIsResizing(true);
    document.body.classList.add("is-resizing-devtools");
  }

  function handleResizeKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    let nextWidth: number | null = null;
    if (event.key === "ArrowLeft") {
      nextWidth = clampPanelWidth(
        panelWidthRef.current + DEVTOOLS_PANEL_WIDTH_STEP,
      );
    } else if (event.key === "ArrowRight") {
      nextWidth = clampPanelWidth(
        panelWidthRef.current - DEVTOOLS_PANEL_WIDTH_STEP,
      );
    } else if (event.key === "Home") {
      nextWidth = DEVTOOLS_PANEL_MIN_WIDTH;
    } else if (event.key === "End") {
      nextWidth = panelWidthMaximum();
    }

    if (nextWidth == null) {
      return;
    }

    event.preventDefault();
    panelWidthRef.current = nextWidth;
    setPanelWidth(nextWidth);
    storePanelWidth(nextWidth);
  }

  function openDetachedInspector() {
    if (!frameUrl) {
      return;
    }
    window.open(frameUrl, "_blank", "noopener");
  }

  const statusMessage =
    error ||
    (!selectedSimulator
      ? "No simulator selected."
      : isLoading && targets.length === 0
        ? "Loading DevTools targets..."
        : targets.length === 0
          ? selectedSimulator.isBooted
            ? "No DevTools targets. Open Safari, enable inspectable WKWebViews, start Metro, or launch a Chrome remote debugging target."
            : "No DevTools targets. Boot the simulator for Safari/WebKit, or start Metro or Chrome remote debugging."
          : "");
  const panelStyle = {
    "--webkit-panel-width": `${panelWidth}px`,
  } as CSSProperties;

  return (
    <aside
      aria-label="DevTools"
      className={`webkit-panel devtools-panel ${isResizing ? "resizing" : ""}`}
      style={panelStyle}
    >
      <div
        aria-label="Resize DevTools"
        aria-orientation="vertical"
        aria-valuemax={panelWidthMaximum()}
        aria-valuemin={DEVTOOLS_PANEL_MIN_WIDTH}
        aria-valuenow={panelWidth}
        className="webkit-resize-x"
        onKeyDown={handleResizeKeyDown}
        onPointerDown={beginResize}
        role="separator"
        tabIndex={0}
        title="Resize DevTools"
      />

      <div className="webkit-targetbar">
        <select
          aria-label="DevTools Target"
          className="webkit-target-select"
          disabled={targets.length === 0}
          onChange={(event) => applySelectedTargetId(event.target.value)}
          value={selectedTarget?.id ?? ""}
        >
          {targets.length === 0 ? (
            <option value="">No targets</option>
          ) : (
            targets.map((target) => (
              <option key={target.id} value={target.id}>
                {targetLabel(target)}
              </option>
            ))
          )}
        </select>
        <button
          aria-label="Refresh DevTools Targets"
          className="tbtn icon-btn"
          disabled={isLoading}
          onClick={() => void loadTargets()}
          title="Refresh DevTools Targets"
          type="button"
        >
          <RefreshIcon />
        </button>
        <button
          aria-label="Open DevTools In New Tab"
          className="tbtn icon-btn"
          disabled={!frameUrl}
          onClick={openDetachedInspector}
          title="Open DevTools In New Tab"
          type="button"
        >
          <PopOutIcon />
        </button>
        <button
          aria-label="Close DevTools"
          className="tbtn icon-btn"
          onClick={onClose}
          title="Close DevTools"
          type="button"
        >
          <CloseIcon />
        </button>
      </div>

      {selectedTarget ? (
        <div className="webkit-target-meta">
          <span>{selectedTarget.source}</span>
          {selectedTarget.meta ? <span>{selectedTarget.meta}</span> : null}
        </div>
      ) : null}

      <div className="webkit-frame-wrap">
        {frameUrl ? (
          <>
            <iframe
              allow="clipboard-read; clipboard-write"
              className="webkit-frame"
              onLoad={() => setFrameLoaded(true)}
              ref={frameRef}
              src={frameUrl}
              title="DevTools"
            />
            {!frameLoaded ? (
              <div className="webkit-status" role="status">
                Loading DevTools...
              </div>
            ) : null}
          </>
        ) : (
          <div className={`webkit-status ${error ? "error" : ""}`}>
            {statusMessage}
          </div>
        )}
      </div>

      {discovery?.warnings.length ? (
        <div className="webkit-warnings">
          {discovery.warnings.map((warning) => (
            <div key={warning}>{warning}</div>
          ))}
        </div>
      ) : null}
    </aside>
  );
}

function mapChromeTarget(target: ChromeDevToolsTarget): DevToolsTarget {
  const source = sourceLabel(target.source);
  return {
    frameUrl: buildChromeDevToolsFrameUrl(target),
    id: `chrome:${target.id}`,
    meta: target.bundleIdentifier ?? target.url,
    source,
    title: chromeTargetLabel(target),
  };
}

function mapWebKitTarget(target: WebKitTarget): DevToolsTarget {
  return {
    frameUrl: buildWebKitInspectorFrameUrl(target),
    id: `webkit:${target.id}`,
    meta: target.url ?? "",
    source: webKitTargetKindLabel(target),
    title: webKitTargetLabel(target),
  };
}

function readStoredPanelWidth(): number {
  if (typeof window === "undefined") {
    return DEVTOOLS_PANEL_DEFAULT_WIDTH;
  }

  for (const storageKey of [
    DEVTOOLS_PANEL_WIDTH_STORAGE_KEY,
    ...LEGACY_PANEL_WIDTH_STORAGE_KEYS,
  ]) {
    const value = Number.parseFloat(
      window.localStorage.getItem(storageKey) ?? "",
    );
    if (Number.isFinite(value)) {
      return clampPanelWidth(value);
    }
  }
  return clampPanelWidth(DEVTOOLS_PANEL_DEFAULT_WIDTH);
}

function storePanelWidth(width: number): void {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(DEVTOOLS_PANEL_WIDTH_STORAGE_KEY, String(width));
}

function clampPanelWidth(width: number): number {
  return Math.round(
    Math.min(Math.max(width, DEVTOOLS_PANEL_MIN_WIDTH), panelWidthMaximum()),
  );
}

function panelWidthMaximum(): number {
  if (typeof window === "undefined") {
    return DEVTOOLS_PANEL_DEFAULT_WIDTH;
  }

  return Math.max(
    DEVTOOLS_PANEL_MIN_WIDTH,
    Math.min(
      window.innerWidth * 0.82,
      window.innerWidth - DEVTOOLS_PANEL_MIN_VIEWPORT_WIDTH,
    ),
  );
}

function buildChromeDevToolsFrameUrl(target: ChromeDevToolsTarget): string {
  const url = frontendUrl(target.devtoolsFrontendUrl);
  const token = accessTokenFromLocation();
  if (!token) {
    return url.toString();
  }

  if (isSimDeckHttpUrl(url)) {
    url.searchParams.set("simdeckToken", token);
  }
  for (const paramName of ["ws", "wss"]) {
    const rawSocketUrl = url.searchParams.get(paramName);
    if (!rawSocketUrl) {
      continue;
    }

    const socketUrl = normalizeWebSocketUrl(rawSocketUrl, paramName, url);
    if (isSimDeckWebSocketUrl(socketUrl)) {
      socketUrl.searchParams.set("simdeckToken", token);
    }
    url.searchParams.set(paramName, devToolsSocketParam(socketUrl));
  }
  return url.toString();
}

function buildWebKitInspectorFrameUrl(target: WebKitTarget): string {
  const url = frontendUrl(target.inspectorUrl);
  const token = accessTokenFromLocation();
  if (!token) {
    return url.toString();
  }

  if (isSimDeckHttpUrl(url)) {
    url.searchParams.set("simdeckToken", token);
  }
  const rawSocketUrl = url.searchParams.get("ws");
  if (rawSocketUrl) {
    const socketUrl = normalizeWebSocketUrl(rawSocketUrl, "ws", url);
    if (isSimDeckWebSocketUrl(socketUrl)) {
      socketUrl.searchParams.set("simdeckToken", token);
    }
    url.searchParams.set("ws", socketUrl.toString());
  }
  return url.toString();
}

function frontendUrl(value: string): URL {
  if (value.startsWith("http://") || value.startsWith("https://")) {
    return new URL(value);
  }
  return new URL(apiUrl(value), window.location.href);
}

function normalizeWebSocketUrl(
  rawUrl: string,
  paramName: string,
  frontendUrlValue: URL,
): URL {
  if (rawUrl.startsWith("ws://") || rawUrl.startsWith("wss://")) {
    return new URL(rawUrl);
  }

  const base = new URL(frontendUrlValue);
  base.protocol =
    paramName === "wss" || base.protocol === "https:" ? "wss:" : "ws:";
  if (rawUrl.startsWith("/")) {
    return new URL(rawUrl, base);
  }
  return new URL(`${base.protocol}//${rawUrl}`);
}

function isSimDeckHttpUrl(url: URL): boolean {
  return url.host === simDeckBaseUrl().host;
}

function isSimDeckWebSocketUrl(url: URL): boolean {
  return url.host === simDeckBaseUrl().host;
}

function simDeckBaseUrl(): URL {
  return new URL(apiUrl("/"), window.location.href);
}

function devToolsSocketParam(socketUrl: URL): string {
  return `${socketUrl.host}${socketUrl.pathname}${socketUrl.search}${socketUrl.hash}`;
}

function targetLabel(target: DevToolsTarget): string {
  if (target.title.startsWith(`${target.source}:`)) {
    return target.title;
  }
  return `${target.source}: ${target.title}`;
}

function chromeTargetLabel(target: ChromeDevToolsTarget): string {
  const title = target.title?.trim();
  const appName = target.appName?.trim();
  if (title && appName && !title.includes(appName)) {
    return `${appName}: ${title}`;
  }
  return title || appName || `Process ${target.processIdentifier}`;
}

function webKitTargetLabel(target: WebKitTarget): string {
  const title = target.title?.trim();
  const url = target.url?.trim();
  const appName = target.appName?.trim();
  if (title && appName) {
    return `${appName}: ${title}`;
  }
  return title || url || appName || `Page ${target.pageId}`;
}

function sourceLabel(source: string): string {
  switch (source) {
    case "react-native":
      return "React Native";
    case "react-native-metro":
      return "React Native Metro";
    case "chrome-inspector":
      return "Chrome Inspector";
    case "nativescript":
      return "NativeScript";
    case "swiftui":
      return "SwiftUI";
    case "in-app-inspector":
      return "UIKit";
    default:
      return "App runtime";
  }
}

function webKitTargetKindLabel(target: WebKitTarget): string {
  if (target.kind === "safari-page" || target.appName === "Safari") {
    return "Safari";
  }
  if (target.kind === "web-content-proxy") {
    return "WebKit proxy";
  }
  return target.appName ?? "WebKit";
}

function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Failed to load DevTools targets.";
}

function requestWithTimeout<T>(
  request: (signal: AbortSignal) => Promise<T>,
  timeoutMs: number,
  message: string,
): Promise<T> {
  const controller = new AbortController();
  const timer = window.setTimeout(() => controller.abort(), timeoutMs);
  return request(controller.signal)
    .catch((error: unknown) => {
      if (controller.signal.aborted) {
        throw new Error(message);
      }
      throw error;
    })
    .finally(() => window.clearTimeout(timer));
}

function mergeWarnings(...groups: string[][]): string[] {
  const seen = new Set<string>();
  return groups.flat().filter((warning) => {
    if (seen.has(warning)) {
      return false;
    }
    seen.add(warning);
    return true;
  });
}

function RefreshIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M13 4.5V2h-2.5M3 11.5V14h2.5M12.4 6A4.7 4.7 0 0 0 4.2 4.2L3 5.4m.6 4.6a4.7 4.7 0 0 0 8.2 1.8L13 10.6"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.4"
      />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M4.5 4.5 11.5 11.5M11.5 4.5 4.5 11.5"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="1.5"
      />
    </svg>
  );
}

function PopOutIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M6 4H3.8c-.4 0-.8.4-.8.8v7.4c0 .4.4.8.8.8h7.4c.4 0 .8-.4.8-.8V10M9 3h4v4M8 8l5-5"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.4"
      />
    </svg>
  );
}
