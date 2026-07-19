import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";

import {
  fetchCameraStatus,
  startCameraSimulation,
  stopCameraSimulation,
  switchCameraSimulationSource,
} from "../../api/simulators";
import type {
  CameraSourceKind,
  CameraStatusResponse,
  SimulatorMetadata,
} from "../../api/types";
import {
  cameraVideoConstraints,
  videoDevices,
  startCameraFeed,
  type CameraDevice,
  type CameraFeed,
  type CameraStats,
} from "./cameraTransport";
import {
  createCameraBenchmarkSource,
  type CameraBenchmarkSource,
} from "./cameraBenchmark";

interface CameraSimulationModalProps {
  onClose: () => void;
  open: boolean;
  selectedSimulator: SimulatorMetadata | null;
}

type SourceMode = "placeholder" | "camera" | "media";
type MirrorMode = "auto" | "on" | "off";
type CameraAccess = "idle" | "requesting" | "granted" | "denied";
const CAMERA_DEVICE_STORAGE_KEY = "simdeck.camera.deviceId";

declare global {
  interface Window {
    __simdeckCameraBenchmark?: {
      snapshot(): {
        camera: ReturnType<CameraBenchmarkSource["snapshot"]> | null;
        ready: boolean;
        transport: CameraStats | null;
        udid: string;
      };
      start(): Promise<CameraStatusResponse>;
      stop(): Promise<CameraStatusResponse>;
    };
  }
}

export function CameraSimulationModal({
  onClose,
  open,
  selectedSimulator,
}: CameraSimulationModalProps) {
  const [sourceMode, setSourceMode] = useState<SourceMode>("placeholder");
  const [mediaPath, setMediaPath] = useState("");
  const [cameraId, setCameraId] = useState(storedCameraId);
  const [cameras, setCameras] = useState<CameraDevice[]>([]);
  const [cameraAccess, setCameraAccess] = useState<CameraAccess>("idle");
  const [cameraStats, setCameraStats] = useState<CameraStats | null>(null);
  const [mirror, setMirror] = useState<MirrorMode>("auto");
  const [status, setStatus] = useState<CameraStatusResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isApplying, setIsApplying] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [error, setError] = useState("");
  const cameraFeedRef = useRef<CameraFeed | null>(null);
  const cameraStreamRef = useRef<MediaStream | null>(null);
  const cameraStatsRef = useRef<CameraStats | null>(null);
  const benchmarkSourceRef = useRef<CameraBenchmarkSource | null>(null);

  const udid = selectedSimulator?.udid ?? "";
  const benchmarkMode = cameraBenchmarkEnabled();
  const canApply = Boolean(
    selectedSimulator?.isBooted &&
    (sourceMode !== "media" || mediaPath.trim()) &&
    (sourceMode !== "camera" || cameraAccess === "granted"),
  );

  useEffect(() => {
    if (!open) {
      return;
    }
    setError("");
    setIsApplying(false);
    setIsStopping(false);
    setCameraAccess(
      benchmarkMode
        ? "granted"
        : cameraStreamRef.current
              ?.getVideoTracks()
              .some((track) => track.readyState === "live")
          ? "granted"
          : "idle",
    );
    void refreshStatus();
  }, [benchmarkMode, open, udid]);

  useEffect(() => {
    cameraStatsRef.current = cameraStats;
  }, [cameraStats]);

  useEffect(() => {
    if (!benchmarkMode) {
      return;
    }
    const benchmark = {
      snapshot: () => ({
        camera: benchmarkSourceRef.current?.snapshot() ?? null,
        ready: Boolean(selectedSimulator?.isBooted && udid),
        transport: cameraStatsRef.current,
        udid,
      }),
      start: async () => {
        if (!selectedSimulator?.isBooted || !udid) {
          throw new Error(
            "Boot and select a simulator before benchmarking the camera.",
          );
        }
        const stream = await cameraStream();
        const nextStatus = await startCameraSimulation(udid, {
          mirror: "off",
          source: { kind: "camera" },
        });
        setStatus(nextStatus);
        setSourceMode("camera");
        await startCurrentCameraFeed(stream);
        return nextStatus;
      },
      stop: async () => {
        stopCameraFeed(true);
        const nextStatus = await stopCameraSimulation(udid);
        setStatus(nextStatus);
        return nextStatus;
      },
    };
    window.__simdeckCameraBenchmark = benchmark;
    return () => {
      if (window.__simdeckCameraBenchmark === benchmark) {
        delete window.__simdeckCameraBenchmark;
      }
    };
  }, [benchmarkMode, selectedSimulator?.isBooted, udid]);

  useEffect(() => {
    if (!selectedSimulator?.isBooted || !udid) {
      return;
    }
    let cancelled = false;
    const resume = async () => {
      try {
        const nextStatus = await fetchCameraStatus(udid);
        if (cancelled) {
          return;
        }
        setStatus(nextStatus);
        if (!nextStatus.alive || nextStatus.source !== "camera") {
          return;
        }
        setSourceMode("camera");
        if (
          cameraFeedRef.current ||
          !(await cameraPermissionWasGranted()) ||
          cancelled
        ) {
          return;
        }
        setCameraAccess("granted");
        const stream = await cameraStream();
        if (cancelled) {
          for (const track of stream.getTracks()) {
            track.stop();
          }
          return;
        }
        await startCurrentCameraFeed(stream);
      } catch (resumeError) {
        if (!cancelled) {
          setError(cameraAccessError(resumeError));
        }
      }
    };
    void resume();
    return () => {
      cancelled = true;
    };
  }, [selectedSimulator?.isBooted, udid]);

  useEffect(() => {
    if (!open || sourceMode !== "camera" || !navigator.mediaDevices) {
      return;
    }
    const refreshCameras = () => {
      void navigator.mediaDevices
        .enumerateDevices()
        .then((devices) => setCameras(videoDevices(devices)))
        .catch(() => setCameras([]));
    };
    refreshCameras();
    navigator.mediaDevices.addEventListener?.("devicechange", refreshCameras);
    return () =>
      navigator.mediaDevices.removeEventListener?.(
        "devicechange",
        refreshCameras,
      );
  }, [open, sourceMode]);

  useEffect(() => {
    if (!open || sourceMode !== "camera") {
      return;
    }
    const activeTrack = cameraStreamRef.current
      ?.getVideoTracks()
      .find((track) => track.readyState === "live");
    if (activeTrack) {
      setCameraAccess("granted");
      return;
    }
    let cancelled = false;
    let permission: PermissionStatus | null = null;
    const updatePermission = async () => {
      if (permission && !cancelled) {
        setCameraAccess(
          permission.state === "granted"
            ? "granted"
            : permission.state === "denied"
              ? "denied"
              : "idle",
        );
      }
      if (permission?.state === "granted" && navigator.mediaDevices) {
        const devices = videoDevices(
          await navigator.mediaDevices.enumerateDevices(),
        );
        if (!cancelled) {
          setCameras(devices);
        }
      }
    };
    const inspectPermission = async () => {
      try {
        permission = await navigator.permissions?.query({
          name: "camera" as PermissionName,
        });
      } catch {
        permission = null;
      }
      if (permission) {
        permission.addEventListener?.("change", updatePermission);
        await updatePermission();
        return;
      }
      try {
        const mediaDevices = await navigator.mediaDevices.enumerateDevices();
        const devices = videoDevices(mediaDevices);
        if (!cancelled) {
          setCameras(devices);
          setCameraAccess(
            mediaDevices.some(
              (device) =>
                device.kind === "videoinput" && device.label.trim().length > 0,
            )
              ? "granted"
              : "idle",
          );
        }
      } catch {
        if (!cancelled) {
          setCameraAccess("idle");
        }
      }
    };
    void inspectPermission();
    return () => {
      cancelled = true;
      permission?.removeEventListener?.("change", updatePermission);
    };
  }, [open, sourceMode]);

  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        close();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  const stopCameraFeed = useCallback((stopTracks: boolean) => {
    cameraFeedRef.current?.stop();
    cameraFeedRef.current = null;
    setCameraStats(null);
    if (stopTracks) {
      benchmarkSourceRef.current?.stop();
      benchmarkSourceRef.current = null;
      for (const track of cameraStreamRef.current?.getTracks() ?? []) {
        track.stop();
      }
      cameraStreamRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => stopCameraFeed(true);
  }, [stopCameraFeed, udid]);

  const statusText = useMemo(() => {
    if (!status?.alive) {
      return "not running";
    }
    return status.sourceLabel ? `running · ${status.sourceLabel}` : "running";
  }, [status]);
  const updatesRunningCamera = Boolean(status?.alive);

  if (!open) {
    return null;
  }

  async function refreshStatus() {
    if (!udid) {
      setStatus(null);
      return;
    }
    setIsLoading(true);
    try {
      const nextStatus = await fetchCameraStatus(udid);
      setStatus(nextStatus);
      if (
        nextStatus.mirror === "auto" ||
        nextStatus.mirror === "on" ||
        nextStatus.mirror === "off"
      ) {
        setMirror(nextStatus.mirror);
      }
      if (
        nextStatus.source === "camera" ||
        nextStatus.source === "placeholder"
      ) {
        setSourceMode(nextStatus.source);
      } else if (
        nextStatus.source === "image" ||
        nextStatus.source === "video"
      ) {
        setSourceMode("media");
      }
      if (nextStatus.arg) {
        if (nextStatus.source === "image" || nextStatus.source === "video") {
          setMediaPath(nextStatus.arg);
        }
      }
    } catch (statusError) {
      setStatus(null);
      setError(
        statusError instanceof Error
          ? statusError.message
          : "Unable to load camera status.",
      );
    } finally {
      setIsLoading(false);
    }
  }

  function requestSource(): { kind: CameraSourceKind; arg?: string } {
    if (sourceMode === "camera") {
      return { kind: "camera" };
    }
    if (sourceMode === "media") {
      const value = mediaPath.trim();
      const kind: CameraSourceKind = looksLikeVideo(value) ? "video" : "image";
      return { kind, arg: value };
    }
    return { kind: "placeholder" };
  }

  async function cameraStream(): Promise<MediaStream> {
    const current = cameraStreamRef.current;
    const currentTrack = current
      ?.getVideoTracks()
      .find((track) => track.readyState === "live");
    if (
      current &&
      currentTrack &&
      (!cameraId || currentTrack.getSettings().deviceId === cameraId)
    ) {
      return current;
    }
    for (const track of current?.getTracks() ?? []) {
      track.stop();
    }
    cameraStreamRef.current = null;
    if (benchmarkMode) {
      benchmarkSourceRef.current?.stop();
      const source = createCameraBenchmarkSource(cameraBenchmarkOptions());
      benchmarkSourceRef.current = source;
      cameraStreamRef.current = source.stream;
      setCameraAccess("granted");
      return source.stream;
    }
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error("Camera capture is unavailable in this browser.");
    }
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: cameraVideoConstraints(cameraId),
    });
    cameraStreamRef.current = stream;
    const selectedId = stream.getVideoTracks()[0]?.getSettings().deviceId ?? "";
    if (selectedId) {
      setCameraId(selectedId);
      storeCameraId(selectedId);
    }
    const devices = videoDevices(
      await navigator.mediaDevices.enumerateDevices(),
    );
    setCameras(devices);
    return stream;
  }

  async function requestCameraAccess() {
    setCameraAccess("requesting");
    setError("");
    try {
      await cameraStream();
      setCameraAccess("granted");
    } catch (cameraError) {
      const denied =
        cameraError instanceof DOMException &&
        cameraError.name === "NotAllowedError";
      setCameraAccess(denied ? "denied" : "idle");
      setError(cameraAccessError(cameraError));
    }
  }

  async function startCurrentCameraFeed(stream: MediaStream) {
    cameraFeedRef.current?.stop();
    cameraFeedRef.current = await startCameraFeed({
      stream,
      udid,
      onError: setError,
      onStats: setCameraStats,
    });
  }

  async function apply(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedSimulator?.isBooted) {
      setError("Boot the selected simulator before starting the camera.");
      return;
    }
    setIsApplying(true);
    setError("");
    let stream: MediaStream | null = null;
    try {
      if (sourceMode === "camera") {
        stream = await cameraStream();
      }
      const nextStatus = updatesRunningCamera
        ? await switchCameraSimulationSource(udid, {
            mirror,
            source: requestSource(),
          })
        : await startCameraSimulation(udid, {
            mirror,
            source: requestSource(),
          });
      setStatus(nextStatus);
      if (stream) {
        await startCurrentCameraFeed(stream);
      } else {
        stopCameraFeed(true);
      }
    } catch (applyError) {
      if (stream) {
        stopCameraFeed(true);
      }
      if (
        sourceMode === "camera" &&
        applyError instanceof DOMException &&
        applyError.name === "NotAllowedError"
      ) {
        setCameraAccess("denied");
      }
      setError(
        sourceMode === "camera"
          ? cameraAccessError(applyError)
          : applyError instanceof Error
            ? applyError.message
            : "Unable to start the camera.",
      );
    } finally {
      setIsApplying(false);
    }
  }

  async function stop() {
    setIsStopping(true);
    setError("");
    try {
      stopCameraFeed(true);
      const nextStatus = await stopCameraSimulation(udid);
      setStatus(nextStatus);
    } catch (stopError) {
      setError(
        stopError instanceof Error
          ? stopError.message
          : "Unable to stop the camera.",
      );
    } finally {
      setIsStopping(false);
    }
  }

  function close() {
    if (!cameraFeedRef.current) {
      stopCameraFeed(true);
    }
    onClose();
  }

  return (
    <div
      className="new-sim-overlay"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) {
          close();
        }
      }}
    >
      <form
        aria-labelledby="camera-sim-title"
        aria-modal="true"
        className="new-sim-window camera-sim-window"
        onSubmit={apply}
        role="dialog"
      >
        <div className="new-sim-titlebar">
          <span className="new-sim-window-controls" aria-hidden="true">
            <span className="new-sim-window-dot close" />
            <span className="new-sim-window-dot minimize" />
            <span className="new-sim-window-dot zoom" />
          </span>
          <h2 id="camera-sim-title">Camera</h2>
        </div>

        <div className="new-sim-body">
          <div
            className="new-sim-platform-switcher camera-source-switcher"
            aria-label="Camera source"
          >
            <button
              className={sourceMode === "placeholder" ? "active" : ""}
              onClick={() => setSourceMode("placeholder")}
              type="button"
            >
              Pattern
            </button>
            <button
              className={sourceMode === "media" ? "active" : ""}
              onClick={() => setSourceMode("media")}
              type="button"
            >
              Media
            </button>
            <button
              className={sourceMode === "camera" ? "active" : ""}
              onClick={() => setSourceMode("camera")}
              type="button"
            >
              Camera
            </button>
          </div>
          <fieldset
            className="new-sim-fieldset camera-sim-fieldset"
            disabled={isApplying || isStopping}
          >
            {sourceMode === "media" ? (
              <label className="new-sim-field">
                <span>File or URL:</span>
                <input
                  autoCapitalize="none"
                  autoCorrect="off"
                  onChange={(event) => setMediaPath(event.currentTarget.value)}
                  placeholder="/Users/me/Movies/camera.mov"
                  value={mediaPath}
                />
              </label>
            ) : null}
            {sourceMode === "camera" ? (
              cameraAccess === "granted" ? (
                <label className="new-sim-field">
                  <span>Camera:</span>
                  <select
                    onChange={(event) => {
                      const nextCameraId = event.currentTarget.value;
                      setCameraId(nextCameraId);
                      storeCameraId(nextCameraId);
                    }}
                    value={cameraId}
                  >
                    <option value="">
                      {benchmarkMode
                        ? "Deterministic benchmark"
                        : "System default"}
                    </option>
                    {cameras.map((camera) => (
                      <option key={camera.id} value={camera.id}>
                        {camera.name}
                      </option>
                    ))}
                  </select>
                </label>
              ) : (
                <div className="camera-access-panel">
                  <span
                    className="camera-access-indicator"
                    aria-hidden="true"
                  />
                  <span className="camera-access-copy">
                    <strong>
                      {cameraAccess === "denied"
                        ? "Camera access is blocked"
                        : "Choose which camera to use"}
                    </strong>
                    <span>
                      {cameraAccess === "denied"
                        ? "Allow camera access for this site in your browser settings, then try again."
                        : "Your browser will ask for permission before showing available cameras."}
                    </span>
                  </span>
                  <button
                    className="new-sim-button camera-access-button"
                    disabled={cameraAccess === "requesting"}
                    onClick={requestCameraAccess}
                    type="button"
                  >
                    {cameraAccess === "requesting"
                      ? "Requesting..."
                      : cameraAccess === "denied"
                        ? "Try again"
                        : "Allow access"}
                  </button>
                </div>
              )
            ) : null}
            <label className="new-sim-field">
              <span>Mirror:</span>
              <select
                onChange={(event) =>
                  setMirror(event.currentTarget.value as MirrorMode)
                }
                value={mirror}
              >
                <option value="auto">Auto</option>
                <option value="off">Off</option>
                <option value="on">On</option>
              </select>
            </label>
            <p className="new-sim-status camera-sim-status">
              {isLoading ? "Loading camera status..." : `Status: ${statusText}`}
            </p>
            {cameraStats ? (
              <p className="new-sim-status camera-sim-transport-status">
                H.264 · {cameraStats.encodedFramesPerSecond} fps ·{" "}
                {(cameraStats.bitrate / 1_000_000).toFixed(1)} Mbps ·{" "}
                {cameraStats.outputWidth}×{cameraStats.outputHeight}
                {cameraStats.skippedFrames > 0
                  ? ` · ${cameraStats.skippedFrames} dropped`
                  : ""}
              </p>
            ) : null}
          </fieldset>
          {error ? <p className="new-sim-error">{error}</p> : null}
        </div>

        <div className="new-sim-actions">
          <button
            className="new-sim-button"
            disabled={isApplying || isStopping}
            onClick={close}
            type="button"
          >
            Close
          </button>
          {status?.alive ? (
            <button
              className="new-sim-button"
              disabled={isApplying || isStopping}
              onClick={stop}
              type="button"
            >
              {isStopping ? "Stopping..." : "Stop"}
            </button>
          ) : null}
          <span className="new-sim-action-spacer" />
          <button
            className="new-sim-button primary"
            disabled={!canApply || isApplying || isStopping}
            type="submit"
          >
            {isApplying
              ? updatesRunningCamera
                ? "Updating..."
                : "Starting..."
              : updatesRunningCamera
                ? "Update source"
                : "Start Camera"}
          </button>
        </div>
      </form>
    </div>
  );
}

function storedCameraId(): string {
  try {
    return window.localStorage.getItem(CAMERA_DEVICE_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

function storeCameraId(cameraId: string) {
  try {
    if (cameraId) {
      window.localStorage.setItem(CAMERA_DEVICE_STORAGE_KEY, cameraId);
    } else {
      window.localStorage.removeItem(CAMERA_DEVICE_STORAGE_KEY);
    }
  } catch {
    return;
  }
}

function cameraBenchmarkEnabled(): boolean {
  return (
    new URLSearchParams(window.location.search).get("cameraBenchmark") === "1"
  );
}

function cameraBenchmarkOptions() {
  const params = new URLSearchParams(window.location.search);
  const numeric = (name: string) => {
    const value = Number(params.get(name));
    return Number.isFinite(value) && value > 0 ? value : undefined;
  };
  return {
    framesPerSecond: numeric("cameraBenchmarkFps"),
    height: numeric("cameraBenchmarkHeight"),
    width: numeric("cameraBenchmarkWidth"),
  };
}

async function cameraPermissionWasGranted(): Promise<boolean> {
  try {
    const permission = await navigator.permissions?.query({
      name: "camera" as PermissionName,
    });
    if (permission) {
      return permission.state === "granted";
    }
  } catch {
    // Browsers without camera permission queries expose device labels after a grant.
  }
  try {
    return (await navigator.mediaDevices.enumerateDevices()).some(
      (device) =>
        device.kind === "videoinput" && device.label.trim().length > 0,
    );
  } catch {
    return false;
  }
}

function looksLikeVideo(value: string): boolean {
  if (/^https?:\/\//i.test(value)) {
    return true;
  }
  return /\.(mp4|m4v|mov|qt|avi|mkv|webm|mpg|mpeg|3gp|3g2)$/i.test(value);
}

function cameraAccessError(error: unknown): string {
  if (!(error instanceof DOMException)) {
    return error instanceof Error
      ? error.message
      : "Unable to access the camera.";
  }
  if (error.name === "NotAllowedError") {
    return "Camera access is blocked for this site. Change the camera permission in your browser settings, then try again.";
  }
  if (error.name === "NotFoundError") {
    return "No camera was found. Connect a camera, then try again.";
  }
  if (error.name === "NotReadableError") {
    return "The camera is in use by another application. Close it there, then try again.";
  }
  if (error.name === "OverconstrainedError") {
    return "The selected camera is no longer available. Choose another camera.";
  }
  return error.message || "Unable to access the camera.";
}
