import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
} from "react";

import {
  ApiError,
  accessTokenFromLocation,
  apiRequest,
  pairBrowser,
} from "../api/client";
import { apiUrl, configureSimDeckClient } from "../api/config";
import {
  bootSimulator,
  captureSimulatorScreenshot,
  dismissKeyboard,
  launchSimulatorBundle,
  openAppSwitcher,
  openSimulatorUrl,
  pressHome,
  pressSimulatorButton,
  recordSimulatorScreen,
  rotateDigitalCrown,
  rotateRight,
  simulatorControlSocketUrl,
  shutdownSimulator,
  toggleAppearance,
  uploadSimulatorApp,
  type ControlMessage,
} from "../api/controls";
import {
  fetchAccessibilityTree,
  fetchChromeProfile,
  fetchSimulatorState,
} from "../api/simulators";
import type {
  AccessibilityNode,
  AccessibilitySource,
  AccessibilitySourcePreference,
  AccessibilityTreeResponse,
  ChromeProfile,
  SimulatorMetadata,
  SimulatorStateResponse,
  TouchPhase,
} from "../api/types";
import { AccessibilityInspector } from "../features/accessibility/AccessibilityInspector";
import { DevToolsPanel } from "../features/devtools/DevToolsPanel";
import { isEditableTarget } from "../features/input/keycodes";
import { useKeyboardInput } from "../features/input/useKeyboardInput";
import { usePointerInput } from "../features/input/usePointerInput";
import {
  shouldRenderNativeChrome,
  simulatorHasFixedOrientation,
  simulatorRuntimeLabel,
} from "../features/simulators/simulatorDisplay";
import { useSimulatorList } from "../features/simulators/useSimulatorList";
import { sendWebRtcControlMessage } from "../features/stream/streamWorkerClient";
import type {
  StreamConfig,
  StreamEncoder,
  StreamFps,
  StreamQualityPreset,
  StreamTransport,
} from "../features/stream/streamTypes";
import { useLiveStream } from "../features/stream/useLiveStream";
import { DebugPanel } from "../features/toolbar/DebugPanel";
import { Toolbar } from "../features/toolbar/Toolbar";
import { SimulatorViewport } from "../features/viewport/SimulatorViewport";
import type {
  Point,
  Size,
  TouchIndicator,
  ViewMode,
} from "../features/viewport/types";
import { useViewportLayout } from "../features/viewport/useViewportLayout";
import { NewSimulatorModal } from "../features/simulators/NewSimulatorModal";
import { nextViewportWheelPanState } from "../features/viewport/viewportWheel";
import {
  buildShellRotationTransform,
  clampPan,
  clampZoom,
  computeChromeScreenBorderRadius,
  computeChromeScreenRect,
  normalizeQuarterTurns,
  screenAspectRatio,
  shellSize,
} from "../features/viewport/viewportMath";
import {
  DEVICE_SCREEN_WIDTH,
  ZOOM_ANIMATION_MS,
  ZOOM_STEP,
} from "../shared/constants";
import { useElementSize } from "../shared/hooks/useElementSize";
import {
  ACCESSIBILITY_SOURCE_STORAGE_KEY,
  clearLegacyVolatileUiState,
  DEFAULT_VIEWPORT_STATE,
  DEBUG_VISIBLE_STORAGE_KEY,
  DEVTOOLS_VISIBLE_STORAGE_KEY,
  HIERARCHY_VISIBLE_STORAGE_KEY,
  nextAccessibilitySourcePreference,
  readPersistedUiState,
  readStoredAccessibilitySource,
  readStoredFlag,
  sanitizeAccessibilitySources,
  TOUCH_OVERLAY_VISIBLE_STORAGE_KEY,
  viewportStateForUDID,
  writePersistedUiState,
  writeStoredFlag,
} from "./uiState";

const ACCESSIBILITY_REFRESH_MS = 1500;
const REACT_NATIVE_ACCESSIBILITY_REFRESH_MS = 500;
const FLUTTER_ACCESSIBILITY_REFRESH_MS = 1000;
const ANDROID_METADATA_REFRESH_MS = 1000;
const DEFAULT_ACCESSIBILITY_MAX_DEPTH = 10;
const LOGICAL_INSPECTOR_MAX_DEPTH = 80;
const FLUTTER_INSPECTOR_MAX_DEPTH = 48;
const AUTH_REQUIRED_MESSAGE = "SimDeck API access token is required.";
const NOT_CONNECTED_MESSAGE = "Not connected";
const LOCAL_STREAM_DEFAULTS: StreamConfig = {
  encoder: "auto",
  fps: 60,
  quality: "full",
};
const REMOTE_STREAM_DEFAULTS: StreamConfig = {
  encoder: "software",
  fps: 30,
  quality: "balanced",
};
const H264_WS_DEFAULT_FPS = 60;
const H264_WS_LOCAL_DEFAULT_QUALITY: StreamQualityPreset = "full";
const H264_WS_REMOTE_DEFAULT_QUALITY: StreamQualityPreset = "auto";
const CONTROL_BACKLOG_DROP_BYTES = 4096;
const STREAM_CONFIG_USER_CHANGE_GRACE_MS = 1000;
const STREAM_ENCODER_VALUES = new Set<StreamEncoder>([
  "auto",
  "hardware",
  "software",
]);
const STREAM_TRANSPORT_VALUES = new Set<StreamTransport>([
  "auto",
  "h264",
  "webrtc",
]);
const MOBILE_VIEWPORT_MEDIA_QUERY = "(max-width: 600px)";
clearLegacyVolatileUiState();

interface StreamQualityResponse {
  ok?: boolean;
  quality?: {
    fps?: number;
    maxEdge?: number;
    profile?: string;
    videoCodec?: string;
  };
  videoCodec?: string;
}

function buildChromeUrl(
  udid: string,
  stamp: string,
  includeButtons = true,
): string {
  return buildAuthenticatedAssetUrl(
    `/api/simulators/${udid}/chrome.png`,
    stamp,
    includeButtons ? undefined : { buttons: "false" },
  );
}

function buildChromeButtonUrl(
  udid: string,
  button: string,
  pressed: boolean,
  stamp: string,
): string {
  return buildAuthenticatedAssetUrl(
    `/api/simulators/${udid}/chrome-button/${encodeURIComponent(button)}.png`,
    stamp,
    pressed ? { pressed: "true" } : undefined,
  );
}

function buildScreenMaskUrl(udid: string, stamp: string): string {
  return buildAuthenticatedAssetUrl(
    `/api/simulators/${udid}/screen-mask.png`,
    stamp,
  );
}

function buildAuthenticatedAssetUrl(
  path: string,
  stamp: string,
  params?: Record<string, string>,
): string {
  const url = new URL(apiUrl(path), window.location.href);
  url.searchParams.set("stamp", String(stamp));
  for (const [key, value] of Object.entries(params ?? {})) {
    url.searchParams.set(key, value);
  }
  const token = accessTokenFromLocation();
  if (token) {
    url.searchParams.set("simdeckToken", token);
  }
  return url.toString();
}

function shouldUseRemoteStreamDefault(apiRoot: string): boolean {
  if (apiRoot) {
    return true;
  }
  return (
    new URLSearchParams(window.location.search).get("remoteStream") === "1"
  );
}

function readStreamTransportQueryParam(): StreamTransport {
  const value = new URLSearchParams(window.location.search).get("stream");
  if (value === "h264-ws") {
    return "h264";
  }
  return value && STREAM_TRANSPORT_VALUES.has(value as StreamTransport)
    ? (value as StreamTransport)
    : "auto";
}

function defaultStreamConfigForTransport(
  remote: boolean,
  transport: StreamTransport,
): StreamConfig {
  const base = remote ? REMOTE_STREAM_DEFAULTS : LOCAL_STREAM_DEFAULTS;
  if (transport === "h264") {
    return {
      ...base,
      fps: H264_WS_DEFAULT_FPS,
      maxEdge: undefined,
      quality: remote
        ? H264_WS_REMOTE_DEFAULT_QUALITY
        : H264_WS_LOCAL_DEFAULT_QUALITY,
    };
  }
  return base;
}

function shouldForceInitialFitMode(): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  return (
    window.matchMedia?.(MOBILE_VIEWPORT_MEDIA_QUERY).matches ??
    window.innerWidth <= 600
  );
}

function writeStreamTransportQueryParam(transport: StreamTransport) {
  const url = new URL(window.location.href);
  if (transport === "auto") {
    url.searchParams.delete("stream");
  } else {
    url.searchParams.set("stream", transport);
  }
  window.history.replaceState(
    null,
    "",
    `${url.pathname}${url.search}${url.hash}`,
  );
}

function downloadBlob(blob: Blob, fileName: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

function captureFileBaseName(
  simulator: SimulatorMetadata,
  artifact: "Recording" | "Screenshot",
): string {
  const safeName = simulator.name.replace(/[^A-Za-z0-9._-]+/g, "-");
  return `SimDeck ${artifact} - ${safeName || simulator.udid}`;
}

function simulatorDisplaySize(
  simulator: SimulatorMetadata | null,
): Size | null {
  const display = simulator?.privateDisplay;
  if (!display || display.displayWidth <= 0 || display.displayHeight <= 0) {
    return null;
  }
  return {
    width: display.displayWidth,
    height: display.displayHeight,
  };
}

function simulatorDisplayReady(simulator: SimulatorMetadata): boolean {
  const display = simulator.privateDisplay;
  return Boolean(
    simulator.isBooted &&
    display?.displayReady &&
    display.displayWidth > 0 &&
    display.displayHeight > 0,
  );
}

function normalizeSimulatorRotationQuarterTurns(
  simulator: SimulatorMetadata | null,
): number | null {
  const display = simulator?.privateDisplay;
  if (
    !simulator?.isBooted ||
    !display?.displayReady ||
    !Number.isFinite(display.rotationQuarterTurns)
  ) {
    return null;
  }
  return normalizeQuarterTurns(display.rotationQuarterTurns ?? 0);
}

function mergeAccessibilitySources(
  ...sources: unknown[]
): AccessibilitySource[] {
  return sanitizeAccessibilitySources(sources.flat());
}

function simulatorMatchesIdentifier(
  simulator: SimulatorMetadata,
  identifier: string,
): boolean {
  const normalized = identifier.trim().toLowerCase();
  if (!normalized) {
    return false;
  }
  return [
    simulator.udid,
    simulator.name,
    simulator.deviceTypeName,
    simulator.deviceTypeIdentifier,
  ].some((value) => value?.toLowerCase() === normalized);
}

type SimulatorTransition = {
  kind: "boot" | "shutdown";
  udid: string;
};

type AppInstallState = {
  fileName?: string;
  phase: "dragging" | "installing" | "installed";
};

export interface AppShellProps {
  apiRoot?: string;
  fixedSimulatorUDID?: string | null;
  hideSimulatorSelection?: boolean;
  pairingEnabled?: boolean;
  remoteStream?: boolean;
}

export function AppShell({
  apiRoot = "",
  fixedSimulatorUDID = null,
  hideSimulatorSelection = false,
  pairingEnabled = true,
  remoteStream = shouldUseRemoteStreamDefault(apiRoot),
}: AppShellProps = {}) {
  configureSimDeckClient({ apiRoot });
  const initialStreamTransportRef = useRef<StreamTransport>(
    readStreamTransportQueryParam(),
  );
  const [initialUiState] = useState(readPersistedUiState);
  const [initialSelectedUDID] = useState(
    () =>
      fixedSimulatorUDID ??
      readDeviceQueryParam() ??
      initialUiState.selectedUDID,
  );
  const forceInitialFitMode = shouldForceInitialFitMode();
  const initialViewportState = initialSelectedUDID
    ? viewportStateForUDID(initialUiState, initialSelectedUDID, {
        forceFit: forceInitialFitMode,
      })
    : DEFAULT_VIEWPORT_STATE;
  const {
    error: listError,
    isLoading,
    refresh,
    simulators,
    updateSimulator,
  } = useSimulatorList({ remote: remoteStream });
  const providerDisconnected = isProviderDisconnected(listError);
  const [debugVisible, setDebugVisible] = useState(() =>
    readStoredFlag(DEBUG_VISIBLE_STORAGE_KEY),
  );
  const [hierarchyVisible, setHierarchyVisible] = useState(() =>
    readStoredFlag(HIERARCHY_VISIBLE_STORAGE_KEY),
  );
  const [devToolsVisible, setDevToolsVisible] = useState(() =>
    readStoredFlag(DEVTOOLS_VISIBLE_STORAGE_KEY, false),
  );
  const [selectedUDID, setSelectedUDID] = useState(initialSelectedUDID ?? "");
  const [search, setSearch] = useState("");
  const openURLValueRef = useRef(
    initialUiState.openURLValue ?? "https://example.com",
  );
  const bundleIDValueRef = useRef(
    initialUiState.bundleIDValue ?? "com.apple.Preferences",
  );
  const [menuOpen, setMenuOpen] = useState(false);
  const [newSimulatorOpen, setNewSimulatorOpen] = useState(false);
  const [localError, setLocalError] = useState("");
  const [failedStreamUDIDs, setFailedStreamUDIDs] = useState<Set<string>>(
    () => new Set(),
  );
  const [pairingCode, setPairingCode] = useState("");
  const [pairingError, setPairingError] = useState("");
  const [pairingBusy, setPairingBusy] = useState(false);
  const [simulatorTransition, setSimulatorTransition] =
    useState<SimulatorTransition | null>(null);
  const [appInstallState, setAppInstallState] =
    useState<AppInstallState | null>(null);
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
  const [chromeProfileReady, setChromeProfileReady] = useState(false);
  const [chromeLoaded, setChromeLoaded] = useState(false);
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
  const [touchOverlayVisible, setTouchOverlayVisible] = useState(() =>
    readStoredFlag(TOUCH_OVERLAY_VISIBLE_STORAGE_KEY, true),
  );
  const [streamConfig, setStreamConfig] = useState<StreamConfig>(() =>
    defaultStreamConfigForTransport(
      remoteStream,
      initialStreamTransportRef.current,
    ),
  );
  const [streamTransport, setStreamTransport] = useState<StreamTransport>(
    initialStreamTransportRef.current,
  );
  const [streamConfigApplyKey, setStreamConfigApplyKey] = useState(0);
  const [streamConfigReady, setStreamConfigReady] = useState(false);
  const [touchIndicators, setTouchIndicators] = useState<TouchIndicator[]>([]);
  const [selectedSimulatorState, setSelectedSimulatorState] =
    useState<SimulatorStateResponse | null>(null);

  const menuRef = useRef<HTMLDivElement | null>(null);
  const appInstallInputRef = useRef<HTMLInputElement | null>(null);
  const appInstallDragDepthRef = useRef(0);
  const appInstallStatusTimeoutRef = useRef(0);
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
  const touchIndicatorTimeoutRef = useRef<number>(0);
  const gestureStartZoomRef = useRef(1);
  const effectiveZoomRef = useRef(initialViewportState.zoom ?? 1);
  const panRef = useRef<Point>(initialViewportState.pan);
  const applyZoomAtClientPointRef = useRef<
    (nextScale: number, clientX: number, clientY: number) => void
  >(() => {});
  const accessibilityRequestIdRef = useRef(0);
  const accessibilityLoadingRef = useRef(false);
  const streamConfigRequestIdRef = useRef(0);
  const streamConfigUserChangeAtRef = useRef(0);
  const streamConfigUserTouchedRef = useRef(false);
  const controlSocketRef = useRef<{
    udid: string;
    socket: WebSocket;
    pending: string[];
  } | null>(null);
  const pendingTouchMoveRef = useRef<{
    coords: Point;
    udid: string;
  } | null>(null);
  const touchMoveFrameRef = useRef(0);
  const refreshRef = useRef(refresh);
  const previousAndroidDisplayKeyRef = useRef("");
  const previousAndroidViewportSizeKeyRef = useRef("");
  const canvasSize = useElementSize(outerCanvasElement);
  const zoomDockSize = useElementSize(zoomDockElement);
  refreshRef.current = refresh;

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

  const baseSelectedSimulator =
    (fixedSimulatorUDID
      ? (simulators.find(
          (simulator) => simulator.udid === fixedSimulatorUDID,
        ) ??
        simulators.find((simulator) =>
          simulatorMatchesIdentifier(simulator, fixedSimulatorUDID),
        ))
      : null) ??
    simulators.find((simulator) => simulator.udid === selectedUDID) ??
    simulators.find((simulator) =>
      simulatorMatchesIdentifier(simulator, selectedUDID),
    ) ??
    filteredSimulators.find((simulator) => simulatorDisplayReady(simulator)) ??
    filteredSimulators.find((simulator) => simulator.isBooted) ??
    filteredSimulators[0] ??
    null;
  const selectedSimulator =
    baseSelectedSimulator &&
    selectedSimulatorState?.udid === baseSelectedSimulator.udid
      ? {
          ...baseSelectedSimulator,
          ...selectedSimulatorState.simulator,
        }
      : baseSelectedSimulator;
  const selectedSimulatorTransitionKind =
    selectedSimulator != null &&
    simulatorTransition?.udid === selectedSimulator.udid
      ? simulatorTransition.kind
      : null;
  const selectedSimulatorDetail =
    selectedSimulatorTransitionKind === "boot"
      ? "Starting..."
      : selectedSimulatorTransitionKind === "shutdown"
        ? "Stopping..."
        : selectedSimulator != null
          ? simulatorRuntimeLabel(selectedSimulator)
          : "";
  const simulatorStatusOverlayLabel =
    selectedSimulator != null &&
    simulatorTransition?.udid === selectedSimulator.udid
      ? simulatorTransition.kind === "boot"
        ? "Booting..."
        : "Stopping..."
      : "";

  useEffect(() => {
    const udid = baseSelectedSimulator?.udid;
    if (!udid) {
      setSelectedSimulatorState(null);
      return;
    }

    let cancelled = false;
    let timeoutId = 0;
    let controller: AbortController | null = null;

    const loadState = () => {
      controller?.abort();
      controller =
        typeof AbortController !== "undefined" ? new AbortController() : null;
      void fetchSimulatorState(
        udid,
        controller ? { signal: controller.signal } : {},
      )
        .then((state) => {
          if (!cancelled) {
            setSelectedSimulatorState(state);
          }
        })
        .catch((error) => {
          if (
            !cancelled &&
            !(error instanceof DOMException && error.name === "AbortError")
          ) {
            setSelectedSimulatorState(null);
          }
        })
        .finally(() => {
          if (!cancelled) {
            timeoutId = window.setTimeout(
              loadState,
              baseSelectedSimulator?.isBooted ? 1000 : 3000,
            );
          }
        });
    };

    setSelectedSimulatorState(null);
    loadState();
    return () => {
      cancelled = true;
      if (timeoutId) {
        window.clearTimeout(timeoutId);
      }
      controller?.abort();
    };
  }, [baseSelectedSimulator?.isBooted, baseSelectedSimulator?.udid]);

  const handleStreamCanvasRef = useCallback(
    (node: HTMLCanvasElement | null) => {
      streamCanvasRef.current = node;
      setStreamCanvasElement(node);
    },
    [],
  );

  const syncStreamConfig = useCallback(async () => {
    const requestId = ++streamConfigRequestIdRef.current;
    try {
      const response = await apiRequest<StreamQualityResponse>(
        "/api/stream-quality",
      );
      if (requestId !== streamConfigRequestIdRef.current) {
        return;
      }
      if (
        Date.now() - streamConfigUserChangeAtRef.current <
        STREAM_CONFIG_USER_CHANGE_GRACE_MS
      ) {
        return;
      }
      if (streamTransport === "h264" && !streamConfigUserTouchedRef.current) {
        return;
      }
      setStreamConfig((current) =>
        mergeStreamQualityResponse(current, response, {
          preserveAutoQuality: streamTransport === "h264",
        }),
      );
    } catch {
      // Keep the existing local/default selection; the stream path will surface
      // provider reachability errors separately.
    } finally {
      if (requestId === streamConfigRequestIdRef.current) {
        setStreamConfigReady(true);
      }
    }
  }, [streamTransport]);

  useEffect(() => {
    let cancelled = false;
    setStreamConfigReady(false);

    const run = () => {
      if (!cancelled) {
        void syncStreamConfig();
      }
    };

    run();
    return () => {
      cancelled = true;
    };
  }, [remoteStream, syncStreamConfig]);

  const {
    deviceNaturalSize,
    error: streamError,
    fps,
    hasFrame,
    runtimeInfo,
    stats,
    status: streamStatus,
    streamBackend,
    streamCanvasKey,
  } = useLiveStream({
    canvasElement: streamCanvasElement,
    paused: !streamConfigReady,
    remote: remoteStream,
    simulator: selectedSimulator,
    streamConfig,
    streamConfigApplyKey,
    streamTransport,
  });

  useEffect(() => {
    if (
      streamStatus.state !== "error" ||
      !isStreamProviderDisconnectError(streamStatus.error)
    ) {
      return;
    }
    void refreshRef.current();
  }, [streamStatus.error, streamStatus.state]);

  const updateStreamEncoder = useCallback((encoder: StreamEncoder) => {
    streamConfigUserTouchedRef.current = true;
    streamConfigUserChangeAtRef.current = Date.now();
    setStreamConfigReady(true);
    setStreamConfigApplyKey((current) => current + 1);
    setStreamConfig((current) => ({ ...current, encoder }));
  }, []);

  const updateStreamFps = useCallback((fps: StreamFps) => {
    streamConfigUserTouchedRef.current = true;
    streamConfigUserChangeAtRef.current = Date.now();
    setStreamConfigReady(true);
    setStreamConfigApplyKey((current) => current + 1);
    setStreamConfig((current) => ({ ...current, fps }));
  }, []);

  const updateStreamQuality = useCallback((quality: StreamQualityPreset) => {
    streamConfigUserTouchedRef.current = true;
    streamConfigUserChangeAtRef.current = Date.now();
    setStreamConfigReady(true);
    setStreamConfigApplyKey((current) => current + 1);
    setStreamConfig((current) => ({ ...current, maxEdge: undefined, quality }));
  }, []);

  const updateStreamTransport = useCallback(
    (transport: StreamTransport) => {
      setStreamTransport(transport);
      writeStreamTransportQueryParam(transport);
      if (transport !== "h264" || streamConfigUserTouchedRef.current) {
        return;
      }
      streamConfigUserChangeAtRef.current = Date.now();
      setStreamConfigReady(true);
      setStreamConfigApplyKey((current) => current + 1);
      setStreamConfig((current) => ({
        ...current,
        fps: H264_WS_DEFAULT_FPS,
        maxEdge: undefined,
        quality: remoteStream
          ? H264_WS_REMOTE_DEFAULT_QUALITY
          : H264_WS_LOCAL_DEFAULT_QUALITY,
      }));
    },
    [remoteStream],
  );

  useEffect(() => {
    if (
      !selectedSimulator ||
      !streamError ||
      readDeviceQueryParam() ||
      fixedSimulatorUDID ||
      !isStreamAttachFailure(streamError)
    ) {
      return;
    }
    const failedUDID = selectedSimulator.udid;
    setFailedStreamUDIDs((current) => {
      if (current.has(failedUDID)) {
        return current;
      }
      return new Set(current).add(failedUDID);
    });
    const nextSimulator = simulators.find(
      (simulator) =>
        simulator.isBooted &&
        simulator.udid !== failedUDID &&
        !failedStreamUDIDs.has(simulator.udid),
    );
    if (nextSimulator) {
      setSelectedUDID(nextSimulator.udid);
      setLocalError(
        `${selectedSimulator.name} did not expose a live simulator screen. Switched to ${nextSimulator.name}.`,
      );
    }
  }, [
    failedStreamUDIDs,
    fixedSimulatorUDID,
    selectedSimulator,
    simulators,
    streamError,
  ]);
  const shouldRenderChrome =
    selectedSimulator != null && shouldRenderNativeChrome(selectedSimulator);
  const viewportChromeProfile = shouldRenderChrome ? chromeProfile : null;
  const isAndroidViewport = isAndroidSimulator(selectedSimulator);
  const selectedHasFixedOrientation =
    selectedSimulator != null &&
    simulatorHasFixedOrientation(selectedSimulator);
  const androidDisplayKey =
    isAndroidViewport && selectedSimulator
      ? androidDisplayKeyForSimulator(selectedSimulator)
      : "";
  const effectiveDeviceNaturalSize = useMemo(() => {
    const displaySize = simulatorDisplaySize(selectedSimulator);
    if (isAndroidViewport) {
      return deviceNaturalSize ?? displaySize;
    }
    return (
      deviceNaturalSize ??
      (!shouldRenderChrome && chromeProfile
        ? {
            width: chromeProfile.screenWidth,
            height: chromeProfile.screenHeight,
          }
        : displaySize)
    );
  }, [
    chromeProfile,
    deviceNaturalSize,
    isAndroidViewport,
    selectedSimulator,
    shouldRenderChrome,
  ]);
  const androidViewportSizeKey =
    isAndroidViewport && effectiveDeviceNaturalSize
      ? `${Math.round(effectiveDeviceNaturalSize.width)}x${Math.round(effectiveDeviceNaturalSize.height)}`
      : "";

  const zoomDockReservedHeight =
    zoomDockElement && typeof window !== "undefined"
      ? (zoomDockSize?.height ?? 0) +
        Number.parseFloat(
          window.getComputedStyle(zoomDockElement).bottom || "0",
        )
      : 0;

  const { fitScale, effectiveZoom } = useViewportLayout({
    canvasSize,
    chromeProfile: viewportChromeProfile,
    deviceNaturalSize: effectiveDeviceNaturalSize,
    pan,
    rotationQuarterTurns,
    reservedBottomInset: zoomDockReservedHeight,
    viewMode,
    zoom,
  });

  useEffect(() => {
    effectiveZoomRef.current = effectiveZoom;
  }, [effectiveZoom]);

  useEffect(() => {
    panRef.current = pan;
  }, [pan]);

  const isBooted = Boolean(selectedSimulator?.isBooted);
  const isInstallingApp = appInstallState?.phase === "installing";
  const canInstallApp = Boolean(
    selectedSimulator?.isBooted && !isInstallingApp,
  );
  const appInstallAccept = isAndroidViewport ? ".apk" : ".ipa";
  const appInstallOverlayLabel = appInstallState
    ? appInstallStatusLabel(
        appInstallState,
        selectedSimulator,
        isAndroidViewport,
      )
    : "";
  const autoViewportOffsetY =
    viewMode === "manual" ? 0 : -zoomDockReservedHeight / 2;
  const screenAspect = screenAspectRatio(effectiveDeviceNaturalSize);
  const chromeHasInteractiveButtons = Boolean(
    viewportChromeProfile?.buttons?.length,
  );
  const chromeHasCrown = Boolean(
    viewportChromeProfile?.buttons?.some(
      (button) =>
        button.type?.toLowerCase() === "crown" ||
        button.name.toLowerCase() === "digital-crown",
    ),
  );
  const chromeAssetStamp = [
    selectedSimulator?.deviceTypeIdentifier,
    selectedSimulator?.deviceTypeName,
    selectedSimulator?.runtimeIdentifier,
    selectedSimulator?.runtimeName,
    selectedSimulator?.udid,
    chromeHasInteractiveButtons ? "buttons" : "no-buttons",
    chromeHasCrown ? "crown" : "no-crown",
  ]
    .filter(Boolean)
    .join(":");
  const chromeUrl = selectedSimulator
    ? buildChromeUrl(
        selectedSimulator.udid,
        chromeAssetStamp,
        !chromeHasInteractiveButtons || chromeHasCrown,
      )
    : "";
  const chromeButtonUrl = useCallback(
    (button: string, pressed = false) =>
      selectedSimulator
        ? buildChromeButtonUrl(
            selectedSimulator.udid,
            button,
            pressed,
            chromeAssetStamp,
          )
        : "",
    [chromeAssetStamp, selectedSimulator?.udid],
  );
  const chromeUsesAsset = Boolean(viewportChromeProfile && chromeUrl);
  const chromeRequired = Boolean(
    (shouldRenderChrome && !chromeProfileReady) || chromeUsesAsset,
  );
  const chromeAssetUrls = useMemo(() => {
    if (!chromeRequired || !selectedSimulator || !viewportChromeProfile) {
      return [];
    }
    const urls = new Set<string>();
    if (chromeUrl) {
      urls.add(chromeUrl);
    }
    if (viewportChromeProfile.hasScreenMask) {
      urls.add(buildScreenMaskUrl(selectedSimulator.udid, chromeAssetStamp));
    }
    for (const button of viewportChromeProfile.buttons ?? []) {
      urls.add(chromeButtonUrl(button.name, false));
      if (button.imageDownName) {
        urls.add(chromeButtonUrl(button.name, true));
      }
    }
    return [...urls].filter(Boolean);
  }, [
    chromeButtonUrl,
    chromeRequired,
    chromeUrl,
    chromeAssetStamp,
    selectedSimulator?.udid,
    viewportChromeProfile,
  ]);
  const simulatorRotationQuarterTurns =
    normalizeSimulatorRotationQuarterTurns(selectedSimulator);

  useEffect(() => {
    writeStoredFlag(DEBUG_VISIBLE_STORAGE_KEY, debugVisible);
  }, [debugVisible]);

  useEffect(() => {
    writeStoredFlag(HIERARCHY_VISIBLE_STORAGE_KEY, hierarchyVisible);
  }, [hierarchyVisible]);

  useEffect(() => {
    writeStoredFlag(DEVTOOLS_VISIBLE_STORAGE_KEY, devToolsVisible);
  }, [devToolsVisible]);

  useEffect(() => {
    writeStoredFlag(TOUCH_OVERLAY_VISIBLE_STORAGE_KEY, touchOverlayVisible);
  }, [touchOverlayVisible]);

  const toggleDevTools = useCallback(() => {
    setDevToolsVisible((current) => !current);
  }, []);

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
      bundleIDValue: bundleIDValueRef.current,
      openURLValue: openURLValueRef.current,
      selectedUDID,
    }));
  }, [selectedUDID]);

  useEffect(() => {
    if (search && simulators.length > 0 && filteredSimulators.length === 0) {
      setSearch("");
    }
  }, [filteredSimulators.length, search, simulators.length]);

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
    if (
      !fixedSimulatorUDID &&
      selectedSimulator &&
      selectedSimulator.udid !== selectedUDID
    ) {
      setSelectedUDID(selectedSimulator.udid);
    }
  }, [fixedSimulatorUDID, selectedSimulator, selectedUDID]);

  useEffect(() => {
    if (!selectedSimulator) {
      setStreamStamp(Date.now());
      setChromeProfile(null);
      setChromeProfileReady(false);
      setLocalError("");
      setAccessibilityRoots([]);
      setAccessibilitySelectedId("");
      setAccessibilityHoveredId(null);
      setAccessibilityPickerActive(false);
      setAccessibilityError("");
      setAccessibilitySource("");
      setAccessibilityAvailableSources([]);
      accessibilityRequestIdRef.current += 1;
      accessibilityLoadingRef.current = false;
      setAccessibilityLoading(false);
      return;
    }

    const persistedState = readPersistedUiState();
    const nextViewportState = viewportStateForUDID(
      persistedState,
      selectedSimulator.udid,
      { forceFit: shouldForceInitialFitMode() },
    );
    const keepCenteredViewportMode =
      viewMode === "fit" ||
      viewMode === "center" ||
      (viewMode === "manual" && zoom != null && Math.abs(zoom - 1) < 0.001);
    setStreamStamp(Date.now());
    setChromeProfile(null);
    setChromeProfileReady(false);
    setViewMode(
      keepCenteredViewportMode ? viewMode : nextViewportState.viewMode,
    );
    setZoom(
      keepCenteredViewportMode
        ? viewMode === "manual"
          ? 1
          : null
        : nextViewportState.zoom,
    );
    setPan(
      keepCenteredViewportMode
        ? {
            x: 0,
            y: viewMode === "manual" ? -zoomDockReservedHeight / 2 : 0,
          }
        : nextViewportState.pan,
    );
    setRotationQuarterTurns(nextViewportState.rotationQuarterTurns);
    setLocalError("");
    setAccessibilityRoots([]);
    setAccessibilitySelectedId(
      persistedState.accessibilitySelectedByUDID?.[selectedSimulator.udid] ??
        "",
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

    if (providerDisconnected) {
      setAccessibilityRoots([]);
      setAccessibilitySelectedId("");
      setAccessibilityHoveredId(null);
      setAccessibilityAvailableSources([]);
      setAccessibilitySource("");
      setAccessibilityError("Not connected");
      setAccessibilityLoading(false);
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
    setAccessibilityError((current) =>
      current === "Not connected" ? current : "",
    );

    try {
      const snapshot = await fetchAccessibilityTree(
        selectedSimulator.udid,
        accessibilityPreferredSource,
        {
          maxDepth:
            accessibilityPreferredSource === "native-ax"
              ? DEFAULT_ACCESSIBILITY_MAX_DEPTH
              : accessibilityPreferredSource === "flutter"
                ? FLUTTER_INSPECTOR_MAX_DEPTH
                : LOGICAL_INSPECTOR_MAX_DEPTH,
        },
      );
      if (accessibilityRequestIdRef.current !== requestId) {
        return;
      }
      const roots = snapshot.roots ?? [];
      const availableSources = mergeAccessibilitySources(
        sanitizeAccessibilitySources(snapshot.availableSources),
        snapshot.source,
      );
      setAccessibilityRoots(roots);
      setAccessibilityAvailableSources(availableSources);
      setAccessibilitySource(snapshot.source);
      setAccessibilityError(
        roots.length === 0
          ? userFacingAccessibilityError(snapshot.fallbackReason ?? "")
          : "",
      );
      const nextPreferredSource = nextAccessibilitySourcePreference(
        accessibilityPreferredSource,
        snapshot.source,
        availableSources,
      );
      if (nextPreferredSource) {
        setAccessibilityPreferredSource(nextPreferredSource);
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
          ? userFacingAccessibilityError(snapshotError.message)
          : "Failed to load accessibility hierarchy.",
      );
      setAccessibilityRoots([]);
      setAccessibilitySelectedId("");
      setAccessibilityHoveredId(null);
      setAccessibilitySource("");
      if (accessibilityPreferredSource !== "auto") {
        setAccessibilityPreferredSource("auto");
      }
    } finally {
      if (accessibilityRequestIdRef.current === requestId) {
        accessibilityLoadingRef.current = false;
        setAccessibilityLoading(false);
      }
    }
  }, [accessibilityPreferredSource, providerDisconnected, selectedSimulator]);

  const changeAccessibilitySource = useCallback(
    (source: AccessibilitySource) => {
      if (source === accessibilityPreferredSource) {
        return;
      }
      accessibilityRequestIdRef.current += 1;
      accessibilityLoadingRef.current = false;
      setAccessibilityPreferredSource(source);
      setAccessibilityRoots([]);
      setAccessibilitySelectedId("");
      setAccessibilityHoveredId(null);
      setAccessibilityError("");
      setAccessibilitySource("");
      setAccessibilityLoading(false);
    },
    [accessibilityPreferredSource],
  );

  useEffect(() => {
    if (!hierarchyVisible) {
      return;
    }

    const refreshMs =
      accessibilityPreferredSource === "react-native" ||
      accessibilitySource === "react-native"
        ? REACT_NATIVE_ACCESSIBILITY_REFRESH_MS
        : accessibilityPreferredSource === "flutter" ||
            accessibilitySource === "flutter"
          ? FLUTTER_ACCESSIBILITY_REFRESH_MS
          : ACCESSIBILITY_REFRESH_MS;
    let disposed = false;
    let timeout: number | null = null;
    const refreshLoop = async () => {
      const startedAt = Date.now();
      await loadAccessibilityTree();
      if (disposed) {
        return;
      }
      timeout = window.setTimeout(
        refreshLoop,
        Math.max(0, refreshMs - (Date.now() - startedAt)),
      );
    };
    void refreshLoop();
    return () => {
      disposed = true;
      if (timeout != null) {
        window.clearTimeout(timeout);
      }
    };
  }, [
    accessibilityPreferredSource,
    accessibilitySource,
    hierarchyVisible,
    loadAccessibilityTree,
  ]);

  useEffect(() => {
    if (!isBooted) {
      setAccessibilityPickerActive(false);
    }
  }, [isBooted]);

  useEffect(() => {
    if (isAndroidViewport) {
      setRotationQuarterTurns((current) =>
        normalizeQuarterTurns(current) === 0 ? current : 0,
      );
      return;
    }
    if (simulatorRotationQuarterTurns == null) {
      return;
    }
    setRotationQuarterTurns((current) => {
      const normalizedCurrent = normalizeQuarterTurns(current);
      if (normalizedCurrent === simulatorRotationQuarterTurns) {
        return current;
      }
      beginZoomAnimation();
      return simulatorRotationQuarterTurns;
    });
  }, [isAndroidViewport, simulatorRotationQuarterTurns]);

  useEffect(() => {
    if (!isAndroidViewport || !selectedSimulator?.isBooted) {
      return;
    }

    let cancelled = false;
    const refreshAndroidMetadata = () => {
      if (cancelled || document.visibilityState !== "visible") {
        return;
      }
      void refreshRef.current();
    };

    const intervalId = window.setInterval(
      refreshAndroidMetadata,
      ANDROID_METADATA_REFRESH_MS,
    );
    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [isAndroidViewport, selectedSimulator?.isBooted, selectedSimulator?.udid]);

  useEffect(() => {
    if (!isAndroidViewport || !androidDisplayKey) {
      previousAndroidDisplayKeyRef.current = "";
      return;
    }

    const previousKey = previousAndroidDisplayKeyRef.current;
    previousAndroidDisplayKeyRef.current = androidDisplayKey;
    if (previousKey && previousKey !== androidDisplayKey) {
      beginZoomAnimation();
    }
  }, [androidDisplayKey, isAndroidViewport]);

  useEffect(() => {
    if (!isAndroidViewport || !androidViewportSizeKey) {
      previousAndroidViewportSizeKeyRef.current = "";
      return;
    }

    const previousKey = previousAndroidViewportSizeKeyRef.current;
    previousAndroidViewportSizeKeyRef.current = androidViewportSizeKey;
    if (previousKey && previousKey !== androidViewportSizeKey) {
      beginZoomAnimation();
    }
  }, [androidViewportSizeKey, isAndroidViewport]);

  useEffect(() => {
    if (!chromeRequired) {
      setChromeLoaded(true);
      return;
    }
    if (chromeAssetUrls.length === 0) {
      setChromeLoaded(false);
      return;
    }

    let cancelled = false;
    let remaining = chromeAssetUrls.length;

    setChromeLoaded(false);

    function markComplete() {
      if (cancelled) {
        return;
      }
      remaining -= 1;
      if (remaining <= 0) {
        setChromeLoaded(true);
      }
    }

    const images = chromeAssetUrls.map((url) => {
      const image = new Image();
      let completed = false;
      const completeImage = () => {
        if (completed) {
          return;
        }
        completed = true;
        markComplete();
      };
      image.decoding = "async";
      image.onload = completeImage;
      image.onerror = completeImage;
      image.src = url;
      if (image.complete) {
        window.setTimeout(completeImage, 0);
      }
      return image;
    });

    return () => {
      cancelled = true;
      images.forEach((image) => {
        image.onload = null;
        image.onerror = null;
      });
    };
  }, [chromeAssetUrls, chromeRequired]);

  useEffect(() => {
    let cancelled = false;

    async function loadChromeProfile() {
      if (!selectedSimulator) {
        setChromeProfile(null);
        setChromeProfileReady(true);
        return;
      }
      try {
        const profile = await fetchChromeProfile(selectedSimulator.udid);
        if (!cancelled) {
          setChromeProfile(profile);
          setChromeProfileReady(true);
        }
      } catch {
        if (!cancelled) {
          setChromeProfile(null);
          setChromeProfileReady(true);
        }
      }
    }

    void loadChromeProfile();
    return () => {
      cancelled = true;
    };
  }, [
    selectedSimulator?.privateDisplay?.displayHeight,
    selectedSimulator?.privateDisplay?.displayWidth,
    selectedSimulator?.privateDisplay?.rotationQuarterTurns,
    selectedSimulator?.udid,
  ]);

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

    document.addEventListener("pointerdown", handleDocumentPointerDown, true);
    window.addEventListener("keydown", handleWindowKeyDown);
    return () => {
      document.removeEventListener(
        "pointerdown",
        handleDocumentPointerDown,
        true,
      );
      window.removeEventListener("keydown", handleWindowKeyDown);
    };
  }, [menuOpen]);

  useEffect(() => {
    function handleWindowKeyDown(event: KeyboardEvent) {
      if (
        isEditableTarget(event.target) ||
        event.altKey ||
        event.metaKey ||
        !event.ctrlKey ||
        !event.shiftKey ||
        event.key.toLowerCase() !== "d"
      ) {
        return;
      }

      event.preventDefault();
      event.stopImmediatePropagation();
      setDebugVisible((current) => !current);
    }

    window.addEventListener("keydown", handleWindowKeyDown);
    return () => {
      window.removeEventListener("keydown", handleWindowKeyDown);
    };
  }, []);

  useEffect(() => {
    setPan((currentPan) => {
      const nextPan = clampPan(
        currentPan,
        effectiveZoom,
        canvasSize,
        effectiveDeviceNaturalSize,
        viewportChromeProfile,
        rotationQuarterTurns,
        viewMode === "manual" ? zoomDockReservedHeight : 0,
      );
      return nextPan.x === currentPan.x && nextPan.y === currentPan.y
        ? currentPan
        : nextPan;
    });
  }, [
    canvasSize,
    effectiveDeviceNaturalSize,
    effectiveZoom,
    rotationQuarterTurns,
    viewportChromeProfile,
    viewMode,
    zoomDockReservedHeight,
  ]);

  useEffect(() => {
    return () => {
      if (zoomAnimationTimeoutRef.current) {
        clearTimeout(zoomAnimationTimeoutRef.current);
      }
      if (touchIndicatorTimeoutRef.current) {
        clearTimeout(touchIndicatorTimeoutRef.current);
      }
      if (appInstallStatusTimeoutRef.current) {
        clearTimeout(appInstallStatusTimeoutRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (!touchOverlayVisible) {
      setTouchIndicators([]);
    }
  }, [touchOverlayVisible]);

  useKeyboardInput({
    enabled: Boolean(selectedSimulator?.isBooted && selectedSimulator.udid),
    onKey: ({ keyCode, modifiers }) => {
      if (!selectedSimulator) {
        return;
      }
      sendControl(selectedSimulator.udid, { type: "key", keyCode, modifiers });
    },
  });

  const pointerInput = usePointerInput({
    canvasSize,
    chromeProfile: viewportChromeProfile,
    deviceNaturalSize: effectiveDeviceNaturalSize,
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
      sendTouchControl(selectedSimulator.udid, phase, coords);
    },
    onEdgeTouch: (phase, coords, edge) => {
      if (!selectedSimulator) {
        return;
      }
      if (phase === "began" && accessibilitySelectedId) {
        setAccessibilitySelectedId("");
        setAccessibilityHoveredId(null);
      }
      sendControl(selectedSimulator.udid, {
        type: "edgeTouch",
        ...coords,
        phase,
        edge,
      });
    },
    onMultiTouch: (phase: TouchPhase, first: Point, second: Point) => {
      if (!selectedSimulator) {
        return;
      }
      if (phase === "began" && accessibilitySelectedId) {
        setAccessibilitySelectedId("");
        setAccessibilityHoveredId(null);
      }
      sendControl(selectedSimulator.udid, {
        type: "multiTouch",
        x1: first.x,
        y1: first.y,
        x2: second.x,
        y2: second.y,
        phase,
      });
    },
    onTouchPreview: showTouchIndicator,
    onMultiTouchPreview: showTouchIndicators,
    pan,
    reservedBottomInset: zoomDockReservedHeight,
    rotationQuarterTurns,
    setPan,
  });

  const pairingRequired =
    !remoteStream &&
    pairingEnabled &&
    listError === AUTH_REQUIRED_MESSAGE &&
    !accessTokenFromLocation();
  const visibleListError = providerDisconnected
    ? NOT_CONNECTED_MESSAGE
    : remoteStream && listError === AUTH_REQUIRED_MESSAGE
      ? ""
      : selectedSimulator
        ? friendlyClientError(listError)
        : listError;
  const toolbarError = pairingRequired
    ? localError
    : localError || (selectedSimulator ? "" : visibleListError);
  const visibleStreamError = friendlyStreamError(streamStatus.error, {
    remote: remoteStream,
  });
  const streamStatusMessage = visibleStreamError
    ? streamStatus.detail
      ? `${visibleStreamError} ${streamStatus.detail}`
      : visibleStreamError
    : "";
  const streamStatusLabel = streamAgentStatusLabel({
    hasFrame,
    simulator: selectedSimulator,
    simulatorState: selectedSimulatorState,
    stats,
    streamStatusDetail: streamStatus.detail,
    streamStatusState: streamStatus.state,
  });
  const viewportStatusOverlayLabel =
    (providerDisconnected ? NOT_CONNECTED_MESSAGE : "") ||
    simulatorStatusOverlayLabel ||
    streamStatusMessage ||
    (selectedSimulator ? visibleListError : "");
  const viewportHasStreamError = Boolean(
    providerDisconnected ||
    streamStatus.state === "error" ||
    visibleStreamError ||
    (selectedSimulator && visibleListError),
  );
  const deviceTransform = `translate(${pan.x}px, ${pan.y + autoViewportOffsetY}px) scale(${effectiveZoom})`;
  const chromeScreenRect = computeChromeScreenRect(
    viewportChromeProfile,
    effectiveDeviceNaturalSize,
  );
  const chromeScreenBorderRadius = computeChromeScreenBorderRadius(
    viewportChromeProfile,
    chromeScreenRect,
  );
  const chromeScreenStyle =
    viewportChromeProfile && chromeScreenRect
      ? ({
          left: `${(chromeScreenRect.x / viewportChromeProfile.totalWidth) * 100}%`,
          top: `${(chromeScreenRect.y / viewportChromeProfile.totalHeight) * 100}%`,
          width: `${(chromeScreenRect.width / viewportChromeProfile.totalWidth) * 100}%`,
          height: `${(chromeScreenRect.height / viewportChromeProfile.totalHeight) * 100}%`,
          borderRadius: viewportChromeProfile.hasScreenMask
            ? "0"
            : (chromeScreenBorderRadius ?? "0"),
          ...(viewportChromeProfile.hasScreenMask && selectedSimulator
            ? {
                maskImage: `url("${buildScreenMaskUrl(
                  selectedSimulator.udid,
                  chromeAssetStamp,
                )}")`,
                maskMode: "alpha",
                maskRepeat: "no-repeat",
                maskSize: "100% 100%",
                WebkitMaskImage: `url("${buildScreenMaskUrl(
                  selectedSimulator.udid,
                  chromeAssetStamp,
                )}")`,
                WebkitMaskRepeat: "no-repeat",
                WebkitMaskSize: "100% 100%",
              }
            : {}),
        } satisfies CSSProperties)
      : null;
  const screenOnlyStyle =
    !viewportChromeProfile && chromeProfile && chromeProfile.screenWidth > 0
      ? isAndroidViewport
        ? androidScreenRadiusStyle(chromeProfile, effectiveDeviceNaturalSize)
        : ({
            borderRadius: `${Math.min(
              chromeProfile.cornerRadius *
                (DEVICE_SCREEN_WIDTH / chromeProfile.screenWidth),
              DEVICE_SCREEN_WIDTH / 2,
            )}px`,
          } satisfies CSSProperties)
      : null;
  const viewportScreenStyle = chromeScreenStyle ?? screenOnlyStyle;
  const shellStyle = viewportChromeProfile
    ? {
        width: `${viewportChromeProfile.totalWidth}px`,
        height: `${viewportChromeProfile.totalHeight}px`,
      }
    : null;
  const deviceFrameSize = shellSize(
    effectiveDeviceNaturalSize,
    viewportChromeProfile,
    rotationQuarterTurns,
  );
  const naturalShellSize = shellSize(
    effectiveDeviceNaturalSize,
    viewportChromeProfile,
  );
  const deviceFrameStyle = {
    width: `${deviceFrameSize.width}px`,
    height: `${deviceFrameSize.height}px`,
  };
  const devicePresentationStyle = {
    width: `${naturalShellSize.width}px`,
    height: `${naturalShellSize.height}px`,
    transform: buildShellRotationTransform(
      effectiveDeviceNaturalSize,
      viewportChromeProfile,
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

  async function downloadSimulatorScreenshot(withBezel: boolean) {
    if (!selectedSimulator) {
      return;
    }
    setLocalError("");
    try {
      const blob = await captureSimulatorScreenshot(selectedSimulator.udid, {
        withBezel,
      });
      downloadBlob(
        blob,
        `${captureFileBaseName(selectedSimulator, "Screenshot")}${withBezel ? " Bezel" : ""}.png`,
      );
    } catch (captureError) {
      setLocalError(
        captureError instanceof Error
          ? captureError.message
          : "Capture failed.",
      );
    }
  }

  async function downloadSimulatorRecording() {
    if (!selectedSimulator) {
      return;
    }
    setLocalError("");
    try {
      const blob = await recordSimulatorScreen(selectedSimulator.udid, 5);
      downloadBlob(
        blob,
        `${captureFileBaseName(selectedSimulator, "Recording")}.mp4`,
      );
    } catch (captureError) {
      setLocalError(
        captureError instanceof Error
          ? captureError.message
          : "Recording failed.",
      );
    }
  }

  function selectedStateFromSimulator(
    simulator: SimulatorMetadata,
    current: SimulatorStateResponse | null,
  ): SimulatorStateResponse {
    const display = simulator.privateDisplay;
    return {
      booted: simulator.isBooted,
      displayReady: display?.displayReady ?? false,
      displayStatus:
        display?.displayStatus ??
        (simulator.isBooted ? "Waiting for display" : "Boot required"),
      foregroundApp:
        current?.udid === simulator.udid ? current.foregroundApp : null,
      frameSequence: display?.frameSequence ?? 0,
      lastFrameAgeMs:
        current?.udid === simulator.udid ? current.lastFrameAgeMs : null,
      lastFrameAt: display?.lastFrameAt ?? 0,
      simulator,
      udid: simulator.udid,
    };
  }

  async function runSimulatorLifecycleAction(
    kind: SimulatorTransition["kind"],
    udid: string,
    action: () => Promise<SimulatorMetadata | null>,
  ) {
    setSimulatorTransition({ kind, udid });
    setLocalError("");
    try {
      const simulator = await action();
      if (simulator) {
        updateSimulator(simulator);
        setSelectedSimulatorState((current) =>
          current?.udid === simulator.udid ||
          baseSelectedSimulator?.udid === simulator.udid
            ? selectedStateFromSimulator(simulator, current)
            : current,
        );
      }
      setSimulatorTransition((current) =>
        current?.udid === udid && current.kind === kind ? null : current,
      );
      void refresh();
    } catch (actionError) {
      setLocalError(
        actionError instanceof Error ? actionError.message : "Request failed.",
      );
      setSimulatorTransition((current) =>
        current?.udid === udid && current.kind === kind ? null : current,
      );
    }
  }

  const closeControlSocket = useCallback(() => {
    const current = controlSocketRef.current;
    controlSocketRef.current = null;
    current?.socket.close();
  }, []);

  const ensureControlSocket = useCallback((udid: string) => {
    const current = controlSocketRef.current;
    if (
      current?.udid === udid &&
      current.socket.readyState !== WebSocket.CLOSING &&
      current.socket.readyState !== WebSocket.CLOSED
    ) {
      return current;
    }

    current?.socket.close();
    const socket = new WebSocket(simulatorControlSocketUrl(udid));
    const state = { udid, socket, pending: [] as string[] };
    controlSocketRef.current = state;

    socket.addEventListener("open", () => {
      for (const message of state.pending.splice(0)) {
        socket.send(message);
      }
    });
    socket.addEventListener("close", () => {
      if (controlSocketRef.current === state) {
        controlSocketRef.current = null;
      }
    });
    socket.addEventListener("error", () => {
      setLocalError("Simulator control stream disconnected.");
    });

    return state;
  }, []);

  function sendControl(udid: string, message: ControlMessage): boolean {
    if (message.type === "touch") {
      sendTouchControl(udid, message.phase, { x: message.x, y: message.y });
      return true;
    }
    return sendControlNow(udid, message);
  }

  function sendTouchControl(udid: string, phase: TouchPhase, coords: Point) {
    if (phase === "moved") {
      pendingTouchMoveRef.current = { coords, udid };
      if (!touchMoveFrameRef.current) {
        touchMoveFrameRef.current = window.requestAnimationFrame(() => {
          touchMoveFrameRef.current = 0;
          flushPendingTouchMove();
        });
      }
      return;
    }

    flushPendingTouchMove();
    sendControlNow(udid, { type: "touch", ...coords, phase });
  }

  function flushPendingTouchMove() {
    const pending = pendingTouchMoveRef.current;
    pendingTouchMoveRef.current = null;
    if (touchMoveFrameRef.current) {
      window.cancelAnimationFrame(touchMoveFrameRef.current);
      touchMoveFrameRef.current = 0;
    }
    if (!pending) {
      return;
    }
    sendControlNow(pending.udid, {
      type: "touch",
      ...pending.coords,
      phase: "moved",
    });
  }

  function sendControlNow(udid: string, message: ControlMessage): boolean {
    setLocalError("");
    const encoded = JSON.stringify(message);
    const dropIfBacklogged =
      message.type === "touch" && message.phase === "moved";
    if (sendWebRtcControlMessage(encoded, { dropIfBacklogged })) {
      return true;
    }
    if (remoteStream) {
      return false;
    }
    const state = ensureControlSocket(udid);
    if (state.socket.readyState === WebSocket.OPEN) {
      if (
        dropIfBacklogged &&
        state.socket.bufferedAmount > CONTROL_BACKLOG_DROP_BYTES
      ) {
        return true;
      }
      state.socket.send(encoded);
    } else {
      if (dropIfBacklogged) {
        const lastIndex = state.pending.length - 1;
        if (lastIndex >= 0 && state.pending[lastIndex].includes('"moved"')) {
          state.pending[lastIndex] = encoded;
          return true;
        }
      }
      state.pending.push(encoded);
    }
    return true;
  }

  useEffect(() => {
    return () => {
      if (touchMoveFrameRef.current) {
        window.cancelAnimationFrame(touchMoveFrameRef.current);
        touchMoveFrameRef.current = 0;
      }
      pendingTouchMoveRef.current = null;
      closeControlSocket();
    };
  }, [closeControlSocket]);

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

  function applyZoom(nextScale: number, nextPan?: Point, animate = true) {
    const panForClamp = nextPan ?? {
      x: panRef.current.x,
      y: panRef.current.y + autoViewportOffsetY,
    };
    const clampedScale = clampZoom(nextScale, fitScale);
    const clampedPan = clampPan(
      panForClamp,
      clampedScale,
      canvasSize,
      effectiveDeviceNaturalSize,
      viewportChromeProfile,
      rotationQuarterTurns,
      zoomDockReservedHeight,
    );
    effectiveZoomRef.current = clampedScale;
    panRef.current = clampedPan;
    if (animate) {
      beginZoomAnimation();
    }
    setViewMode("manual");
    setZoom(clampedScale);
    setPan(clampedPan);
  }

  function applyZoomAtClientPoint(
    nextScale: number,
    clientX: number,
    clientY: number,
  ) {
    const canvasRect = outerCanvasRef.current?.getBoundingClientRect();
    if (!canvasRect) {
      applyZoom(
        nextScale,
        {
          x: panRef.current.x,
          y: panRef.current.y + autoViewportOffsetY,
        },
        false,
      );
      return;
    }

    const currentZoom = effectiveZoomRef.current;
    const currentPan = panRef.current;
    const clampedScale = clampZoom(nextScale, fitScale);
    const ratio = clampedScale / Math.max(currentZoom, 0.001);
    const cursor = {
      x: clientX - (canvasRect.left + canvasRect.width / 2),
      y: clientY - (canvasRect.top + canvasRect.height / 2),
    };
    const currentVisualPan = {
      x: currentPan.x,
      y: currentPan.y + autoViewportOffsetY,
    };
    const nextVisualPan = {
      x: cursor.x - (cursor.x - currentVisualPan.x) * ratio,
      y: cursor.y - (cursor.y - currentVisualPan.y) * ratio,
    };
    applyZoom(
      clampedScale,
      {
        x: nextVisualPan.x,
        y: nextVisualPan.y,
      },
      false,
    );
  }

  useEffect(() => {
    applyZoomAtClientPointRef.current = applyZoomAtClientPoint;
  });

  function handleViewportWheel(event: React.WheelEvent<HTMLElement>) {
    if (!selectedSimulator) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();
    const deltaScale =
      event.deltaMode === WheelEvent.DOM_DELTA_LINE
        ? 16
        : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
          ? 240
          : 1;
    const deltaX = event.deltaX * deltaScale;
    const deltaY = event.deltaY * deltaScale;

    if (event.ctrlKey || event.metaKey) {
      const nextScale = effectiveZoom * Math.exp(-deltaY * 0.002);
      applyZoomAtClientPoint(nextScale, event.clientX, event.clientY);
      return;
    }

    if (chromeHasCrown && selectedSimulator.isBooted) {
      sendCrownRotation(deltaY);
      return;
    }

    setPan(
      (currentPan) =>
        nextViewportWheelPanState({
          canvasSize,
          chromeProfile: viewportChromeProfile,
          deltaX,
          deltaY,
          deviceNaturalSize: effectiveDeviceNaturalSize,
          effectiveZoom,
          fitScale,
          pan: currentPan,
          reservedBottomInset: zoomDockReservedHeight,
          rotationQuarterTurns,
          viewMode,
          zoom,
        }).pan,
    );
  }

  function showTouchIndicator(phase: TouchPhase, coords: Point) {
    showTouchIndicators(phase, [coords]);
  }

  function showTouchIndicators(phase: TouchPhase, coords: Point[]) {
    if (!touchOverlayVisible) {
      return;
    }

    setTouchIndicators(
      coords.map((coord, index) => ({
        id: index + 1,
        phase,
        x: coord.x,
        y: coord.y,
      })),
    );
    if (touchIndicatorTimeoutRef.current) {
      clearTimeout(touchIndicatorTimeoutRef.current);
      touchIndicatorTimeoutRef.current = 0;
    }
    if (phase === "ended" || phase === "cancelled") {
      touchIndicatorTimeoutRef.current = window.setTimeout(() => {
        setTouchIndicators([]);
        touchIndicatorTimeoutRef.current = 0;
      }, 240);
    }
  }

  useEffect(() => {
    if (!outerCanvasElement) {
      return;
    }
    const canvasElement = outerCanvasElement;

    type WebKitGestureEvent = Event & {
      clientX?: number;
      clientY?: number;
      scale?: number;
    };

    function handleGestureStart(event: Event) {
      event.preventDefault();
      gestureStartZoomRef.current = effectiveZoomRef.current;
    }

    function handleGestureChange(event: Event) {
      event.preventDefault();
      const gestureEvent = event as WebKitGestureEvent;
      const bounds = canvasElement.getBoundingClientRect();
      applyZoomAtClientPointRef.current(
        gestureStartZoomRef.current * (gestureEvent.scale ?? 1),
        gestureEvent.clientX ?? bounds.left + bounds.width / 2,
        gestureEvent.clientY ?? bounds.top + bounds.height / 2,
      );
    }

    canvasElement.addEventListener("gesturestart", handleGestureStart, {
      passive: false,
    });
    canvasElement.addEventListener("gesturechange", handleGestureChange, {
      passive: false,
    });
    return () => {
      canvasElement.removeEventListener("gesturestart", handleGestureStart);
      canvasElement.removeEventListener("gesturechange", handleGestureChange);
    };
  }, [outerCanvasElement]);

  function promptForURL() {
    if (!selectedSimulator) {
      return;
    }
    const nextValue = window.prompt(
      `Open URL on ${selectedSimulator.name}`,
      openURLValueRef.current,
    );
    if (nextValue == null) {
      return;
    }
    const trimmed = nextValue.trim();
    if (!trimmed) {
      return;
    }
    openURLValueRef.current = trimmed;
    writePersistedUiState((current) => ({
      ...current,
      openURLValue: trimmed,
    }));
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
      bundleIDValueRef.current,
    );
    if (nextValue == null) {
      return;
    }
    const trimmed = nextValue.trim();
    if (!trimmed) {
      return;
    }
    bundleIDValueRef.current = trimmed;
    writePersistedUiState((current) => ({
      ...current,
      bundleIDValue: trimmed,
    }));
    setMenuOpen(false);
    void runAction(() =>
      launchSimulatorBundle(selectedSimulator.udid, { bundleId: trimmed }),
    );
  }

  function sendHardwareButtonEvent(
    button: string,
    phase: "down" | "up",
    usagePage?: number,
    usage?: number,
  ) {
    if (!selectedSimulator) {
      return;
    }
    if (phase === "down") {
      setAccessibilitySelectedId("");
      setAccessibilityHoveredId(null);
    }
    if (
      !sendControl(selectedSimulator.udid, {
        type: "button",
        button,
        phase,
        usagePage,
        usage,
      })
    ) {
      void runAction(
        () =>
          pressSimulatorButton(selectedSimulator.udid, {
            button,
            phase,
            usagePage,
            usage,
          }),
        false,
      );
    }
  }

  function sendCrownRotation(delta: number) {
    if (!selectedSimulator || !Number.isFinite(delta) || delta === 0) {
      return;
    }
    setAccessibilitySelectedId("");
    setAccessibilityHoveredId(null);
    if (
      !sendControl(selectedSimulator.udid, {
        type: "crown",
        delta,
      })
    ) {
      void runAction(
        () => rotateDigitalCrown(selectedSimulator.udid, { delta }),
        false,
      );
    }
  }

  function prepareSimulatorInput() {
    setMenuOpen(false);
    setAccessibilitySelectedId("");
    setAccessibilityHoveredId(null);
    window.getSelection()?.removeAllRanges();
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement) {
      activeElement.blur();
    }
  }

  function handleSimulatorCreated(response: {
    simulator: SimulatorMetadata;
    pairedWatchSimulator?: SimulatorMetadata | null;
  }) {
    updateSimulator(response.simulator);
    if (response.pairedWatchSimulator) {
      updateSimulator(response.pairedWatchSimulator);
    }
    setSelectedUDID(response.simulator.udid);
    setNewSimulatorOpen(false);
    setLocalError("");
    void refresh();
  }

  function openInstallAppPicker() {
    if (!selectedSimulator) {
      setLocalError("Select a simulator before installing an app.");
      return;
    }
    if (!selectedSimulator.isBooted) {
      setLocalError("Boot the selected simulator before installing an app.");
      return;
    }
    appInstallInputRef.current?.click();
  }

  function handleInstallInputChange(
    event: React.ChangeEvent<HTMLInputElement>,
  ) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) {
      return;
    }
    void installAppFile(file, selectedSimulator);
  }

  function handleAppInstallDragEnter(event: React.DragEvent<HTMLElement>) {
    if (!dragEventHasFiles(event.dataTransfer)) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    appInstallDragDepthRef.current += 1;
    if (appInstallState?.phase !== "installing") {
      setAppInstallState({ phase: "dragging" });
    }
  }

  function handleAppInstallDragOver(event: React.DragEvent<HTMLElement>) {
    if (!dragEventHasFiles(event.dataTransfer)) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = canInstallApp ? "copy" : "none";
  }

  function handleAppInstallDragLeave(event: React.DragEvent<HTMLElement>) {
    if (!dragEventHasFiles(event.dataTransfer)) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    appInstallDragDepthRef.current = Math.max(
      0,
      appInstallDragDepthRef.current - 1,
    );
    if (appInstallDragDepthRef.current === 0) {
      setAppInstallState((current) =>
        current?.phase === "dragging" ? null : current,
      );
    }
  }

  function handleAppInstallDrop(event: React.DragEvent<HTMLElement>) {
    if (!dragEventHasFiles(event.dataTransfer)) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    appInstallDragDepthRef.current = 0;
    const files = Array.from(event.dataTransfer.files);
    if (files.length !== 1) {
      setAppInstallState(null);
      setLocalError("Drop one `.ipa` or `.apk` file at a time.");
      return;
    }
    void installAppFile(files[0], selectedSimulator);
  }

  async function installAppFile(
    file: File,
    simulator: SimulatorMetadata | null,
  ) {
    if (appInstallState?.phase === "installing") {
      setLocalError("Wait for the current app install to finish.");
      return;
    }
    if (!simulator) {
      setAppInstallState(null);
      setLocalError("Select a simulator before installing an app.");
      return;
    }
    if (!simulator.isBooted) {
      setAppInstallState(null);
      setLocalError("Boot the selected simulator before installing an app.");
      return;
    }

    const appKind = installableAppKind(file.name);
    const simulatorIsAndroid = isAndroidSimulator(simulator);
    if (!appKind) {
      setAppInstallState(null);
      setLocalError(
        simulatorIsAndroid
          ? "Drop an `.apk` file for Android emulators."
          : "Drop an `.ipa` file for iOS simulators.",
      );
      return;
    }
    if (simulatorIsAndroid && appKind !== "apk") {
      setAppInstallState(null);
      setLocalError("Android emulators can only install `.apk` files.");
      return;
    }
    if (!simulatorIsAndroid && appKind !== "ipa") {
      setAppInstallState(null);
      setLocalError("iOS simulators can only install `.ipa` files.");
      return;
    }

    const fileName = file.name || "app";
    setLocalError("");
    setAppInstallState({ fileName, phase: "installing" });
    try {
      await uploadSimulatorApp(simulator.udid, file);
      setAppInstallState({ fileName, phase: "installed" });
      clearAppInstallStatusLater(fileName);
      await refresh();
    } catch (error) {
      setAppInstallState(null);
      setLocalError(error instanceof Error ? error.message : "Install failed.");
    }
  }

  function clearAppInstallStatusLater(fileName: string) {
    if (appInstallStatusTimeoutRef.current) {
      window.clearTimeout(appInstallStatusTimeoutRef.current);
    }
    appInstallStatusTimeoutRef.current = window.setTimeout(() => {
      appInstallStatusTimeoutRef.current = 0;
      setAppInstallState((current) =>
        current?.phase === "installed" && current.fileName === fileName
          ? null
          : current,
      );
    }, 1800);
  }

  async function submitPairing(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const code = pairingCode.trim();
    if (!code) {
      setPairingError("Enter the pairing code from the SimDeck terminal.");
      return;
    }
    setPairingBusy(true);
    setPairingError("");
    try {
      await pairBrowser(code);
      setPairingCode("");
      await refresh();
    } catch (error) {
      setPairingError(
        error instanceof ApiError && error.status === 401
          ? "Pairing code did not match."
          : error instanceof Error
            ? error.message
            : "Pairing failed.",
      );
    } finally {
      setPairingBusy(false);
    }
  }

  return (
    <div className="app">
      {pairingRequired ? (
        <div className="pairing-gate" role="dialog" aria-modal="true">
          <form className="pairing-panel" onSubmit={submitPairing}>
            <h2>Pair SimDeck</h2>
            <p>Enter the pairing code shown in the SimDeck terminal.</p>
            <input
              autoComplete="one-time-code"
              autoFocus
              inputMode="numeric"
              onChange={(event) => setPairingCode(event.target.value)}
              placeholder="000 000"
              value={pairingCode}
            />
            {pairingError ? <span>{pairingError}</span> : null}
            <button
              className="tbtn accent"
              disabled={pairingBusy}
              type="submit"
            >
              Pair
            </button>
          </form>
        </div>
      ) : null}
      <input
        accept={appInstallAccept}
        aria-hidden="true"
        onChange={handleInstallInputChange}
        ref={appInstallInputRef}
        style={{ display: "none" }}
        tabIndex={-1}
        type="file"
      />
      <Toolbar
        canInstallApp={canInstallApp}
        closeMenu={() => setMenuOpen(false)}
        debugVisible={debugVisible}
        error={toolbarError}
        filteredSimulators={filteredSimulators}
        hierarchyVisible={hierarchyVisible}
        hideSimulatorSelection={hideSimulatorSelection}
        isLoading={isLoading}
        menuOpen={menuOpen}
        menuRef={menuRef}
        onBoot={() => {
          if (!selectedSimulator) {
            return;
          }
          const udid = selectedSimulator.udid;
          void runSimulatorLifecycleAction("boot", udid, () =>
            bootSimulator(udid),
          );
        }}
        onCaptureScreenshot={() => {
          void downloadSimulatorScreenshot(false);
        }}
        onCaptureScreenshotWithBezel={() => {
          void downloadSimulatorScreenshot(true);
        }}
        onChangeSearch={setSearch}
        onDismissKeyboard={() => {
          if (!selectedSimulator) {
            return;
          }
          if (
            !sendControl(selectedSimulator.udid, { type: "dismissKeyboard" })
          ) {
            void runAction(
              () => dismissKeyboard(selectedSimulator.udid),
              false,
            );
          }
        }}
        onHome={() => {
          if (!selectedSimulator) {
            return;
          }
          setAccessibilitySelectedId("");
          setAccessibilityHoveredId(null);
          if (!sendControl(selectedSimulator.udid, { type: "home" })) {
            void runAction(() => pressHome(selectedSimulator.udid), false);
          }
        }}
        onInstallAppPrompt={openInstallAppPicker}
        onOpenAppSwitcher={() => {
          if (!selectedSimulator) {
            return;
          }
          setAccessibilitySelectedId("");
          setAccessibilityHoveredId(null);
          if (!sendControl(selectedSimulator.udid, { type: "appSwitcher" })) {
            void runAction(
              () => openAppSwitcher(selectedSimulator.udid),
              false,
            );
          }
        }}
        onOpenBundlePrompt={promptForBundleID}
        onOpenNewSimulator={() => {
          setMenuOpen(false);
          setNewSimulatorOpen(true);
        }}
        onOpenUrlPrompt={promptForURL}
        onRotateRight={() => {
          if (!selectedSimulator) {
            return;
          }
          if (selectedHasFixedOrientation) {
            return;
          }
          const androidViewport = isAndroidSimulator(selectedSimulator);
          beginZoomAnimation();
          if (androidViewport) {
            void runAction(async () => {
              await rotateRight(selectedSimulator.udid);
              setRotationQuarterTurns(0);
              beginZoomAnimation();
              await refresh();
            }, false);
            return;
          }
          if (sendControl(selectedSimulator.udid, { type: "rotateRight" })) {
            setRotationQuarterTurns((current) => (current + 1) % 4);
            return;
          }
          void runAction(async () => {
            await rotateRight(selectedSimulator.udid);
            setRotationQuarterTurns((current) => (current + 1) % 4);
          }, false);
        }}
        onRecordScreen={() => {
          void downloadSimulatorRecording();
        }}
        onStreamEncoderChange={updateStreamEncoder}
        onStreamFpsChange={updateStreamFps}
        onStreamQualityChange={updateStreamQuality}
        onStreamTransportChange={updateStreamTransport}
        onShutdown={() => {
          if (!selectedSimulator) {
            return;
          }
          const udid = selectedSimulator.udid;
          void runSimulatorLifecycleAction("shutdown", udid, () =>
            shutdownSimulator(udid),
          );
        }}
        onToggleAppearance={() => {
          if (!selectedSimulator) {
            return;
          }
          if (
            !sendControl(selectedSimulator.udid, { type: "toggleAppearance" })
          ) {
            void runAction(
              () => toggleAppearance(selectedSimulator.udid),
              false,
            );
          }
        }}
        onToggleDebug={() => setDebugVisible((current) => !current)}
        onToggleDevTools={toggleDevTools}
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
        onToggleTouchOverlay={() =>
          setTouchOverlayVisible((current) => !current)
        }
        remoteStream={remoteStream}
        search={search}
        selectedSimulator={selectedSimulator}
        selectedSimulatorIdentifier={selectedSimulatorDetail}
        setSelectedUDID={setSelectedUDID}
        showBootButton={Boolean(
          selectedSimulator &&
          !selectedSimulator.isBooted &&
          !selectedSimulatorTransitionKind,
        )}
        streamConfig={streamConfig}
        streamTransport={streamTransport}
        showStopButton={Boolean(
          selectedSimulator?.isBooted && !selectedSimulatorTransitionKind,
        )}
        touchOverlayVisible={touchOverlayVisible}
        devToolsVisible={devToolsVisible}
      />
      <NewSimulatorModal
        onClose={() => setNewSimulatorOpen(false)}
        onCreated={handleSimulatorCreated}
        open={newSimulatorOpen && !hideSimulatorSelection}
        selectedSimulator={selectedSimulator}
      />
      <SimulatorViewport
        accessibilityHoveredId={accessibilityHoveredId}
        appInstallOverlayLabel={appInstallOverlayLabel}
        accessibilityPanel={
          <AccessibilityInspector
            availableSources={accessibilityAvailableSources}
            disconnected={providerDisconnected}
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
            onSourceChange={changeAccessibilitySource}
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
        chromeLoaded={chromeLoaded}
        chromeProfile={viewportChromeProfile}
        chromeRequired={chromeRequired}
        chromeScreenStyle={viewportScreenStyle}
        chromeUrl={chromeUrl}
        chromeButtonUrl={chromeButtonUrl}
        debugPanel={
          debugVisible ? (
            <DebugPanel
              encoder={selectedSimulator.privateDisplay?.encoder}
              fps={fps}
              inline
              onClose={() => setDebugVisible(false)}
              runtimeInfo={runtimeInfo}
              stats={stats}
              status={streamStatus}
            />
          ) : null
        }
        deviceFrameStyle={deviceFrameStyle}
        devicePresentationStyle={devicePresentationStyle}
        deviceTransform={deviceTransform}
        effectiveZoom={effectiveZoom}
        fitScale={fitScale}
        hasFrame={hasFrame}
        isLoading={isLoading}
        isStreamError={viewportHasStreamError}
        isPanning={pointerInput.isPanning}
        isAppInstallDragging={appInstallState?.phase === "dragging"}
        isAppInstalling={appInstallState?.phase === "installing"}
        onAppInstallDragEnter={handleAppInstallDragEnter}
        onAppInstallDragLeave={handleAppInstallDragLeave}
        onAppInstallDragOver={handleAppInstallDragOver}
        onAppInstallDrop={handleAppInstallDrop}
        onChromeButtonEvent={sendHardwareButtonEvent}
        onPanPointerMove={pointerInput.handlePanPointerMove}
        onPanPointerUp={pointerInput.handlePanPointerUp}
        onPickerHover={setAccessibilityHoveredId}
        onPickerSelect={(id) => {
          setAccessibilitySelectedId(id);
          setAccessibilityHoveredId(null);
          setAccessibilityPickerActive(false);
        }}
        onSimulatorInteraction={prepareSimulatorInput}
        onScreenPointerCancel={pointerInput.handleScreenPointerCancel}
        onScreenPointerDown={pointerInput.handleScreenPointerDown}
        onScreenPointerMove={pointerInput.handleScreenPointerMove}
        onScreenPointerUp={pointerInput.handleScreenPointerUp}
        onStartPanning={pointerInput.startPanning}
        onViewportWheel={handleViewportWheel}
        onZoomActual={() =>
          applyZoom(1, { x: 0, y: -zoomDockReservedHeight / 2 })
        }
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
        screenClassName={isAndroidViewport ? "android-screen" : undefined}
        selectedSimulator={selectedSimulator}
        shellStyle={shellStyle}
        streamCanvasRef={handleStreamCanvasRef}
        streamBackend={streamBackend}
        streamCanvasKey={streamCanvasKey}
        streamStatusLabel={streamStatusLabel}
        statusOverlayLabel={viewportStatusOverlayLabel}
        touchIndicators={touchIndicators}
        touchOverlayVisible={touchOverlayVisible}
        viewMode={viewMode}
        devtoolsPanel={
          <DevToolsPanel
            disconnected={providerDisconnected}
            onClose={() => setDevToolsVisible(false)}
            selectedSimulator={selectedSimulator}
            visible={devToolsVisible}
          />
        }
        zoomDockRef={handleZoomDockRef}
        zoomAnimating={zoomAnimating}
      />
    </div>
  );
}

function androidScreenRadiusStyle(
  chromeProfile: ChromeProfile,
  displaySize: Size | null,
): CSSProperties | null {
  const screenWidth =
    displaySize && displaySize.width > 0
      ? displaySize.width
      : chromeProfile.screenWidth;
  if (screenWidth <= 0) {
    return null;
  }

  const scale = DEVICE_SCREEN_WIDTH / screenWidth;
  const maxRadius = DEVICE_SCREEN_WIDTH / 2;
  const radii = chromeProfile.cornerRadii;
  const topLeft = scaledScreenRadius(
    radii?.topLeft ?? chromeProfile.cornerRadius,
    scale,
    maxRadius,
  );
  const topRight = scaledScreenRadius(
    radii?.topRight ?? chromeProfile.cornerRadius,
    scale,
    maxRadius,
  );
  const bottomRight = scaledScreenRadius(
    radii?.bottomRight ?? chromeProfile.cornerRadius,
    scale,
    maxRadius,
  );
  const bottomLeft = scaledScreenRadius(
    radii?.bottomLeft ?? chromeProfile.cornerRadius,
    scale,
    maxRadius,
  );

  if (topLeft <= 0 && topRight <= 0 && bottomRight <= 0 && bottomLeft <= 0) {
    return null;
  }

  const borderRadius = `${topLeft}px ${topRight}px ${bottomRight}px ${bottomLeft}px`;
  return {
    borderRadius,
    borderTopLeftRadius: `${topLeft}px`,
    borderTopRightRadius: `${topRight}px`,
    borderBottomRightRadius: `${bottomRight}px`,
    borderBottomLeftRadius: `${bottomLeft}px`,
    clipPath: `inset(0 round ${borderRadius})`,
  };
}

function scaledScreenRadius(radius: number, scale: number, maxRadius: number) {
  if (!Number.isFinite(radius) || radius <= 0) {
    return 0;
  }
  return Math.min(radius * scale, maxRadius);
}

function androidDisplayKeyForSimulator(simulator: SimulatorMetadata): string {
  const display = simulator.privateDisplay;
  if (!display) {
    return simulator.udid;
  }
  return [
    simulator.udid,
    Math.round(display.displayWidth),
    Math.round(display.displayHeight),
    display.rotationQuarterTurns ?? 0,
  ].join("|");
}

function readDeviceQueryParam(): string | undefined {
  if (typeof window === "undefined") {
    return undefined;
  }

  const value = new URLSearchParams(window.location.search).get("device");
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

function isStreamAttachFailure(message: string): boolean {
  const normalized = message.toLowerCase();
  return (
    normalized.includes("headless screen") ||
    normalized.includes("screen adapter") ||
    normalized.includes("coresimulator did not provide") ||
    normalized.includes("did not expose any live screens")
  );
}

function friendlyClientError(message: string): string {
  const normalized = message.trim().toLowerCase();
  if (normalized === "failed to fetch" || normalized === "load failed") {
    return "SimDeck server is unreachable. Reconnecting in the background.";
  }
  return message;
}

function friendlyStreamError(
  message: string | undefined,
  options: { remote: boolean },
): string {
  const normalized = message?.trim() ?? "";
  if (!normalized) {
    return "";
  }
  if (
    options.remote &&
    normalized.toLowerCase().includes(AUTH_REQUIRED_MESSAGE.toLowerCase())
  ) {
    return "";
  }
  return friendlyClientError(normalized);
}

function isStreamProviderDisconnectError(message: string | undefined): boolean {
  const lower = message?.trim().toLowerCase() ?? "";
  return (
    lower.includes("websocket stream closed") ||
    lower.includes("websocket stream failed")
  );
}

function streamAgentStatusLabel({
  hasFrame,
  simulator,
  simulatorState,
  stats,
  streamStatusDetail,
  streamStatusState,
}: {
  hasFrame: boolean;
  simulator: SimulatorMetadata | null;
  simulatorState: SimulatorStateResponse | null;
  stats: {
    frameSequence: number;
    latestFrameGapMs: number;
    renderedFrames: number;
  };
  streamStatusDetail?: string;
  streamStatusState: string;
}): string {
  if (!simulator) {
    return "No simulator selected";
  }

  const display = simulator.privateDisplay;
  const serverFrameSequence =
    simulatorState?.frameSequence ?? display?.frameSequence ?? 0;
  const serverFrameAgeMs =
    simulatorState?.lastFrameAgeMs ??
    (display?.lastFrameAt
      ? Math.max(0, Date.now() - display.lastFrameAt)
      : null);
  const foreground =
    simulatorState?.foregroundApp?.appName ??
    simulatorState?.foregroundApp?.bundleIdentifier ??
    "unknown foreground app";
  const parts = [
    simulator.isBooted ? "Booted" : "Shutdown",
    display?.displayReady || simulatorState?.displayReady
      ? "display ready"
      : "display not ready",
    `server ${display?.displayStatus ?? simulatorState?.displayStatus ?? "Unknown"}`,
    `server frame ${serverFrameSequence}`,
    serverFrameAgeMs == null
      ? "server frame age unknown"
      : `server frame ${formatMilliseconds(serverFrameAgeMs)} ago`,
    hasFrame
      ? stats.frameSequence > 0
        ? `browser frame ${stats.frameSequence}`
        : `browser rendered ${stats.renderedFrames}`
      : "browser frame pending",
    `browser gap ${formatMilliseconds(stats.latestFrameGapMs)}`,
    `foreground ${foreground}`,
    `client ${streamStatusState}`,
  ];
  if (streamStatusDetail) {
    parts.push(streamStatusDetail);
  }
  return parts.join(" · ");
}

function formatMilliseconds(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return "0ms";
  }
  if (value < 1000) {
    return `${Math.round(value)}ms`;
  }
  return `${(value / 1000).toFixed(1)}s`;
}

function userFacingAccessibilityError(message: string): string {
  const normalized = message.trim();
  if (!normalized) {
    return "";
  }

  const lower = normalized.toLowerCase();
  if (isProviderDisconnected(normalized)) {
    return NOT_CONNECTED_MESSAGE;
  }
  if (
    lower.includes("no app inspector found") ||
    lower.includes("no connected websocket inspector found") ||
    lower.includes("no published app inspector found") ||
    lower.includes("no in-app inspector found") ||
    lower.includes("first probe error:")
  ) {
    return "";
  }

  return normalized;
}

function isProviderDisconnected(message: string): boolean {
  const lower = message.trim().toLowerCase();
  if (!lower || lower === AUTH_REQUIRED_MESSAGE.toLowerCase()) {
    return false;
  }
  return (
    lower.includes("failed to fetch") ||
    lower.includes("load failed") ||
    lower.includes("networkerror") ||
    lower.includes("network error") ||
    lower.includes("timed out waiting for provider")
  );
}

function mergeStreamQualityResponse(
  current: StreamConfig,
  response: StreamQualityResponse,
  options: { preserveAutoQuality?: boolean } = {},
): StreamConfig {
  const quality = response.quality ?? {};
  const next: StreamConfig = {
    ...current,
    encoder: normalizeStreamEncoder(
      quality.videoCodec ?? response.videoCodec,
      current.encoder,
    ),
    fps: normalizeStreamFps(quality.fps, current.fps),
    maxEdge: normalizeMaxEdge(quality.maxEdge, current.maxEdge),
    quality:
      options.preserveAutoQuality && current.quality === "auto"
        ? "auto"
        : normalizeStreamQuality(quality.profile, current.quality),
  };
  return streamConfigsEqual(current, next) ? current : next;
}

function normalizeStreamEncoder(
  value: string | undefined,
  fallback: StreamEncoder,
): StreamEncoder {
  const normalized = value?.trim().toLowerCase() as StreamEncoder | undefined;
  return normalized && STREAM_ENCODER_VALUES.has(normalized)
    ? normalized
    : fallback;
}

function normalizeStreamQuality(
  value: string | undefined,
  fallback: StreamQualityPreset,
): StreamQualityPreset {
  const normalized = value?.trim().toLowerCase();
  if (normalized === "auto") {
    return "auto";
  }
  if (normalized === "full") {
    return "full";
  }
  if (normalized === "quality") {
    return "quality";
  }
  if (normalized === "smooth") {
    return "smooth";
  }
  if (normalized === "balanced" || normalized === "fast") {
    return "balanced";
  }
  if (normalized === "economy" || normalized === "ci-software") {
    return "economy";
  }
  if (normalized === "low") {
    return "low";
  }
  if (normalized === "tiny") {
    return "tiny";
  }
  return fallback;
}

function normalizeStreamFps(
  value: number | undefined,
  fallback: StreamFps,
): StreamFps {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? Math.round(value)
    : fallback;
}

function normalizeMaxEdge(
  value: number | undefined,
  fallback: number | undefined,
): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? Math.round(value)
    : fallback;
}

function isAndroidSimulator(simulator: SimulatorMetadata | null): boolean {
  return Boolean(
    simulator?.platform === "android-emulator" ||
    simulator?.deviceTypeIdentifier === "android-emulator" ||
    simulator?.udid.startsWith("android:"),
  );
}

function dragEventHasFiles(dataTransfer: DataTransfer): boolean {
  return Array.from(dataTransfer.types).includes("Files");
}

function installableAppKind(fileName: string): "apk" | "ipa" | null {
  const lower = fileName.trim().toLowerCase();
  if (lower.endsWith(".apk")) {
    return "apk";
  }
  if (lower.endsWith(".ipa")) {
    return "ipa";
  }
  return null;
}

function appInstallStatusLabel(
  state: AppInstallState,
  simulator: SimulatorMetadata | null,
  android: boolean,
): string {
  const expected = android ? "APK" : "IPA";
  if (state.phase === "dragging") {
    if (!simulator) {
      return "Select a simulator before installing";
    }
    if (!simulator.isBooted) {
      return "Boot selected simulator before installing";
    }
    return `Drop ${expected} to install on ${simulator.name}`;
  }
  if (state.phase === "installing") {
    return `Installing ${state.fileName ?? expected}...`;
  }
  return `Installed ${state.fileName ?? expected}`;
}

function streamConfigsEqual(left: StreamConfig, right: StreamConfig): boolean {
  return (
    left.encoder === right.encoder &&
    left.fps === right.fps &&
    left.maxEdge === right.maxEdge &&
    left.quality === right.quality
  );
}
