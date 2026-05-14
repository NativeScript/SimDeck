import {
  Cross2Icon,
  HomeIcon as RadixHomeIcon,
  ReloadIcon,
} from "@radix-ui/react-icons";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  CSSProperties,
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
} from "react";

import { accessTokenFromLocation } from "../../api/client";
import { apiUrl } from "../../api/config";
import {
  fetchAccessibilityPoint,
  fetchChromeDevToolsTargets,
  fetchWebKitTargets,
} from "../../api/simulators";
import type {
  AccessibilityNode,
  AccessibilityTreeResponse,
  ChromeDevToolsTarget,
  ChromeDevToolsTargetDiscovery,
  SimulatorMetadata,
  WebKitTarget,
  WebKitTargetDiscovery,
} from "../../api/types";
import { usePanelPresence } from "../../shared/hooks/usePanelPresence";

const DEVTOOLS_TARGET_REFRESH_MS = 750;
const SAFARI_ACTIVE_URL_REFRESH_MS = 1200;
const SAFARI_ACTIVE_URL_REQUEST_TIMEOUT_MS = 1800;
const CHROME_DEVTOOLS_REQUEST_TIMEOUT_MS = 6000;
const WEBKIT_DEVTOOLS_REQUEST_TIMEOUT_MS = 6000;
const DEVTOOLS_EMPTY_DISCOVERY_GRACE_MS = 8000;
const SAFARI_AUTO_TARGET_ID = "webkit:safari:auto";
const DEVTOOLS_PANEL_WIDTH_STORAGE_KEY = "xcw-devtools-panel-width";
const LEGACY_PANEL_WIDTH_STORAGE_KEYS = [
  "xcw-chrome-devtools-panel-width",
  "xcw-webkit-panel-width",
];
const DEVTOOLS_PANEL_DEFAULT_WIDTH = 720;
const DEVTOOLS_PANEL_MIN_WIDTH = 420;
const DEVTOOLS_PANEL_MIN_VIEWPORT_WIDTH = 340;
const DEVTOOLS_PANEL_WIDTH_STEP = 40;
const NOT_CONNECTED_MESSAGE = "Not connected";

interface DevToolsPanelProps {
  disconnected: boolean;
  onClose: () => void;
  selectedSimulator: SimulatorMetadata | null;
  visible: boolean;
}

interface ResizeState {
  handle: HTMLDivElement;
  pointerId: number;
  startPointer: number;
  startValue: number;
}

export interface DevToolsTarget {
  appId?: string | null;
  appActive?: boolean;
  appName?: string | null;
  bundleIdentifier?: string | null;
  frameUrl: string;
  id: string;
  meta: string;
  pageActive?: boolean;
  pageId?: number;
  processIdentifier?: number | null;
  safariAuto?: boolean;
  source: string;
  title: string;
  url?: string | null;
}

interface DevToolsDiscovery {
  targets: DevToolsTarget[];
  warnings: string[];
}

export interface DevToolsTargetSelectionInput {
  currentForegroundKey: string;
  currentTargetId: string;
  foregroundApp: ChromeDevToolsTargetDiscovery["foregroundApp"];
  manualOverride: boolean;
  pendingForegroundApp: ChromeDevToolsTargetDiscovery["foregroundApp"];
  pendingForegroundKey: string;
  targets: DevToolsTarget[];
}

export interface DevToolsTargetSelection {
  automaticTargetId: string;
  shouldClearPendingForeground: boolean;
  targetId: string;
}

type ChromeDiscoveryResult =
  PromiseSettledResult<ChromeDevToolsTargetDiscovery>;
type WebKitDiscoveryResult = PromiseSettledResult<WebKitTargetDiscovery>;

export function DevToolsPanel({
  disconnected,
  onClose,
  selectedSimulator,
  visible,
}: DevToolsPanelProps) {
  const [panelWidth, setPanelWidth] = useState(readStoredPanelWidth);
  const [isResizing, setIsResizing] = useState(false);
  const [discovery, setDiscovery] = useState<DevToolsDiscovery | null>(null);
  const [selectedTargetId, setSelectedTargetId] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isWebKitLoading, setIsWebKitLoading] = useState(false);
  const [error, setError] = useState("");
  const [frameInstanceKey, setFrameInstanceKey] = useState(0);
  const [frameLoaded, setFrameLoaded] = useState(false);
  const [overviewVisible, setOverviewVisible] = useState(false);
  const discoveryRef = useRef<DevToolsDiscovery | null>(null);
  const emptyDiscoveryGraceUntilRef = useRef(0);
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const activeWebKitUrlHintRef = useRef("");
  const loadingActiveWebKitUrlRef = useRef(false);
  const loadingTargetsRef = useRef(false);
  const loadingWebKitTargetsRef = useRef(false);
  const panelWidthRef = useRef(panelWidth);
  const requestIdRef = useRef(0);
  const resizeStateRef = useRef<ResizeState | null>(null);
  const selectedSimulatorBootedRef = useRef(false);
  const selectedSimulatorUdidRef = useRef<string | null>(null);
  const stableDiscoveryAtRef = useRef(0);
  const stableDiscoveryRef = useRef<DevToolsDiscovery | null>(null);
  const overviewPinnedRef = useRef(false);
  const foregroundKeyRef = useRef("");
  const foregroundAppRef =
    useRef<ChromeDevToolsTargetDiscovery["foregroundApp"]>(null);
  const pendingForegroundAppRef =
    useRef<ChromeDevToolsTargetDiscovery["foregroundApp"]>(null);
  const pendingForegroundKeyRef = useRef("");
  const selectedTargetIdRef = useRef("");
  const manualTargetSelectionRef = useRef(false);
  const { isPresent, panelState } = usePanelPresence(visible);

  const targets = discovery?.targets ?? [];
  const selectedTarget = useMemo(() => {
    if (targets.length === 0) {
      return null;
    }
    return (
      targets.find((target) => target.id === selectedTargetId) ?? targets[0]
    );
  }, [selectedTargetId, targets]);
  const frameUrl =
    visible && !overviewVisible ? (selectedTarget?.frameUrl ?? "") : "";

  useEffect(() => {
    panelWidthRef.current = panelWidth;
  }, [panelWidth]);

  const applyDiscovery = useCallback(
    (nextDiscovery: DevToolsDiscovery | null) => {
      discoveryRef.current = nextDiscovery;
      if (nextDiscovery?.targets.length) {
        stableDiscoveryRef.current = nextDiscovery;
        stableDiscoveryAtRef.current = Date.now();
        emptyDiscoveryGraceUntilRef.current = 0;
      } else {
        stableDiscoveryRef.current = null;
        stableDiscoveryAtRef.current = 0;
      }
      setDiscovery(nextDiscovery);
    },
    [],
  );

  const applySelectedTargetId = useCallback((nextTargetId: string) => {
    selectedTargetIdRef.current = nextTargetId;
    setSelectedTargetId(nextTargetId);
  }, []);

  const applyActiveWebKitUrlHint = useCallback(
    (nextHint: string) => {
      if (activeWebKitUrlHintRef.current === nextHint) {
        return;
      }
      activeWebKitUrlHintRef.current = nextHint;
      const currentDiscovery = discoveryRef.current;
      if (!currentDiscovery) {
        return;
      }
      applyDiscovery({
        ...currentDiscovery,
        targets: withSafariAutoTarget(currentDiscovery.targets, nextHint),
      });
    },
    [applyDiscovery],
  );

  const resetTargetDiscovery = useCallback(
    (options: { holdEmptyGrace?: boolean } = {}) => {
      requestIdRef.current += 1;
      loadingTargetsRef.current = false;
      loadingWebKitTargetsRef.current = false;
      loadingActiveWebKitUrlRef.current = false;
      emptyDiscoveryGraceUntilRef.current = options.holdEmptyGrace
        ? Date.now() + DEVTOOLS_EMPTY_DISCOVERY_GRACE_MS
        : 0;
      applyDiscovery(null);
      applySelectedTargetId("");
      activeWebKitUrlHintRef.current = "";
      foregroundKeyRef.current = "";
      foregroundAppRef.current = null;
      pendingForegroundKeyRef.current = "";
      pendingForegroundAppRef.current = null;
      manualTargetSelectionRef.current = false;
      setError("");
      setFrameLoaded(false);
      setIsLoading(false);
      setIsWebKitLoading(false);
      setOverviewVisible(false);
    },
    [applyDiscovery, applySelectedTargetId],
  );

  const loadTargets = useCallback(async () => {
    if (disconnected) {
      resetTargetDiscovery();
      return;
    }

    if (!selectedSimulator) {
      resetTargetDiscovery();
      return;
    }

    if (!selectedSimulator.isBooted) {
      resetTargetDiscovery();
      return;
    }

    if (loadingTargetsRef.current) {
      return;
    }

    loadingTargetsRef.current = true;
    const shouldLoadWebKit =
      selectedSimulator.isBooted && !loadingWebKitTargetsRef.current;
    if (shouldLoadWebKit) {
      loadingWebKitTargetsRef.current = true;
      setIsWebKitLoading(true);
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
      const webKitTargets = shouldLoadWebKit
        ? requestWithTimeout(
            (signal) => fetchWebKitTargets(selectedSimulator.udid, { signal }),
            WEBKIT_DEVTOOLS_REQUEST_TIMEOUT_MS,
            "Timed out loading WebKit targets.",
          )
        : null;
      const chromeResultPromise = settleDiscovery(chromeTargets);
      const webKitResultPromise = webKitTargets
        ? settleDiscovery(webKitTargets)
        : null;
      const applyTargetResults = ({
        chromeResult,
        webKitResult,
      }: {
        chromeResult?: ChromeDiscoveryResult;
        webKitResult?: WebKitDiscoveryResult;
      }) => {
        const isBackgroundWebKitResult = !chromeResult && Boolean(webKitResult);
        if (
          requestId !== requestIdRef.current &&
          !(
            isBackgroundWebKitResult &&
            selectedSimulatorUdidRef.current === selectedSimulator.udid &&
            selectedSimulatorBootedRef.current
          )
        ) {
          return;
        }

        const previousDiscovery = discoveryRef.current;
        const previousTargets = previousDiscovery?.targets ?? [];
        const nextTargets: DevToolsTarget[] = [];
        let currentForegroundKey = foregroundKeyRef.current;
        let warnings: string[] = [];
        let errors: string[] = [];
        let providerDisconnectedError = false;

        if (chromeResult) {
          if (chromeResult.status === "fulfilled") {
            const foregroundApp = chromeResult.value.foregroundApp ?? null;
            foregroundAppRef.current = foregroundApp;
            currentForegroundKey = foregroundAppKey(foregroundApp);
            if (currentForegroundKey !== foregroundKeyRef.current) {
              foregroundKeyRef.current = currentForegroundKey;
              pendingForegroundKeyRef.current = currentForegroundKey;
              pendingForegroundAppRef.current = foregroundApp;
            }
            nextTargets.push(
              ...chromeResult.value.targets.map(mapChromeTarget),
            );
            warnings = warnings.concat(chromeResult.value.warnings);
          } else {
            const message = errorMessage(chromeResult.reason);
            providerDisconnectedError ||=
              isProviderDisconnectedMessage(message);
            const staleChromeTargets = providerDisconnectedError
              ? []
              : previousTargets.filter(isChromeTarget);
            if (staleChromeTargets.length > 0) {
              nextTargets.push(...staleChromeTargets);
            } else {
              errors = errors.concat(message);
            }
          }
        } else {
          nextTargets.push(...previousTargets.filter(isChromeTarget));
        }

        if (webKitResult) {
          if (webKitResult.status === "fulfilled") {
            nextTargets.push(
              ...webKitResult.value.targets.map(mapWebKitTarget),
            );
            warnings = warnings.concat(webKitResult.value.warnings);
          } else {
            const message = errorMessage(webKitResult.reason);
            providerDisconnectedError ||=
              isProviderDisconnectedMessage(message);
            errors = errors.concat(message);
          }
        } else {
          nextTargets.push(...previousTargets.filter(isWebKitTarget));
        }

        if (providerDisconnectedError) {
          applyDiscovery(null);
          applySelectedTargetId("");
          manualTargetSelectionRef.current = false;
          setError(NOT_CONNECTED_MESSAGE);
          setFrameLoaded(false);
          setIsLoading(false);
          setIsWebKitLoading(false);
          setOverviewVisible(false);
          return;
        }

        warnings = cleanDevToolsMessages(warnings);
        errors = cleanDevToolsMessages(errors);

        const discoverableTargets = withSafariAutoTarget(
          nextTargets,
          activeWebKitUrlHintRef.current,
        );
        const nextDiscovery = {
          targets: discoverableTargets,
          warnings: mergeWarnings(warnings, errors),
        };

        const webKitDiscoveryPending =
          !webKitResult && shouldLoadWebKit && loadingWebKitTargetsRef.current;
        const hasEmptyDiscovery =
          discoverableTargets.length === 0 && errors.length === 0;
        if (
          hasEmptyDiscovery &&
          selectedSimulator.isBooted &&
          (webKitDiscoveryPending ||
            Date.now() < emptyDiscoveryGraceUntilRef.current)
        ) {
          setError("");
          return;
        }

        if (
          hasEmptyDiscovery &&
          selectedSimulator.isBooted &&
          stableDiscoveryRef.current &&
          Date.now() - stableDiscoveryAtRef.current <
            DEVTOOLS_EMPTY_DISCOVERY_GRACE_MS
        ) {
          setError("");
          discoveryRef.current = stableDiscoveryRef.current;
          setDiscovery(stableDiscoveryRef.current);
          return;
        }

        applyDiscovery(nextDiscovery);
        const selection = resolveDevToolsTargetSelection({
          currentForegroundKey,
          currentTargetId: selectedTargetIdRef.current,
          foregroundApp: foregroundAppRef.current,
          manualOverride: manualTargetSelectionRef.current,
          pendingForegroundApp: pendingForegroundAppRef.current,
          pendingForegroundKey: pendingForegroundKeyRef.current,
          targets: discoverableTargets,
        });
        if (selection.shouldClearPendingForeground) {
          pendingForegroundKeyRef.current = "";
          pendingForegroundAppRef.current = null;
        }
        if (selection.automaticTargetId && !overviewPinnedRef.current) {
          setOverviewVisible(false);
        }
        applySelectedTargetId(selection.targetId);
        if (discoverableTargets.length === 0 && errors.length > 0) {
          setError(errors.join(" "));
        }
      };

      const pendingResults: Promise<void>[] = [
        chromeResultPromise.then((chromeResult) => {
          applyTargetResults({ chromeResult });
        }),
      ];
      if (webKitResultPromise) {
        pendingResults.push(
          webKitResultPromise.then((webKitResult) => {
            applyTargetResults({ webKitResult });
          }),
        );
      }
      await Promise.all(pendingResults);
    } catch (targetError) {
      if (requestId !== requestIdRef.current) {
        return;
      }
      const message = errorMessage(targetError);
      if (isProviderDisconnectedMessage(message)) {
        applyDiscovery(null);
        applySelectedTargetId("");
        manualTargetSelectionRef.current = false;
        setError(NOT_CONNECTED_MESSAGE);
        setFrameLoaded(false);
        setIsLoading(false);
        setIsWebKitLoading(false);
        setOverviewVisible(false);
        return;
      }
      const previousDiscovery = discoveryRef.current;
      if (previousDiscovery && previousDiscovery.targets.length > 0) {
        applyDiscovery({
          ...previousDiscovery,
          warnings: mergeWarnings(
            cleanDevToolsMessages(previousDiscovery.warnings),
            cleanDevToolsMessages([message]),
          ),
        });
        return;
      }
      applyDiscovery(null);
      applySelectedTargetId("");
      manualTargetSelectionRef.current = false;
      setError(userFacingDevToolsMessage(message));
    } finally {
      if (requestId === requestIdRef.current) {
        setIsLoading(false);
      }
      loadingTargetsRef.current = false;
      if (shouldLoadWebKit) {
        loadingWebKitTargetsRef.current = false;
        setIsWebKitLoading(false);
      }
    }
  }, [
    disconnected,
    resetTargetDiscovery,
    selectedSimulator?.isBooted,
    selectedSimulator?.udid,
  ]);

  const loadActiveSafariUrlHint = useCallback(async () => {
    if (
      disconnected ||
      !visible ||
      !selectedSimulator?.isBooted ||
      !selectedSimulator.udid ||
      manualTargetSelectionRef.current ||
      loadingActiveWebKitUrlRef.current
    ) {
      return;
    }

    const currentTargets = discoveryRef.current?.targets ?? [];
    if (!currentTargets.some(isSafariAutoTarget)) {
      return;
    }
    if (!currentTargets.some(isConcreteSafariTarget)) {
      applyActiveWebKitUrlHint("");
      return;
    }

    const point = safariActiveUrlProbePoint(selectedSimulator);
    loadingActiveWebKitUrlRef.current = true;
    try {
      const snapshot = await requestWithTimeout(
        (signal) =>
          fetchAccessibilityPoint(selectedSimulator.udid, point.x, point.y, {
            maxDepth: 0,
            signal,
          }),
        SAFARI_ACTIVE_URL_REQUEST_TIMEOUT_MS,
        "Timed out loading Safari active URL.",
      );
      const hint = activeUrlHintFromAccessibilitySnapshot(snapshot);
      if (hint) {
        applyActiveWebKitUrlHint(hint);
      }
    } catch {
      // Safari tab matching is opportunistic; target discovery remains usable.
    } finally {
      loadingActiveWebKitUrlRef.current = false;
    }
  }, [
    applyActiveWebKitUrlHint,
    disconnected,
    selectedSimulator,
    selectedSimulator?.isBooted,
    selectedSimulator?.udid,
    visible,
  ]);

  useEffect(() => {
    selectedSimulatorUdidRef.current = selectedSimulator?.udid ?? null;
    selectedSimulatorBootedRef.current = Boolean(selectedSimulator?.isBooted);
    overviewPinnedRef.current = false;
    resetTargetDiscovery({
      holdEmptyGrace: Boolean(selectedSimulator?.isBooted),
    });
  }, [
    resetTargetDiscovery,
    selectedSimulator?.isBooted,
    selectedSimulator?.udid,
  ]);

  useEffect(() => {
    if (!disconnected) {
      emptyDiscoveryGraceUntilRef.current =
        Date.now() + DEVTOOLS_EMPTY_DISCOVERY_GRACE_MS;
      return;
    }
    resetTargetDiscovery();
  }, [disconnected, resetTargetDiscovery]);

  useEffect(() => {
    if (!visible || !selectedSimulator?.isBooted) {
      return;
    }
    void loadTargets();
    const interval = window.setInterval(() => {
      void loadTargets();
    }, DEVTOOLS_TARGET_REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [loadTargets, selectedSimulator?.isBooted, visible]);

  useEffect(() => {
    if (!visible || !selectedSimulator?.isBooted) {
      return;
    }
    void loadActiveSafariUrlHint();
    const interval = window.setInterval(() => {
      void loadActiveSafariUrlHint();
    }, SAFARI_ACTIVE_URL_REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [loadActiveSafariUrlHint, selectedSimulator?.isBooted, visible]);

  useEffect(() => {
    if (visible) {
      return;
    }
    manualTargetSelectionRef.current = false;
    pendingForegroundKeyRef.current = "";
    pendingForegroundAppRef.current = null;
  }, [visible]);

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

  function openTarget(targetId: string) {
    manualTargetSelectionRef.current = targetId !== SAFARI_AUTO_TARGET_ID;
    overviewPinnedRef.current = false;
    pendingForegroundKeyRef.current = "";
    pendingForegroundAppRef.current = null;
    applySelectedTargetId(targetId);
    setFrameInstanceKey((current) => current + 1);
    setOverviewVisible(false);
  }

  function showOverview() {
    overviewPinnedRef.current = true;
    setOverviewVisible(true);
  }

  const isHoldingEmptyDiscovery =
    discovery === null &&
    Boolean(selectedSimulator?.isBooted) &&
    Date.now() < emptyDiscoveryGraceUntilRef.current;
  const isDiscoveringTargets =
    discovery === null &&
    (isLoading || isWebKitLoading || isHoldingEmptyDiscovery);
  const effectivelyDisconnected =
    disconnected || error === NOT_CONNECTED_MESSAGE;
  const chromeDevToolsBlocked = Boolean(
    selectedTarget && isChromeTarget(selectedTarget) && isSafariBrowser(),
  );
  const safariAutoWaiting = Boolean(
    selectedTarget &&
    isSafariAutoTarget(selectedTarget) &&
    !selectedTarget.frameUrl,
  );
  const statusMessage = effectivelyDisconnected
    ? NOT_CONNECTED_MESSAGE
    : chromeDevToolsBlocked
      ? "Chrome DevTools don't work in Safari"
      : error ||
        (!selectedSimulator
          ? "No simulator selected."
          : isDiscoveringTargets && targets.length === 0
            ? "Connecting..."
            : safariAutoWaiting
              ? "Finding current Safari tab..."
              : targets.length === 0
                ? selectedSimulator.isBooted
                  ? "No DevTools targets. Open Safari, enable inspectable WKWebViews, start Metro, or launch a Chrome remote debugging target."
                  : "No DevTools targets. Boot the simulator for Safari/WebKit, or start Metro or Chrome remote debugging."
                : "");
  const emptyOverviewMessage = effectivelyDisconnected
    ? NOT_CONNECTED_MESSAGE
    : isDiscoveringTargets
      ? "Connecting..."
      : "No targets";
  const displayWarnings = selectedSimulator?.isBooted
    ? (discovery?.warnings ?? []).filter(shouldDisplayDevToolsWarning)
    : [];
  const panelStyle = {
    "--webkit-panel-width": `${panelWidth}px`,
  } as CSSProperties;

  if (!isPresent) {
    return null;
  }

  return (
    <aside
      aria-label="DevTools"
      className={`webkit-panel devtools-panel ${isResizing ? "resizing" : ""}`}
      data-state={panelState}
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
        <button
          aria-label="DevTools Home"
          className={`tbtn icon-btn ${overviewVisible ? "active" : ""}`}
          onClick={showOverview}
          title="DevTools Home"
          type="button"
        >
          <RadixHomeIcon />
        </button>
        <select
          aria-label="DevTools Target"
          className="webkit-target-select"
          disabled={targets.length === 0}
          onChange={(event) => {
            openTarget(event.target.value);
          }}
          value={selectedTarget?.id ?? ""}
        >
          {targets.length === 0 ? (
            <option value="">
              {isDiscoveringTargets ? "Connecting..." : "No targets"}
            </option>
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
          onClick={() => {
            manualTargetSelectionRef.current = false;
            pendingForegroundKeyRef.current = "";
            pendingForegroundAppRef.current = null;
            emptyDiscoveryGraceUntilRef.current =
              Date.now() + DEVTOOLS_EMPTY_DISCOVERY_GRACE_MS;
            setFrameLoaded(false);
            setFrameInstanceKey((current) => current + 1);
            void loadTargets();
          }}
          title="Refresh DevTools Targets"
          type="button"
        >
          <ReloadIcon />
        </button>
        <button
          aria-label="Close DevTools"
          className="tbtn icon-btn"
          onClick={onClose}
          title="Close DevTools"
          type="button"
        >
          <Cross2Icon />
        </button>
      </div>

      {selectedTarget && !overviewVisible ? (
        <div className="webkit-target-meta">
          <span>{selectedTarget.source}</span>
          {selectedTarget.meta ? <span>{selectedTarget.meta}</span> : null}
        </div>
      ) : null}

      <div className="webkit-frame-wrap">
        {effectivelyDisconnected || chromeDevToolsBlocked ? (
          <div className={`webkit-status ${error ? "error" : ""}`}>
            {statusMessage}
          </div>
        ) : overviewVisible ? (
          <DevToolsOverview
            emptyMessage={emptyOverviewMessage}
            targets={targets}
            onSelectTarget={openTarget}
          />
        ) : frameUrl ? (
          <>
            <iframe
              allow="clipboard-read; clipboard-write"
              className="webkit-frame"
              key={`${frameUrl}:${frameInstanceKey}`}
              onLoad={() => setFrameLoaded(true)}
              ref={frameRef}
              src={frameUrl}
              title="DevTools"
            />
            {!frameLoaded ? (
              <div className="webkit-status" role="status">
                Connecting...
              </div>
            ) : null}
          </>
        ) : (
          <div className={`webkit-status ${error ? "error" : ""}`}>
            {statusMessage}
          </div>
        )}
      </div>

      {displayWarnings.length ? (
        <div className="webkit-warnings">
          {displayWarnings.map((warning) => (
            <div key={warning}>{warning}</div>
          ))}
        </div>
      ) : null}
    </aside>
  );
}

interface DevToolsOverviewProps {
  emptyMessage: string;
  onSelectTarget: (targetId: string) => void;
  targets: DevToolsTarget[];
}

function DevToolsOverview({
  emptyMessage,
  onSelectTarget,
  targets,
}: DevToolsOverviewProps) {
  if (targets.length === 0) {
    return (
      <div className="devtools-overview empty">
        <div className="webkit-status">{emptyMessage}</div>
      </div>
    );
  }

  return (
    <div className="devtools-overview">
      <div className="devtools-overview-list">
        {targets.map((target) => (
          <button
            className="devtools-overview-card"
            key={target.id}
            onClick={() => onSelectTarget(target.id)}
            type="button"
          >
            <span className="devtools-overview-card-source">
              {target.source}
            </span>
            <span className="devtools-overview-card-title">{target.title}</span>
            {target.meta ? (
              <span className="devtools-overview-card-meta">{target.meta}</span>
            ) : null}
          </button>
        ))}
      </div>
    </div>
  );
}

function mapChromeTarget(target: ChromeDevToolsTarget): DevToolsTarget {
  const source = sourceLabel(target.source);
  return {
    appName: target.appName ?? null,
    bundleIdentifier: target.bundleIdentifier ?? null,
    frameUrl: buildChromeDevToolsFrameUrl(target),
    id: `chrome:${target.id}`,
    meta: target.bundleIdentifier ?? target.url,
    processIdentifier: target.processIdentifier,
    source,
    title: chromeTargetLabel(target),
  };
}

function mapWebKitTarget(target: WebKitTarget): DevToolsTarget {
  return {
    appId: target.appId,
    appActive: target.appActive ?? false,
    appName: target.appName ?? null,
    frameUrl: buildWebKitInspectorFrameUrl(target),
    id: `webkit:${target.id}`,
    meta: target.url ?? "",
    pageActive: target.pageActive ?? false,
    pageId: target.pageId,
    processIdentifier: webKitTargetProcessIdentifier(target),
    source: webKitTargetKindLabel(target),
    title: webKitTargetLabel(target),
    url: target.url ?? null,
  };
}

export function withSafariAutoTarget(
  targets: DevToolsTarget[],
  activeUrlHint = "",
): DevToolsTarget[] {
  const concreteTargets = targets.filter(
    (target) => !isSafariAutoTarget(target),
  );
  const safariTargets = concreteTargets.filter(
    (target) => target.source === "Safari",
  );
  if (safariTargets.length === 0) {
    return concreteTargets;
  }

  const activeTarget =
    safariTargets.find((target) => target.pageActive) ??
    bestWebKitTargetForUrlHint(safariTargets, activeUrlHint);
  const newestTarget = safariTargets
    .slice()
    .sort((left, right) => (right.pageId ?? 0) - (left.pageId ?? 0))[0];
  const fallbackTarget = activeTarget ?? newestTarget ?? safariTargets[0];
  const autoTarget: DevToolsTarget = {
    appId: fallbackTarget?.appId ?? "com.apple.mobilesafari",
    appActive: safariTargets.some((target) => target.appActive),
    appName: fallbackTarget?.appName ?? "Safari",
    bundleIdentifier:
      fallbackTarget?.bundleIdentifier ?? "com.apple.mobilesafari",
    frameUrl: fallbackTarget?.frameUrl ?? "",
    id: SAFARI_AUTO_TARGET_ID,
    meta: fallbackTarget?.url ?? "Latest Safari tab",
    pageActive: Boolean(activeTarget),
    pageId: fallbackTarget?.pageId,
    processIdentifier: fallbackTarget?.processIdentifier ?? null,
    safariAuto: true,
    source: "Safari",
    title: "Auto",
    url: fallbackTarget?.url ?? null,
  };
  return [autoTarget, ...concreteTargets];
}

export function resolveDevToolsTargetSelection({
  currentForegroundKey,
  currentTargetId,
  foregroundApp,
  manualOverride,
  pendingForegroundApp,
  pendingForegroundKey,
  targets,
}: DevToolsTargetSelectionInput): DevToolsTargetSelection {
  const currentTarget = targets.find((target) => target.id === currentTargetId);
  if (manualOverride) {
    return {
      automaticTargetId: "",
      shouldClearPendingForeground: false,
      targetId: currentTarget?.id || targets[0]?.id || "",
    };
  }

  const pendingForegroundMatches =
    Boolean(pendingForegroundApp) &&
    Boolean(pendingForegroundKey) &&
    pendingForegroundKey === currentForegroundKey;
  const safariAutoTarget = targets.find(isSafariAutoTarget);
  const safariForeground =
    isSafariForegroundApp(foregroundApp) ||
    (pendingForegroundMatches && isSafariForegroundApp(pendingForegroundApp));
  if (safariAutoTarget && safariForeground) {
    return {
      automaticTargetId: safariAutoTarget.id,
      shouldClearPendingForeground: Boolean(
        pendingForegroundMatches && isSafariForegroundApp(pendingForegroundApp),
      ),
      targetId: safariAutoTarget.id,
    };
  }

  const compatibleTarget =
    pendingForegroundApp && pendingForegroundMatches
      ? highlyCompatibleTargetForForeground(
          targets,
          pendingForegroundApp,
          currentTargetId,
        )
      : isSafariForegroundApp(foregroundApp)
        ? highlyCompatibleTargetForForeground(
            targets,
            foregroundApp,
            currentTargetId,
          )
        : null;
  const automaticTarget = compatibleTarget;

  return {
    automaticTargetId: automaticTarget?.id ?? "",
    shouldClearPendingForeground: Boolean(compatibleTarget),
    targetId: automaticTarget?.id || currentTarget?.id || targets[0]?.id || "",
  };
}

function highlyCompatibleTargetForForeground(
  targets: DevToolsTarget[],
  foregroundApp: ChromeDevToolsTargetDiscovery["foregroundApp"],
  currentTargetId = "",
): DevToolsTarget | null {
  if (!foregroundApp) {
    return null;
  }
  const scoredTargets = targets
    .map((target) => ({
      score: foregroundCompatibilityScore(target, foregroundApp),
      target,
    }))
    .filter(({ score }) => score >= 85)
    .sort((left, right) => right.score - left.score);
  const currentTarget = scoredTargets.find(
    ({ target }) => target.id === currentTargetId,
  );
  const bestTarget = scoredTargets[0] ?? null;
  if (!bestTarget) {
    return null;
  }
  if (currentTarget && currentTarget.score >= bestTarget.score) {
    return currentTarget.target;
  }
  return bestTarget.target;
}

function bestWebKitTargetForUrlHint(
  targets: DevToolsTarget[],
  urlHint: string,
): DevToolsTarget | null {
  const hint = urlHint.trim();
  if (!hint) {
    return null;
  }
  const scoredTargets = targets
    .filter(isConcreteSafariTarget)
    .map((target) => ({
      score: safariActiveUrlMatchScore(hint, target),
      target,
    }))
    .filter(({ score }) => score > 0)
    .sort((left, right) => right.score - left.score);
  return scoredTargets[0]?.target ?? null;
}

function safariActiveUrlMatchScore(
  urlHint: string,
  target: DevToolsTarget,
): number {
  const targetUrl = target.url?.trim() ?? "";
  if (!targetUrl) {
    return 0;
  }
  const hintKey = normalizedUrlKey(urlHint);
  const targetKey = normalizedUrlKey(targetUrl);
  if (!hintKey || !targetKey) {
    return 0;
  }
  if (hintKey === targetKey) {
    return 100;
  }

  const hintHost = normalizedUrlHost(hintKey);
  const targetHost = normalizedUrlHost(targetKey);
  if (hintHost && hintHost === targetHost) {
    return 90;
  }
  if (
    targetKey.includes(hintKey) ||
    (targetHost && hintKey.includes(targetHost))
  ) {
    return 80;
  }
  return 0;
}

function foregroundCompatibilityScore(
  target: DevToolsTarget,
  foregroundApp: NonNullable<ChromeDevToolsTargetDiscovery["foregroundApp"]>,
): number {
  let score = 0;
  const foregroundBundle = foregroundApp.bundleIdentifier?.trim() ?? "";
  const foregroundAppName = foregroundApp.appName?.trim() ?? "";
  const foregroundPid = foregroundApp.processIdentifier;
  const webKitMatchesForeground =
    isWebKitTarget(target) &&
    webKitTargetMatchesForegroundApp(target, foregroundApp);

  if (
    Number.isFinite(foregroundPid) &&
    foregroundPid > 0 &&
    target.processIdentifier === foregroundPid
  ) {
    score = Math.max(score, isWebKitTarget(target) ? 100 : 92);
  }

  if (foregroundBundle && target.bundleIdentifier === foregroundBundle) {
    score = Math.max(score, target.source === "React Native Metro" ? 98 : 90);
  }

  if (webKitMatchesForeground && target.appActive) {
    score = Math.max(score, 93);
  }

  if (webKitMatchesForeground && target.pageActive) {
    score = Math.max(score, 112);
  }

  if (
    isSafariForeground(foregroundApp) &&
    (target.source === "Safari" || (isWebKitTarget(target) && target.appActive))
  ) {
    score = Math.max(
      score,
      target.source === "Safari" && target.pageActive
        ? 120
        : target.source === "Safari" && target.appActive
          ? 97
          : 90,
    );
  }

  if (
    foregroundAppName &&
    isWebKitTarget(target) &&
    (target.appName === foregroundAppName ||
      target.title.startsWith(`${foregroundAppName}:`))
  ) {
    score = Math.max(score, 88);
  }

  return score;
}

function webKitTargetMatchesForegroundApp(
  target: DevToolsTarget,
  foregroundApp: NonNullable<ChromeDevToolsTargetDiscovery["foregroundApp"]>,
): boolean {
  const foregroundPid = foregroundApp.processIdentifier;
  if (
    Number.isFinite(foregroundPid) &&
    foregroundPid > 0 &&
    target.processIdentifier === foregroundPid
  ) {
    return true;
  }

  if (isSafariForeground(foregroundApp) && target.source === "Safari") {
    return true;
  }

  const foregroundAppName = foregroundApp.appName?.trim();
  return Boolean(
    foregroundAppName &&
    (target.appName === foregroundAppName ||
      target.title.startsWith(`${foregroundAppName}:`)),
  );
}

function foregroundAppKey(
  foregroundApp: ChromeDevToolsTargetDiscovery["foregroundApp"],
): string {
  if (!foregroundApp) {
    return "";
  }
  return (
    foregroundApp.bundleIdentifier?.trim() ||
    foregroundApp.appName?.trim() ||
    `pid:${foregroundApp.processIdentifier}`
  );
}

function isSafariForegroundApp(
  foregroundApp: ChromeDevToolsTargetDiscovery["foregroundApp"],
): foregroundApp is NonNullable<
  ChromeDevToolsTargetDiscovery["foregroundApp"]
> {
  return Boolean(foregroundApp && isSafariForeground(foregroundApp));
}

function isSafariForeground(
  foregroundApp: NonNullable<ChromeDevToolsTargetDiscovery["foregroundApp"]>,
): boolean {
  return (
    foregroundApp.bundleIdentifier === "com.apple.mobilesafari" ||
    foregroundApp.appName === "Safari" ||
    foregroundApp.appName === "MobileSafari"
  );
}

function safariActiveUrlProbePoint(simulator: SimulatorMetadata): {
  x: number;
  y: number;
} {
  const { width, height } = logicalScreenSizeFromSimulator(simulator);
  return {
    x: Math.max(width * 0.5, 1),
    y: Math.max(height - 54, 1),
  };
}

function logicalScreenSizeFromSimulator(simulator: SimulatorMetadata): {
  width: number;
  height: number;
} {
  const displayWidth = simulator.privateDisplay?.displayWidth ?? 0;
  const displayHeight = simulator.privateDisplay?.displayHeight ?? 0;
  const inferred = logicalScreenSizeFromDisplayPixels(
    displayWidth,
    displayHeight,
  );
  return inferred ?? { width: 402, height: 874 };
}

function logicalScreenSizeFromDisplayPixels(
  width: number,
  height: number,
): { width: number; height: number } | null {
  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }
  const shortEdge = Math.min(width, height);
  const longEdge = Math.max(width, height);
  const scale =
    shortEdge <= 1320 && longEdge >= 1800
      ? 3
      : shortEdge >= 700 && longEdge >= 1000
        ? 2
        : 1;
  return { width: width / scale, height: height / scale };
}

function activeUrlHintFromAccessibilitySnapshot(
  snapshot: AccessibilityTreeResponse,
): string {
  for (const root of snapshot.roots ?? []) {
    const hint = activeUrlHintFromAccessibilityNode(root);
    if (hint) {
      return hint;
    }
  }
  return "";
}

function activeUrlHintFromAccessibilityNode(node: AccessibilityNode): string {
  const values = node as AccessibilityNode & Record<string, unknown>;
  for (const key of [
    "AXValue",
    "value",
    "url",
    "AXLabel",
    "label",
    "title",
    "name",
  ]) {
    const hint = sanitizeActiveUrlHint(values[key]);
    if (hint) {
      return hint;
    }
  }
  for (const child of node.children ?? []) {
    const hint = activeUrlHintFromAccessibilityNode(child);
    if (hint) {
      return hint;
    }
  }
  return "";
}

function sanitizeActiveUrlHint(value: unknown): string {
  if (typeof value !== "string") {
    return "";
  }
  const cleaned = value
    .replace(/[\u0000-\u001f\u007f\u200e\u200f\u202a-\u202e\u2066-\u2069]/g, "")
    .trim();
  if (!cleaned || cleaned.toLowerCase() === "address") {
    return "";
  }
  const lower = cleaned.toLowerCase();
  return lower.startsWith("http://") ||
    lower.startsWith("https://") ||
    lower.startsWith("file://") ||
    cleaned.includes(".")
    ? cleaned
    : "";
}

function normalizedUrlKey(value: string): string {
  let key = value
    .trim()
    .replace(/^"|"$/g, "")
    .replace(/\/+$/g, "")
    .toLowerCase();
  for (const prefix of ["https://", "http://", "file://"]) {
    if (key.startsWith(prefix)) {
      key = key.slice(prefix.length);
      break;
    }
  }
  if (key.startsWith("www.")) {
    key = key.slice(4);
  }
  return key.replace(/\/+$/g, "");
}

function normalizedUrlHost(value: string): string {
  return (
    normalizedUrlKey(value)
      .split(/[/?#]/, 1)[0]
      ?.replace(/^www\./, "")
      .trim() ?? ""
  );
}

function isChromeTarget(target: DevToolsTarget): boolean {
  return target.id.startsWith("chrome:");
}

function isSafariAutoTarget(target: DevToolsTarget): boolean {
  return target.safariAuto === true || target.id === SAFARI_AUTO_TARGET_ID;
}

function isConcreteSafariTarget(target: DevToolsTarget): boolean {
  return target.source === "Safari" && !isSafariAutoTarget(target);
}

function isWebKitTarget(target: DevToolsTarget): boolean {
  return (
    target.id.startsWith("webkit:") ||
    target.source === "Safari" ||
    target.source === "WebKit" ||
    target.source === "WebKit proxy"
  );
}

function isSafariBrowser(): boolean {
  if (typeof navigator === "undefined") {
    return false;
  }
  const userAgent = navigator.userAgent;
  return (
    /safari/i.test(userAgent) &&
    !/chrome|chromium|crios|fxios|edg/i.test(userAgent)
  );
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

function webKitTargetProcessIdentifier(target: WebKitTarget): number | null {
  const match = target.appId.match(/^PID:(\d+)$/);
  return match ? Number.parseInt(match[1], 10) : null;
}

function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Failed to load DevTools targets.";
}

function userFacingDevToolsMessage(message: string): string {
  const normalized = message.trim();
  if (!normalized) {
    return "";
  }

  const lower = normalized.toLowerCase();
  if (isProviderDisconnectedMessage(normalized)) {
    return NOT_CONNECTED_MESSAGE;
  }
  if (
    lower.includes("no app inspector found") ||
    lower.includes("no connected websocket inspector found") ||
    lower.includes("no published app inspector found") ||
    lower.includes("no in-app inspector found") ||
    lower.includes("first probe error:") ||
    lower.includes("timed out loading webkit targets") ||
    lower.includes("timed out loading chrome devtools targets") ||
    lower.includes("devtools target discovery returned no targets") ||
    lower.includes("webkit target discovery returned an empty listing") ||
    lower.includes("unable to read webkit packet header") ||
    lower.includes("unable to write webkit packet") ||
    lower.includes("retried webkit target discovery") ||
    lower.includes("webinspectord was settling")
  ) {
    return "";
  }

  return normalized;
}

function isProviderDisconnectedMessage(message: string): boolean {
  const lower = message.trim().toLowerCase();
  return (
    lower.includes("failed to fetch") ||
    lower.includes("load failed") ||
    lower.includes("networkerror") ||
    lower.includes("network error")
  );
}

function shouldDisplayDevToolsWarning(message: string): boolean {
  const normalized = message.trim();
  if (!normalized || normalized === NOT_CONNECTED_MESSAGE) {
    return false;
  }

  const lower = normalized.toLowerCase();
  return !(
    lower.includes("timed out loading chrome devtools targets") ||
    lower.includes("timed out loading webkit targets") ||
    lower.includes("unable to read webkit packet header") ||
    lower.includes("unable to write webkit packet") ||
    lower.includes("retried webkit target discovery") ||
    lower.includes("webinspectord was settling")
  );
}

function cleanDevToolsMessages(messages: string[]): string[] {
  return messages.flatMap((message) => {
    const nextMessage = userFacingDevToolsMessage(message);
    return nextMessage ? [nextMessage] : [];
  });
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

function settleDiscovery<T>(
  promise: Promise<T>,
): Promise<PromiseSettledResult<T>> {
  return promise.then(
    (value) => ({ status: "fulfilled", value }) as PromiseFulfilledResult<T>,
    (reason) => ({ status: "rejected", reason }) as PromiseRejectedResult,
  );
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
