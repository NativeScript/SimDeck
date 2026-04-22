import { useCallback, useEffect, useRef, useState } from "react";

import {
  bootSimulator,
  launchSimulatorBundle,
  openSimulatorUrl,
  pressHome,
  rotateRight,
  sendKey,
  sendTouch,
  shutdownSimulator
} from "../api/controls";
import { fetchChromeProfile } from "../api/simulators";
import type { ChromeProfile, SimulatorMetadata, TouchPhase } from "../api/types";
import { useKeyboardInput } from "../features/input/useKeyboardInput";
import { usePointerInput } from "../features/input/usePointerInput";
import { useSimulatorList } from "../features/simulators/useSimulatorList";
import { useLiveStream } from "../features/stream/useLiveStream";
import { Toolbar } from "../features/toolbar/Toolbar";
import { SimulatorViewport } from "../features/viewport/SimulatorViewport";
import type { Point, ViewMode } from "../features/viewport/types";
import { useViewportLayout } from "../features/viewport/useViewportLayout";
import { clampPan, clampZoom, computeChromeScreenRect } from "../features/viewport/viewportMath";
import { STREAM_ORIGIN, ZOOM_ANIMATION_MS, ZOOM_STEP } from "../shared/constants";
import { useElementSize } from "../shared/hooks/useElementSize";

function buildChromeUrl(udid: string, stamp: number): string {
  return `${STREAM_ORIGIN}/api/simulators/${udid}/chrome.png?stamp=${stamp}`;
}

export function AppShell() {
  const { error: listError, isLoading, refresh, simulators } = useSimulatorList();
  const [debugVisible, setDebugVisible] = useState(() => {
    if (typeof window === "undefined") {
      return false;
    }
    return window.localStorage.getItem("xcw-debug-visible") === "1";
  });
  const [selectedUDID, setSelectedUDID] = useState("");
  const [search, setSearch] = useState("");
  const [openURLValue, setOpenURLValue] = useState("https://example.com");
  const [bundleIDValue, setBundleIDValue] = useState("com.apple.Preferences");
  const [menuOpen, setMenuOpen] = useState(false);
  const [localError, setLocalError] = useState("");
  const [streamStamp, setStreamStamp] = useState(Date.now());
  const [viewMode, setViewMode] = useState<ViewMode>("center");
  const [zoom, setZoom] = useState<number | null>(null);
  const [pan, setPan] = useState<Point>({ x: 0, y: 0 });
  const [chromeProfile, setChromeProfile] = useState<ChromeProfile | null>(null);
  const [zoomAnimating, setZoomAnimating] = useState(false);

  const menuRef = useRef<HTMLDivElement | null>(null);
  const outerCanvasRef = useRef<HTMLDivElement | null>(null);
  const streamCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const [outerCanvasElement, setOuterCanvasElement] = useState<HTMLDivElement | null>(null);
  const [streamCanvasElement, setStreamCanvasElement] = useState<HTMLCanvasElement | null>(null);
  const [zoomDockElement, setZoomDockElement] = useState<HTMLDivElement | null>(null);
  const zoomAnimationTimeoutRef = useRef<number>(0);
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
      simulator.runtimeName,
      simulator.runtimeIdentifier,
      simulator.deviceTypeIdentifier,
      simulator.udid
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

  const handleStreamCanvasRef = useCallback((node: HTMLCanvasElement | null) => {
    streamCanvasRef.current = node;
    setStreamCanvasElement(node);
  }, []);

  const {
    deviceNaturalSize,
    error: streamError,
    fps,
    hasFrame,
    runtimeInfo,
    stats,
    status: streamStatus
  } = useLiveStream({
    canvasElement: streamCanvasElement,
    simulator: selectedSimulator,
    stamp: streamStamp
  });

  const zoomDockReservedHeight =
    zoomDockElement && typeof window !== "undefined"
      ? (zoomDockSize?.height ?? 0) +
        Number.parseFloat(window.getComputedStyle(zoomDockElement).bottom || "0")
      : 0;

  const { fitScale, effectiveZoom } = useViewportLayout({
    canvasSize,
    chromeProfile,
    deviceNaturalSize,
    pan,
    reservedBottomInset: zoomDockReservedHeight,
    viewMode,
    zoom
  });

  const isBooted = Boolean(selectedSimulator?.isBooted);
  const autoViewportOffsetY = viewMode === "manual" ? 0 : -zoomDockReservedHeight / 2;
  const screenAspect = deviceNaturalSize
    ? `${deviceNaturalSize.width} / ${deviceNaturalSize.height}`
    : "9 / 19.5";
  const selectedSimulatorIdentifier =
    selectedSimulator?.deviceTypeIdentifier ??
    selectedSimulator?.runtimeIdentifier ??
    selectedSimulator?.udid ??
    "";
  const chromeUrl = selectedSimulator ? buildChromeUrl(selectedSimulator.udid, streamStamp) : "";

  useEffect(() => {
    window.localStorage.setItem("xcw-debug-visible", debugVisible ? "1" : "0");
  }, [debugVisible]);

  useEffect(() => {
    if (selectedSimulator && selectedSimulator.udid !== selectedUDID) {
      setSelectedUDID(selectedSimulator.udid);
    }
  }, [selectedSimulator, selectedUDID]);

  useEffect(() => {
    setStreamStamp(Date.now());
    setChromeProfile(null);
    setViewMode("center");
    setZoom(null);
    setPan({ x: 0, y: 0 });
    setLocalError("");
  }, [selectedSimulator?.udid]);

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
        chromeProfile
      );
      return nextPan.x === currentPan.x && nextPan.y === currentPan.y ? currentPan : nextPan;
    });
  }, [canvasSize, chromeProfile, deviceNaturalSize, effectiveZoom]);

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
      void runAction(() => sendKey(selectedSimulator.udid, { keyCode, modifiers }), false);
    }
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
      void runAction(() => sendTouch(selectedSimulator.udid, { ...coords, phase }), false);
    },
    pan,
    setPan
  });

  const error = localError || streamError || listError;
  const deviceTransform = `translate(${pan.x}px, ${pan.y + autoViewportOffsetY}px) scale(${effectiveZoom})`;
  const chromeScreenRect = computeChromeScreenRect(chromeProfile, deviceNaturalSize);
  const chromeScreenStyle =
    chromeProfile && chromeScreenRect
      ? {
          left: `${(chromeScreenRect.x / chromeProfile.totalWidth) * 100}%`,
          top: `${(chromeScreenRect.y / chromeProfile.totalHeight) * 100}%`,
          width: `${(chromeScreenRect.width / chromeProfile.totalWidth) * 100}%`,
          height: `${(chromeScreenRect.height / chromeProfile.totalHeight) * 100}%`,
          borderRadius: `${chromeProfile.cornerRadius}px`
        }
      : null;
  const shellStyle = chromeProfile
    ? {
        width: `${chromeProfile.totalWidth}px`,
        height: `${chromeProfile.totalHeight}px`
      }
    : null;

  async function runAction(action: () => Promise<unknown>, refreshAfter = true) {
    setLocalError("");
    try {
      await action();
      if (refreshAfter) {
        await refresh();
      }
    } catch (actionError) {
      setLocalError(actionError instanceof Error ? actionError.message : "Request failed.");
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

  function applyZoom(nextScale: number, nextPan = { x: pan.x, y: pan.y + autoViewportOffsetY }) {
    const clampedScale = clampZoom(nextScale, fitScale);
    beginZoomAnimation();
    setViewMode("manual");
    setZoom(clampedScale);
    setPan(clampPan(nextPan, clampedScale, canvasSize, deviceNaturalSize, chromeProfile));
  }

  function promptForURL() {
    if (!selectedSimulator) {
      return;
    }
    const nextValue = window.prompt(`Open URL on ${selectedSimulator.name}`, openURLValue);
    if (nextValue == null) {
      return;
    }
    const trimmed = nextValue.trim();
    if (!trimmed) {
      return;
    }
    setOpenURLValue(trimmed);
    setMenuOpen(false);
    void runAction(() => openSimulatorUrl(selectedSimulator.udid, { url: trimmed }));
  }

  function promptForBundleID() {
    if (!selectedSimulator) {
      return;
    }
    const nextValue = window.prompt(
      `Launch bundle on ${selectedSimulator.name}`,
      bundleIDValue
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
    void runAction(() => launchSimulatorBundle(selectedSimulator.udid, { bundleId: trimmed }));
  }

  return (
    <div className="app">
      <Toolbar
        closeMenu={() => setMenuOpen(false)}
        debugVisible={debugVisible}
        error={error}
        filteredSimulators={filteredSimulators}
        fps={fps}
        isLoading={isLoading}
        menuOpen={menuOpen}
        menuRef={menuRef}
        onBoot={() => {
          if (!selectedSimulator) {
            return;
          }
          void runAction(() => bootSimulator(selectedSimulator.udid));
        }}
        onChangeSearch={setSearch}
        onHome={() => {
          if (!selectedSimulator) {
            return;
          }
          void runAction(() => pressHome(selectedSimulator.udid), false);
        }}
        onOpenBundlePrompt={promptForBundleID}
        onOpenUrlPrompt={promptForURL}
        onRotateRight={() => {
          if (!selectedSimulator) {
            return;
          }
          void runAction(() => rotateRight(selectedSimulator.udid), false);
          setStreamStamp(Date.now());
        }}
        onShutdown={() => {
          if (!selectedSimulator) {
            return;
          }
          void runAction(() => shutdownSimulator(selectedSimulator.udid));
        }}
        onToggleDebug={() => setDebugVisible((current) => !current)}
        onToggleMenu={() => setMenuOpen((current) => !current)}
        runtimeInfo={runtimeInfo}
        search={search}
        selectedSimulator={selectedSimulator}
        selectedSimulatorIdentifier={selectedSimulatorIdentifier}
        setSelectedUDID={setSelectedUDID}
        stats={stats}
        status={streamStatus}
      />
      <SimulatorViewport
        chromeProfile={chromeProfile}
        chromeScreenStyle={chromeScreenStyle}
        chromeUrl={chromeUrl}
        deviceTransform={deviceTransform}
        effectiveZoom={effectiveZoom}
        fitScale={fitScale}
        hasFrame={hasFrame}
        isLoading={isLoading}
        isStreamError={streamStatus.state === "error"}
        isPanning={pointerInput.isPanning}
        onPanPointerMove={pointerInput.handlePanPointerMove}
        onPanPointerUp={pointerInput.handlePanPointerUp}
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
