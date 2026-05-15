import { useEffect, useMemo, useState, type FormEvent } from "react";

import {
  fetchCameraStatus,
  fetchCameraWebcams,
  startCameraSimulation,
  stopCameraSimulation,
  switchCameraSimulationSource,
} from "../../api/simulators";
import type {
  CameraSourceKind,
  CameraStatusResponse,
  CameraWebcam,
  SimulatorMetadata,
} from "../../api/types";

interface CameraSimulationModalProps {
  foregroundBundleId?: string | null;
  onClose: () => void;
  open: boolean;
  selectedSimulator: SimulatorMetadata | null;
}

type SourceMode = "placeholder" | "webcam" | "media";
type MirrorMode = "auto" | "on" | "off";

export function CameraSimulationModal({
  foregroundBundleId,
  onClose,
  open,
  selectedSimulator,
}: CameraSimulationModalProps) {
  const [bundleId, setBundleId] = useState("");
  const [sourceMode, setSourceMode] = useState<SourceMode>("placeholder");
  const [mediaPath, setMediaPath] = useState("");
  const [webcamId, setWebcamId] = useState("");
  const [mirror, setMirror] = useState<MirrorMode>("auto");
  const [status, setStatus] = useState<CameraStatusResponse | null>(null);
  const [webcams, setWebcams] = useState<CameraWebcam[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isApplying, setIsApplying] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [error, setError] = useState("");

  const udid = selectedSimulator?.udid ?? "";
  const canApply = Boolean(
    selectedSimulator?.isBooted &&
    bundleId.trim() &&
    (sourceMode !== "media" || mediaPath.trim()),
  );

  useEffect(() => {
    if (!open) {
      return;
    }
    setBundleId(foregroundBundleId ?? "");
    setError("");
    setIsApplying(false);
    setIsStopping(false);
    void refreshStatus();
    void refreshWebcams();
  }, [foregroundBundleId, open, udid]);

  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  const activeBundleText = useMemo(() => {
    const bundleIds = status?.bundleIds ?? [];
    if (bundleIds.length === 0) {
      return status?.alive ? "helper running" : "not running";
    }
    return bundleIds.join(", ");
  }, [status]);

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
      if (nextStatus.mirror === "on" || nextStatus.mirror === "off") {
        setMirror(nextStatus.mirror);
      }
      if (
        nextStatus.source === "webcam" ||
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
        if (nextStatus.source === "webcam") {
          setWebcamId(nextStatus.arg);
        } else if (
          nextStatus.source === "image" ||
          nextStatus.source === "video"
        ) {
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

  async function refreshWebcams() {
    try {
      const response = await fetchCameraWebcams();
      setWebcams(response.webcams ?? []);
      setWebcamId((current) => current || response.webcams?.[0]?.id || "");
    } catch {
      setWebcams([]);
    }
  }

  function requestSource(): { kind: CameraSourceKind; arg?: string } {
    if (sourceMode === "webcam") {
      return { kind: "webcam", arg: webcamId || undefined };
    }
    if (sourceMode === "media") {
      const value = mediaPath.trim();
      const kind: CameraSourceKind = looksLikeVideo(value) ? "video" : "image";
      return { kind, arg: value };
    }
    return { kind: "placeholder" };
  }

  async function apply(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedSimulator?.isBooted) {
      setError(
        "Boot the selected simulator before enabling camera simulation.",
      );
      return;
    }
    if (!bundleId.trim()) {
      setError(
        "Enter the app bundle identifier to relaunch with camera simulation.",
      );
      return;
    }
    setIsApplying(true);
    setError("");
    try {
      const nextStatus = await startCameraSimulation(udid, {
        bundleId: bundleId.trim(),
        mirror,
        source: requestSource(),
      });
      setStatus(nextStatus);
    } catch (applyError) {
      setError(
        applyError instanceof Error
          ? applyError.message
          : "Unable to start camera simulation.",
      );
    } finally {
      setIsApplying(false);
    }
  }

  async function switchSourceOnly() {
    if (!status?.alive) {
      return;
    }
    setIsApplying(true);
    setError("");
    try {
      const nextStatus = await switchCameraSimulationSource(udid, {
        mirror,
        source: requestSource(),
      });
      setStatus(nextStatus);
    } catch (switchError) {
      setError(
        switchError instanceof Error
          ? switchError.message
          : "Unable to switch camera source.",
      );
    } finally {
      setIsApplying(false);
    }
  }

  async function stop() {
    setIsStopping(true);
    setError("");
    try {
      const nextStatus = await stopCameraSimulation(udid);
      setStatus(nextStatus);
    } catch (stopError) {
      setError(
        stopError instanceof Error
          ? stopError.message
          : "Unable to stop camera simulation.",
      );
    } finally {
      setIsStopping(false);
    }
  }

  return (
    <div
      className="new-sim-overlay"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
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
          <h2 id="camera-sim-title">Camera Simulation</h2>
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
              className={sourceMode === "webcam" ? "active" : ""}
              onClick={() => setSourceMode("webcam")}
              type="button"
            >
              Webcam
            </button>
          </div>
          <fieldset
            className="new-sim-fieldset camera-sim-fieldset"
            disabled={isApplying || isStopping}
          >
            <label className="new-sim-field">
              <span>Bundle ID:</span>
              <input
                autoCapitalize="none"
                autoCorrect="off"
                autoFocus
                onChange={(event) => setBundleId(event.currentTarget.value)}
                placeholder="com.example.app"
                value={bundleId}
              />
            </label>
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
            {sourceMode === "webcam" ? (
              <label className="new-sim-field">
                <span>Mac Camera:</span>
                <select
                  onChange={(event) => setWebcamId(event.currentTarget.value)}
                  value={webcamId}
                >
                  {webcams.length === 0 ? (
                    <option value="">No cameras found</option>
                  ) : null}
                  {webcams.map((webcam) => (
                    <option key={webcam.id} value={webcam.id}>
                      {webcam.name}
                    </option>
                  ))}
                </select>
              </label>
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
              {isLoading
                ? "Loading camera status..."
                : `Status: ${activeBundleText}`}
            </p>
          </fieldset>
          {error ? <p className="new-sim-error">{error}</p> : null}
        </div>

        <div className="new-sim-actions">
          <button
            className="new-sim-button"
            disabled={isApplying || isStopping}
            onClick={onClose}
            type="button"
          >
            Cancel
          </button>
          <button
            className="new-sim-button"
            disabled={!status?.alive || isApplying || isStopping}
            onClick={switchSourceOnly}
            type="button"
          >
            Switch
          </button>
          <button
            className="new-sim-button"
            disabled={!status?.alive || isApplying || isStopping}
            onClick={stop}
            type="button"
          >
            {isStopping ? "Stopping..." : "Stop"}
          </button>
          <span className="new-sim-action-spacer" />
          <button
            className="new-sim-button"
            disabled={!canApply || isApplying || isStopping}
            type="submit"
          >
            {isApplying ? "Applying..." : "Apply"}
          </button>
        </div>
      </form>
    </div>
  );
}

function looksLikeVideo(value: string): boolean {
  if (/^https?:\/\//i.test(value)) {
    return true;
  }
  return /\.(mp4|m4v|mov|qt|avi|mkv|webm|mpg|mpeg|3gp|3g2)$/i.test(value);
}
