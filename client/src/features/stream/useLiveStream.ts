import { useEffect, useRef, useState } from "react";

import { apiHeaders } from "../../api/client";
import { apiUrl } from "../../api/config";
import type { SimulatorMetadata } from "../../api/types";
import type { Size } from "../viewport/types";
import { createEmptyStreamStats } from "./stats";
import {
  buildStreamTarget,
  canUseWebRtc,
  sendWebRtcClientStats,
  StreamWorkerClient,
  type StreamBackend,
} from "./streamWorkerClient";
import type {
  StreamRuntimeInfo,
  StreamStats,
  StreamStatus,
  WorkerToMainMessage,
} from "./streamTypes";

const FPS_SAMPLE_INTERVAL_MS = 500;
const CLIENT_TELEMETRY_INTERVAL_MS = 1000;
const REMOTE_CLIENT_TELEMETRY_INTERVAL_MS = 5000;

interface UseLiveStreamOptions {
  canvasElement: HTMLCanvasElement | null;
  paused?: boolean;
  remote?: boolean;
  simulator: SimulatorMetadata | null;
  streamProfile?: "focus" | "full" | "paused" | "thumb" | "thumbnail";
}

interface UseLiveStreamResult {
  deviceNaturalSize: Size | null;
  error: string;
  fps: number;
  hasFrame: boolean;
  runtimeInfo: StreamRuntimeInfo;
  status: StreamStatus;
  stats: StreamStats;
  streamBackend: StreamBackend;
  streamCanvasKey: string;
}

function detectRuntimeInfo(): StreamRuntimeInfo {
  return {
    gpuLikelyHardware: null,
    gpuRenderer: "",
    gpuVendor: "",
    renderBackend: "Browser video",
    streamBackend: "Browser WebRTC",
    webGL2: false,
  };
}

function createClientTelemetryId(): string {
  return (
    window.crypto?.randomUUID?.() ??
    `page-${Math.random().toString(36).slice(2)}`
  );
}

function buildClientTelemetryUrl(): string {
  return new URL(
    apiUrl("/api/client-stream-stats"),
    window.location.href,
  ).toString();
}

export function useLiveStream({
  canvasElement,
  paused = false,
  remote = false,
  simulator,
  streamProfile = "focus",
}: UseLiveStreamOptions): UseLiveStreamResult {
  const clientTelemetryIdRef = useRef("");
  const workerClientRef = useRef<StreamWorkerClient | null>(null);
  const latestDecodedFramesRef = useRef(0);
  const latestFpsRef = useRef(0);
  const latestStatsRef = useRef<StreamStats>(createEmptyStreamStats());
  const latestStatusRef = useRef<StreamStatus>({ state: "idle" });
  const pageFpsRef = useRef(0);
  const [deviceNaturalSize, setDeviceNaturalSize] = useState<Size | null>(null);
  const [stats, setStats] = useState<StreamStats>(createEmptyStreamStats);
  const [status, setStatus] = useState<StreamStatus>({ state: "idle" });
  const [error, setError] = useState("");
  const [fps, setFps] = useState(0);
  const [streamCanvasRevision, setStreamCanvasRevision] = useState(0);
  const [runtimeInfo] = useState<StreamRuntimeInfo>(detectRuntimeInfo);

  if (!clientTelemetryIdRef.current) {
    clientTelemetryIdRef.current = createClientTelemetryId();
  }

  useEffect(() => {
    let frameCount = 0;
    let lastSampleAt = performance.now();
    let rafId = 0;

    const tick = () => {
      frameCount += 1;
      const now = performance.now();
      const elapsedMs = now - lastSampleAt;
      if (elapsedMs >= CLIENT_TELEMETRY_INTERVAL_MS) {
        pageFpsRef.current = (frameCount * 1000) / elapsedMs;
        frameCount = 0;
        lastSampleAt = now;
      }

      rafId = window.requestAnimationFrame(tick);
    };

    rafId = window.requestAnimationFrame(tick);
    return () => {
      window.cancelAnimationFrame(rafId);
    };
  }, []);

  useEffect(() => {
    if (paused || !canvasElement || workerClientRef.current) {
      return;
    }

    const workerClient = new StreamWorkerClient(
      (message: WorkerToMainMessage) => {
        if (message.type === "stats") {
          setStats(message.stats);
          return;
        }

        if (message.type === "status") {
          setStatus(message.status);
          if (message.status.error) {
            setError(message.status.error);
          } else if (
            message.status.state === "streaming" ||
            message.status.state === "idle" ||
            message.status.state === "connecting"
          ) {
            setError("");
          }
          return;
        }

        setDeviceNaturalSize(message.size);
      },
    );

    try {
      workerClient.attachCanvas(canvasElement);
      workerClientRef.current = workerClient;
    } catch (attachError) {
      const message =
        attachError instanceof Error
          ? attachError.message
          : "Unable to attach the stream canvas.";
      setError(message);
      setStatus({ error: message, state: "error" });
      workerClient.destroy();
      return;
    }

    const destroyOnPageHide = () => {
      workerClient.destroy();
      if (workerClientRef.current === workerClient) {
        workerClientRef.current = null;
      }
    };
    window.addEventListener("pagehide", destroyOnPageHide);

    return () => {
      window.removeEventListener("pagehide", destroyOnPageHide);
      workerClient.destroy();
      workerClientRef.current = null;
    };
  }, [canvasElement, paused]);

  useEffect(() => {
    latestDecodedFramesRef.current = stats.decodedFrames;
    latestStatsRef.current = stats;
  }, [stats]);

  useEffect(() => {
    latestStatusRef.current = status;
  }, [status]);

  useEffect(() => {
    latestFpsRef.current = fps;
  }, [fps]);

  useEffect(() => {
    setStreamCanvasRevision((current) => current + 1);
  }, [simulator?.udid]);

  useEffect(() => {
    let lastSampleFrames = latestDecodedFramesRef.current;
    let lastSampleAt = performance.now();
    setFps(0);

    const intervalId = window.setInterval(() => {
      const now = performance.now();
      const decodedFrames = latestDecodedFramesRef.current;
      const elapsedMs = now - lastSampleAt;
      if (elapsedMs <= 0) {
        return;
      }

      const nextFps = ((decodedFrames - lastSampleFrames) * 1000) / elapsedMs;
      setFps(Math.max(0, nextFps));
      lastSampleFrames = decodedFrames;
      lastSampleAt = now;
    }, FPS_SAMPLE_INTERVAL_MS);

    return () => {
      clearInterval(intervalId);
    };
  }, [simulator?.udid]);

  useEffect(() => {
    const workerClient = workerClientRef.current;
    if (!workerClient) {
      return;
    }

    setDeviceNaturalSize(null);
    setStats(createEmptyStreamStats());
    setStatus({ state: "idle" });
    setError("");
    setFps(0);

    if (paused || !simulator?.isBooted) {
      workerClient.disconnect();
      workerClient.clear();
      return;
    }

    if (!canUseWebRtc()) {
      setStatus({
        error: "This browser does not support WebRTC video.",
        state: "error",
      });
      return;
    }

    workerClient.connect(
      buildStreamTarget(simulator.udid, {
        clientId: clientTelemetryIdRef.current,
        remote,
      }),
    );
    return () => {
      workerClient.disconnect();
    };
  }, [canvasElement, simulator?.isBooted, simulator?.udid, paused, remote]);

  useEffect(() => {
    if (paused || !simulator?.isBooted) {
      return;
    }
    let attempts = 0;
    const send = () => {
      attempts += 1;
      return Boolean(
        workerClientRef.current?.sendStreamControl({
          forceKeyframe: streamProfile === "focus" || streamProfile === "full",
          profile: streamProfile,
        }),
      );
    };
    if (send()) {
      return;
    }
    const interval = window.setInterval(() => {
      if (send() || attempts >= 8) {
        window.clearInterval(interval);
      }
    }, 250);
    return () => window.clearInterval(interval);
  }, [paused, simulator?.isBooted, simulator?.udid, streamProfile]);

  useEffect(() => {
    if (!simulator?.udid) {
      return;
    }

    const postTelemetry = () => {
      const latestStats = latestStatsRef.current;
      const latestStatus = latestStatusRef.current;
      const payload = {
        ...latestStats,
        appFps: latestFpsRef.current,
        clientId: clientTelemetryIdRef.current,
        focused: document.hasFocus(),
        kind: "page",
        pageFps: pageFpsRef.current,
        status: latestStatus.state,
        timestampMs: Date.now(),
        udid: simulator.udid,
        url: window.location.href,
        userAgent: window.navigator.userAgent,
        visibilityState: document.visibilityState,
      };
      if (sendWebRtcClientStats(payload) || remote) {
        return;
      }
      void fetch(buildClientTelemetryUrl(), {
        body: JSON.stringify(payload),
        cache: "no-store",
        headers: apiHeaders(),
        method: "POST",
      }).catch(() => {
        // Diagnostic only; UI state should never depend on telemetry.
      });
    };

    const intervalMs = remote
      ? REMOTE_CLIENT_TELEMETRY_INTERVAL_MS
      : CLIENT_TELEMETRY_INTERVAL_MS;
    postTelemetry();
    const intervalId = window.setInterval(postTelemetry, intervalMs);
    return () => {
      window.clearInterval(intervalId);
    };
  }, [remote, simulator?.udid]);

  return {
    deviceNaturalSize,
    error,
    fps,
    hasFrame: status.state === "streaming" || stats.decodedFrames > 0,
    runtimeInfo,
    stats,
    status,
    streamBackend: "webrtc",
    streamCanvasKey: `webrtc-${streamCanvasRevision}`,
  };
}
