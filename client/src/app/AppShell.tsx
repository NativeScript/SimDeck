import { useCallback, useEffect, useRef, useState } from "react";

import {
  bootSimulator,
  dismissKeyboard,
  launchSimulatorBundle,
  openAppSwitcher,
  openSimulatorUrl,
  pressHome,
  rotateLeft,
  rotateRight,
  sendKey,
  sendTouch,
  shutdownSimulator,
  toggleAppearance,
} from "../api/controls";
import { fetchAccessibilityTree, fetchChromeProfile } from "../api/simulators";
import type {
  AccessibilityNode,
  AccessibilitySource,
  AccessibilitySourcePreference,
  AccessibilityTreeResponse,
  ChromeProfile,
  TouchPhase,
} from "../api/types";
import { AccessibilityInspector } from "../features/accessibility/AccessibilityInspector";
import { useKeyboardInput } from "../features/input/useKeyboardInput";
import { usePointerInput } from "../features/input/usePointerInput";
import { simulatorRuntimeLabel } from "../features/simulators/simulatorDisplay";
import { useSimulatorList } from "../features/simulators/useSimulatorList";
import { useLiveStream } from "../features/stream/useLiveStream";
import { Toolbar } from "../features/toolbar/Toolbar";
import { SimulatorViewport } from "../features/viewport/SimulatorViewport";
import type { Point, ViewMode } from "../features/viewport/types";
import { useViewportLayout } from "../features/viewport/useViewportLayout";
import {
  buildShellRotationTransform,
  clampPan,
  clampZoom,
  computeChromeScreenRect,
  screenAspectRatio,
  shellSize,
} from "../features/viewport/viewportMath";
import {
  STREAM_ORIGIN,
  ZOOM_ANIMATION_MS,
  ZOOM_STEP,
} from "../shared/constants";
import { useElementSize } from "../shared/hooks/useElementSize";
import {
  ACCESSIBILITY_SOURCE_STORAGE_KEY,
  DEBUG_VISIBLE_STORAGE_KEY,
  DEFAULT_VIEWPORT_STATE,
  HIERARCHY_VISIBLE_STORAGE_KEY,
  readPersistedUiState,
  readStoredAccessibilitySource,
  readStoredFlag,
  sanitizeAccessibilitySources,
  viewportStateForUDID,
  writePersistedUiState,
  writeStoredFlag,
} from "./uiState";

const ACCESSIBILITY_REFRESH_MS = 1500;

function buildChromeUrl(udid: string, stamp: number): string {
  return `${STREAM_ORIGIN}/api/simulators/${udid}/chrome.png?stamp=${stamp}`;
}

type SimulatorTransition = {
  kind: "boot" | "shutdown";
  udid: string;
};

export function AppShell() {
  const [initialUiState] = useState(readPersistedUiState);
  const [initialSelectedUDID] = useState(
    () => readDeviceQueryParam() ?? initialUiState.selectedUDID,
  );
  const initialViewportState = initialSelectedUDID
    ? viewportStateForUDID(initialUiState, initialSelectedUDID)
    : DEFAULT_VIEWPORT_STATE;
  const {
    error: listError,
    isLoading,
    refresh,
    simulators,
  } = useSimulatorList();
  const [debugVisible, setDebugVisible] = useState(() =>
    readStoredFlag(DEBUG_VISIBLE_STORAGE_KEY),
  );
  const [hierarchyVisible, setHierarchyVisible] = useState(() =>
    readStoredFlag(HIERARCHY_VISIBLE_STORAGE_KEY),
  );
  const [selectedUDID, setSelectedUDID] = useState(initialSelectedUDID ?? "");
  const [search, setSearch] = useState(initialUiState.search ?? "");
  const [openURLValue, setOpenURLValue] = useState(
    initialUiState.openURLValue ?? "https://example.com",
  );
  const [bundleIDValue, setBundleIDValue] = useState(
    initialUiState.bundleIDValue ?? "com.apple.Preferences",
  );
  const [menuOpen, setMenuOpen] = useState(false);
  const [localError, setLocalError] = useState("");
  const [simulatorTransition, setSimulatorTransition] =
    useState<SimulatorTransition | null>(null);
  const [rotationQuarterTurns, setRotationQuarterTurns] = useState(
    initialViewportState.rotationQuarterTurns,
  );
  const [streamStamp, setStreamStamp] = useState(Date.now());
  const [viewMode, setViewMode] = useState<ViewMode>(
    initialViewportState.viewMode,
  );
  const [zoom, setZoom] = useState<number | null>(initialViewportState.zoom);
  const [pan, setPan] = useState<Point>(initialViewportState.pan);
  const [chromeProfile, setChromeProfile] = useState<ChromeProfile | null>(
    null,
  );
  const [accessibilityRoots, setAccessibilityRoots] = useState<
    AccessibilityNode[]
  >([]);
  const [accessibilitySelectedId, setAccessibilitySelectedId] = useState(
    initialSelectedUDID
      ? (initialUiState.accessibilitySelectedByUDID?.[initialSelectedUDID] ??
          "")
      : "",
  );
  const [accessibilityHoveredId, setAccessibilityHoveredId] = useState<
    string | null
  >(null);
  const [accessibilityPickerActive, setAccessibilityPickerActive] =
    useState(false);
  const [accessibilityError, setAccessibilityError] = useState("");
  const [accessibilityLoading, setAccessibilityLoading] = useState(false);
  const [accessibilitySource, setAccessibilitySource] = useState<
    AccessibilityTreeResponse["source"] | ""
  >("");
  const [accessibilityAvailableSources, setAccessibilityAvailableSources] =
    useState<AccessibilitySource[]>([]);
  const [accessibilityPreferredSource, setAccessibilityPreferredSource] =
    useState<AccessibilitySourcePreference>(readStoredAccessibilitySource);
  const [zoomAnimating, setZoomAnimating] = useState(false);

  const menuRef = useRef<HTMLDivElement | null>(null);
  const outerCanvasRef = useRef<HTMLDivElement | null>(null);
  const streamCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const [outerCanvasElement, setOuterCanvasElement] =
    useState<HTMLDivElement | null>(null);
  const [streamCanvasElement, setStreamCanvasElement] =
    useState<HTMLCanvasElement | null>(null);
  const [zoomDockElement, setZoomDockElement] = useState<HTMLDivElement | null>(
    null,
  );
  const zoomAnimationTimeoutRef = useRef<number>(0);
  const accessibilityRequestIdRef = useRef(0);
  const accessibilityLoadingRef = useRef(false);
  const canvasSize = useElementSize(outerCanvasElement);
  const zoomDockSize = useElementSize(zoomDockElement);

  const handleOuterCanvasRef = useCallback((node: HTMLDivElement | null) => {
    outerCanvasRef.current = node;
    setOuterCanvasElement(node);
  }, []);

  const handleZoomDockRef = useCallback((node: HTMLDivElement | null) => {
    setZoomDockElement(node);
  }, []);

  const searchNeedle = search.trim().toLowerCase();
  const filteredSimulators = simulators.filter((simulator) => {
    if (!searchNeedle) {
      return true;
    }
    return [
      simulator.name,
      simulatorRuntimeLabel(simulator),
      simulator.runtimeName,
      simulator.runtimeIdentifier,
      simulator.deviceTypeIdentifier,
      simulator.udid,
    ]
      .filter(Boolean)
      .join(" ")
      .toLowerCase()
      .includes(searchNeedle);
  });

  const selectedSimulator =
    simulators.find((simulator) => simulator.udid === selectedUDID) ??
    filteredSimulators[0] ??
    null;
  const selectedSimulatorDetail =
    selectedSimulator != null &&
    simulatorTransition?.udid === selectedSimulator.udid
      ? simulatorTransition.kind === "boot"
        ? "Starting..."
        : "Stopping..."
      : selectedSimulator != null
        ? simulatorRuntimeLabel(selectedSimulator)
        : "";

  const handleStreamCanvasRef = useCallback(
    (node: HTMLCanvasElement | null) => {
      streamCanvasRef.current = node;
      setStreamCanvasElement(node);
    },
    [],
  );

  const {
    deviceNaturalSize,
    error: streamError,
    fps,
    hasFrame,
    runtimeInfo,
    stats,
    status: streamStatus,
  } = useLiveStream({
    canvasElement: streamCanvasElement,
    simulator: selectedSimulator,
  });

  const zoomDockReservedHeight =
    zoomDockElement && typeof window !== "undefined"
      ? (zoomDockSize?.height ?? 0) +
        Number.parseFloat(
          window.getComputedStyle(zoomDockElement).bottom || "0",
        )
      : 0;

  const { fitScale, effectiveZoom } = useViewportLayout({
    canvasSize,
    chromeProfile,
    deviceNaturalSize,
    pan,
    rotationQuarterTurns,
    reservedBottomInset: zoomDockReservedHeight,
    viewMode,
    zoom,
  });

  const isBooted = Boolean(selectedSimulator?.isBooted);
  const autoViewportOffsetY =
    viewMode === "manual" ? 0 : -zoomDockReservedHeight / 2;
  const screenAspect = screenAspectRatio(deviceNaturalSize);
  const chromeUrl = selectedSimulator
    ? buildChromeUrl(selectedSimulator.udid, streamStamp)
    : "";

  useEffect(() => {
    writeStoredFlag(DEBUG_VISIBLE_STORAGE_KEY, debugVisible);
  }, [debugVisible]);

  useEffect(() => {
    writeStoredFlag(HIERARCHY_VISIBLE_STORAGE_KEY, hierarchyVisible);
  }, [hierarchyVisible]);

  useEffect(() => {
    window.localStorage.setItem(
      ACCESSIBILITY_SOURCE_STORAGE_KEY,
      accessibilityPreferredSource,
    );
  }, [accessibilityPreferredSource]);

  useEffect(() => {
    if (simulatorTransition == null) {
      return;
    }
    const simulator = simulators.find(
      (candidate) => candidate.udid === simulatorTransition.udid,
    );
    if (
      (simulatorTransition.kind === "boot" && simulator?.isBooted) ||
      (simulatorTransition.kind === "shutdown" && simulator?.isBooted === false)
    ) {
      setSimulatorTransition(null);
    }
  }, [simulatorTransition, simulators]);

  useEffect(() => {
    writePersistedUiState((current) => ({
      ...current,
      bundleIDValue,
      openURLValue,
      search,
      selectedUDID,
    }));
  }, [bundleIDValue, openURLValue, search, selectedUDID]);

  useEffect(() => {
    if (!selectedSimulator) {
      return;
    }

    writePersistedUiState((current) => ({
      ...current,
      viewportByUDID: {
        ...(current.viewportByUDID ?? {}),
        [selectedSimulator.udid]: {
          pan,
          rotationQuarterTurns,
          viewMode,
          zoom,
        },
      },
    }));
  }, [pan, rotationQuarterTurns, selectedSimulator?.udid, viewMode, zoom]);

  useEffect(() => {
    if (!selectedSimulator) {
      return;
    }

    writePersistedUiState((current) => ({
      ...current,
      accessibilitySelectedByUDID: {
        ...(current.accessibilitySelectedByUDID ?? {}),
        [selectedSimulator.udid]: accessibilitySelectedId,
      },
    }));
  }, [accessibilitySelectedId, selectedSimulator?.udid]);

  useEffect(() => {
    if (selectedSimulator && selectedSimulator.udid !== selectedUDID) {
      setSelectedUDID(selectedSimulator.udid);
    }
  }, [selectedSimulator, selectedUDID]);

  useEffect(() => {
    const nextViewportState = selectedSimulator
      ? viewportStateForUDID(readPersistedUiState(), selectedSimulator.udid)
      : DEFAULT_VIEWPORT_STATE;
    setStreamStamp(Date.now());
    setChromeProfile(null);
    setViewMode(nextViewportState.viewMode);
    setZoom(nextViewportState.zoom);
    setPan(nextViewportState.pan);
    setRotationQuarterTurns(nextViewportState.rotationQuarterTurns);
    setLocalError("");
    setAccessibilityRoots([]);
    setAccessibilitySelectedId(
      selectedSimulator
        ? (readPersistedUiState().accessibilitySelectedByUDID?.[
            selectedSimulator.udid
          ] ?? "")
        : "",
    );
    setAccessibilityHoveredId(null);
    setAccessibilityPickerActive(false);
    setAccessibilityError("");
    setAccessibilitySource("");
    setAccessibilityAvailableSources([]);
    accessibilityRequestIdRef.current += 1;
    accessibilityLoadingRef.current = false;
    setAccessibilityLoading(false);
  }, [selectedSimulator?.udid]);

  const loadAccessibilityTree = useCallback(async () => {
    if (accessibilityLoadingRef.current) {
      return;
    }

    if (!selectedSimulator?.isBooted) {
      setAccessibilityRoots([]);
      setAccessibilitySelectedId("");
      setAccessibilityAvailableSources([]);
      setAccessibilitySource("");
      setAccessibilityError(
        selectedSimulator ? "Boot the simulator to inspect UI." : "",
      );
      return;
    }

    const requestId = accessibilityRequestIdRef.current + 1;
    accessibilityRequestIdRef.current = requestId;
    accessibilityLoadingRef.current = true;
    setAccessibilityLoading(true);
    setAccessibilityError("");

    try {
      const snapshot = await fetchAccessibilityTree(
        selectedSimulator.udid,
        accessibilityPreferredSource,
      );
      if (accessibilityRequestIdRef.current !== requestId) {
        return;
      }
      const roots = snapshot.roots ?? [];
      const availableSources = sanitizeAccessibilitySources(
        snapshot.availableSources,
      );
      setAccessibilityRoots(roots);
      setAccessibilityAvailableSources(availableSources);
      setAccessibilitySource(snapshot.source);
      setAccessibilityError(
        roots.length === 0 ? (snapshot.fallbackReason ?? "") : "",
      );
      if (
        snapshot.source === "native-ax" &&
        availableSources.includes("nativescript") &&
        accessibilityPreferredSource !== "nativescript"
      ) {
        setAccessibilityPreferredSource("nativescript");
      }
      if (
        accessibilityPreferredSource !== "auto" &&
        !availableSources.includes(accessibilityPreferredSource)
      ) {
        setAccessibilityPreferredSource(snapshot.source);
      }
      if (roots.length === 0) {
        setAccessibilitySelectedId("");
      }
    } catch (snapshotError) {
      if (accessibilityRequestIdRef.current !== requestId) {
        return;
      }
      setAccessibilityError(
        snapshotError instanceof Error
          ? snapshotError.message
          : "Failed to load accessibility hierarchy.",
      );
      setAccessibilityRoots([]);
      setAccessibilitySelectedId("");
      setAccessibilityHoveredId(null);
      setAccessibilitySource("");
      setAccessibilityAvailableSources([]);
      if (accessibilityPreferredSource !== "auto") {
        setAccessibilityPreferredSource("auto");
      }
    } finally {
      if (accessibilityRequestIdRef.current === requestId) {
        accessibilityLoadingRef.current = false;
        setAccessibilityLoading(false);
      }
    }
  }, [accessibilityPreferredSource, selectedSimulator]);

  useEffect(() => {
    if (!hierarchyVisible) {
      return;
    }

    void loadAccessibilityTree();
    const interval = window.setInterval(() => {
      void loadAccessibilityTree();
    }, ACCESSIBILITY_REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [hierarchyVisible, loadAccessibilityTree]);

  useEffect(() => {
    if (!isBooted) {
      setAccessibilityPickerActive(false);
    }
  }, [isBooted]);

  useEffect(() => {
    let cancelled = false;

    async function loadChromeProfile() {
      if (!selectedSimulator) {
        setChromeProfile(null);
        return;
      }

      try {
        const profile = await fetchChromeProfile(selectedSimulator.udid);
        if (!cancelled) {
          setChromeProfile(profile);
        }
      } catch {
        if (!cancelled) {
          setChromeProfile(null);
        }
      }
    }

    void loadChromeProfile();
    return () => {
      cancelled = true;
    };
  }, [selectedSimulator?.udid]);

  useEffect(() => {
    if (!menuOpen) {
      return;
    }

    function handleDocumentPointerDown(event: PointerEvent) {
      if (menuRef.current?.contains(event.target as Node)) {
        return;
      }
      setMenuOpen(false);
    }

    function handleWindowKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setMenuOpen(false);
      }
    }

    document.addEventListener("pointerdown", handleDocumentPointerDown);
    window.addEventListener("keydown", handleWindowKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handleDocumentPointerDown);
      window.removeEventListener("keydown", handleWindowKeyDown);
    };
  }, [menuOpen]);

  useEffect(() => {
    setPan((currentPan) => {
      const nextPan = clampPan(
        currentPan,
        effectiveZoom,
        canvasSize,
        deviceNaturalSize,
        chromeProfile,
        rotationQuarterTurns,
      );
      return nextPan.x === currentPan.x && nextPan.y === currentPan.y
        ? currentPan
        : nextPan;
    });
  }, [
    canvasSize,
    chromeProfile,
    deviceNaturalSize,
    effectiveZoom,
    rotationQuarterTurns,
  ]);

  useEffect(() => {
    return () => {
      if (zoomAnimationTimeoutRef.current) {
        clearTimeout(zoomAnimationTimeoutRef.current);
      }
    };
  }, []);

  useKeyboardInput({
    enabled: Boolean(selectedSimulator?.isBooted && selectedSimulator.udid),
    onKey: ({ keyCode, modifiers }) => {
      if (!selectedSimulator) {
        return;
      }
      void runAction(
        () => sendKey(selectedSimulator.udid, { keyCode, modifiers }),
        false,
      );
    },
  });

  const pointerInput = usePointerInput({
    canvasSize,
    chromeProfile,
    deviceNaturalSize,
    effectiveZoom,
    fitScale,
    isBooted,
    onTouch: (phase: TouchPhase, coords: Point) => {
      if (!selectedSimulator) {
        return;
      }
      if (phase === "began" && accessibilitySelectedId) {
        setAccessibilitySelectedId("");
        setAccessibilityHoveredId(null);
      }
      void runAction(
        () => sendTouch(selectedSimulator.udid, { ...coords, phase }),
        false,
      );
    },
    pan,
    rotationQuarterTurns,
    setPan,
  });

  const error = localError || streamError || listError;
  const deviceTransform = `translate(${pan.x}px, ${pan.y + autoViewportOffsetY}px) scale(${effectiveZoom})`;
  const chromeScreenRect = computeChromeScreenRect(
    chromeProfile,
    deviceNaturalSize,
  );
  const chromeScreenStyle =
    chromeProfile && chromeScreenRect
      ? {
          left: `${(chromeScreenRect.x / chromeProfile.totalWidth) * 100}%`,
          top: `${(chromeScreenRect.y / chromeProfile.totalHeight) * 100}%`,
          width: `${(chromeScreenRect.width / chromeProfile.totalWidth) * 100}%`,
          height: `${(chromeScreenRect.height / chromeProfile.totalHeight) * 100}%`,
          borderRadius: `${chromeProfile.cornerRadius}px`,
        }
      : null;
  const shellStyle = chromeProfile
    ? {
        width: `${chromeProfile.totalWidth}px`,
        height: `${chromeProfile.totalHeight}px`,
      }
    : null;
  const deviceFrameSize = shellSize(
    deviceNaturalSize,
    chromeProfile,
    rotationQuarterTurns,
  );
  const naturalShellSize = shellSize(deviceNaturalSize, chromeProfile);
  const deviceFrameStyle = {
    width: `${deviceFrameSize.width}px`,
    height: `${deviceFrameSize.height}px`,
  };
  const devicePresentationStyle = {
    width: `${naturalShellSize.width}px`,
    height: `${naturalShellSize.height}px`,
    transform: buildShellRotationTransform(
      deviceNaturalSize,
      chromeProfile,
      rotationQuarterTurns,
    ),
  };

  async function runAction(
    action: () => Promise<unknown>,
    refreshAfter = true,
  ): Promise<boolean> {
    setLocalError("");
    try {
      await action();
      if (refreshAfter) {
        await refresh();
      }
      return true;
    } catch (actionError) {
      setLocalError(
        actionError instanceof Error ? actionError.message : "Request failed.",
      );
      return false;
    }
  }

  function beginZoomAnimation() {
    setZoomAnimating(true);
    if (zoomAnimationTimeoutRef.current) {
      clearTimeout(zoomAnimationTimeoutRef.current);
    }
    zoomAnimationTimeoutRef.current = window.setTimeout(() => {
      setZoomAnimating(false);
      zoomAnimationTimeoutRef.current = 0;
    }, ZOOM_ANIMATION_MS);
  }

  function applyZoom(
    nextScale: number,
    nextPan = { x: pan.x, y: pan.y + autoViewportOffsetY },
  ) {
    const clampedScale = clampZoom(nextScale, fitScale);
    beginZoomAnimation();
    setViewMode("manual");
    setZoom(clampedScale);
    setPan(
      clampPan(
        nextPan,
        clampedScale,
        canvasSize,
        deviceNaturalSize,
        chromeProfile,
        rotationQuarterTurns,
      ),
    );
  }

  function promptForURL() {
    if (!selectedSimulator) {
      return;
    }
    const nextValue = window.prompt(
      `Open URL on ${selectedSimulator.name}`,
      openURLValue,
    );
    if (nextValue == null) {
      return;
    }
    const trimmed = nextValue.trim();
    if (!trimmed) {
      return;
    }
    setOpenURLValue(trimmed);
    setMenuOpen(false);
    void runAction(() =>
      openSimulatorUrl(selectedSimulator.udid, { url: trimmed }),
    );
  }

  function promptForBundleID() {
    if (!selectedSimulator) {
      return;
    }
    const nextValue = window.prompt(
      `Launch bundle on ${selectedSimulator.name}`,
      bundleIDValue,
    );
    if (nextValue == null) {
      return;
    }
    const trimmed = nextValue.trim();
    if (!trimmed) {
      return;
    }
    setBundleIDValue(trimmed);
    setMenuOpen(false);
    void runAction(() =>
      launchSimulatorBundle(selectedSimulator.udid, { bundleId: trimmed }),
    );
  }

  return (
    <div className="app">
      <Toolbar
        closeMenu={() => setMenuOpen(false)}
        debugVisible={debugVisible}
        error={error}
        filteredSimulators={filteredSimulators}
        fps={fps}
        hierarchyVisible={hierarchyVisible}
        isLoading={isLoading}
        menuOpen={menuOpen}
        menuRef={menuRef}
        onBoot={() => {
          if (!selectedSimulator) {
            return;
          }
          const udid = selectedSimulator.udid;
          setSimulatorTransition({ kind: "boot", udid });
          void runAction(() => bootSimulator(udid)).then((ok) => {
            if (!ok) {
              setSimulatorTransition((current) =>
                current?.udid === udid ? null : current,
              );
            }
          });
        }}
        onChangeSearch={setSearch}
        onDismissKeyboard={() => {
          if (!selectedSimulator) {
            return;
          }
          void runAction(() => dismissKeyboard(selectedSimulator.udid), false);
        }}
        onHome={() => {
          if (!selectedSimulator) {
            return;
          }
          setAccessibilitySelectedId("");
          setAccessibilityHoveredId(null);
          void runAction(() => pressHome(selectedSimulator.udid), false);
        }}
        onOpenAppSwitcher={() => {
          if (!selectedSimulator) {
            return;
          }
          setAccessibilitySelectedId("");
          setAccessibilityHoveredId(null);
          void runAction(() => openAppSwitcher(selectedSimulator.udid), false);
        }}
        onRotateLeft={() => {
          if (!selectedSimulator) {
            return;
          }
          void runAction(async () => {
            await rotateLeft(selectedSimulator.udid);
            setRotationQuarterTurns((current) => (current + 3) % 4);
            setStreamStamp(Date.now());
          }, false);
        }}
        onOpenBundlePrompt={promptForBundleID}
        onOpenUrlPrompt={promptForURL}
        onRotateRight={() => {
          if (!selectedSimulator) {
            return;
          }
          void runAction(async () => {
            await rotateRight(selectedSimulator.udid);
            setRotationQuarterTurns((current) => (current + 1) % 4);
            setStreamStamp(Date.now());
          }, false);
        }}
        onShutdown={() => {
          if (!selectedSimulator) {
            return;
          }
          const udid = selectedSimulator.udid;
          setSimulatorTransition({ kind: "shutdown", udid });
          void runAction(() => shutdownSimulator(udid)).then((ok) => {
            if (!ok) {
              setSimulatorTransition((current) =>
                current?.udid === udid ? null : current,
              );
            }
          });
        }}
        onToggleAppearance={() => {
          if (!selectedSimulator) {
            return;
          }
          void runAction(() => toggleAppearance(selectedSimulator.udid));
        }}
        onToggleDebug={() => setDebugVisible((current) => !current)}
        onToggleHierarchy={() => {
          setHierarchyVisible((current) => !current);
          if (hierarchyVisible) {
            setAccessibilityPickerActive(false);
          }
          if (!hierarchyVisible) {
            void loadAccessibilityTree();
          }
        }}
        onToggleMenu={() => setMenuOpen((current) => !current)}
        runtimeInfo={runtimeInfo}
        search={search}
        selectedSimulator={selectedSimulator}
        selectedSimulatorIdentifier={selectedSimulatorDetail}
        setSelectedUDID={setSelectedUDID}
        stats={stats}
        status={streamStatus}
      />
      <SimulatorViewport
        accessibilityHoveredId={accessibilityHoveredId}
        accessibilityPanel={
          <AccessibilityInspector
            availableSources={accessibilityAvailableSources}
            error={accessibilityError}
            isLoading={accessibilityLoading}
            onHover={setAccessibilityHoveredId}
            onPickerToggle={() =>
              setAccessibilityPickerActive((current) => !current)
            }
            onSelect={(id) =>
              setAccessibilitySelectedId((current) =>
                current === id ? "" : id,
              )
            }
            onSourceChange={setAccessibilityPreferredSource}
            pickerActive={accessibilityPickerActive}
            roots={accessibilityRoots}
            selectedId={accessibilitySelectedId}
            selectedSimulator={selectedSimulator}
            source={accessibilitySource}
            visible={hierarchyVisible}
          />
        }
        accessibilityPickerActive={accessibilityPickerActive}
        accessibilityRoots={accessibilityRoots}
        accessibilitySelectedId={accessibilitySelectedId}
        chromeProfile={chromeProfile}
        chromeScreenStyle={chromeScreenStyle}
        chromeUrl={chromeUrl}
        deviceFrameStyle={deviceFrameStyle}
        devicePresentationStyle={devicePresentationStyle}
        deviceTransform={deviceTransform}
        effectiveZoom={effectiveZoom}
        fitScale={fitScale}
        hasFrame={hasFrame}
        isLoading={isLoading}
        isStreamError={streamStatus.state === "error"}
        isPanning={pointerInput.isPanning}
        onPanPointerMove={pointerInput.handlePanPointerMove}
        onPanPointerUp={pointerInput.handlePanPointerUp}
        onPickerHover={setAccessibilityHoveredId}
        onPickerSelect={(id) => {
          setAccessibilitySelectedId(id);
          setAccessibilityHoveredId(null);
          setAccessibilityPickerActive(false);
        }}
        onScreenPointerCancel={pointerInput.handleScreenPointerCancel}
        onScreenPointerDown={pointerInput.handleScreenPointerDown}
        onScreenPointerMove={pointerInput.handleScreenPointerMove}
        onScreenPointerUp={pointerInput.handleScreenPointerUp}
        onStartPanning={pointerInput.startPanning}
        onZoomActual={() => applyZoom(1)}
        onZoomCenter={() => {
          beginZoomAnimation();
          setViewMode("center");
          setZoom(null);
          setPan({ x: 0, y: 0 });
        }}
        onZoomFit={() => {
          beginZoomAnimation();
          setViewMode("fit");
          setZoom(null);
          setPan({ x: 0, y: 0 });
        }}
        onZoomIn={() => applyZoom(effectiveZoom * ZOOM_STEP)}
        onZoomOut={() => applyZoom(effectiveZoom / ZOOM_STEP)}
        outerCanvasRef={handleOuterCanvasRef}
        rotationQuarterTurns={rotationQuarterTurns}
        screenAspect={screenAspect}
        selectedSimulator={selectedSimulator}
        shellStyle={shellStyle}
        streamCanvasRef={handleStreamCanvasRef}
        viewMode={viewMode}
        zoomDockRef={handleZoomDockRef}
        zoomAnimating={zoomAnimating}
      />
    </div>
  );
}

function readDeviceQueryParam(): string | undefined {
  if (typeof window === "undefined") {
    return undefined;
  }

  const value = new URLSearchParams(window.location.search).get("device");
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}
