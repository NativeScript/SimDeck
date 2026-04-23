import { useEffect, useRef, useState } from "react";

import type { SimulatorMetadata } from "../../api/types";
import type { Size } from "../viewport/types";
import { createEmptyStreamStats } from "./stats";
import { buildStreamTarget, StreamWorkerClient } from "./streamWorkerClient";
import type {
  StreamRuntimeInfo,
  StreamStats,
  StreamStatus,
  WorkerToMainMessage,
} from "./streamTypes";

const FPS_SAMPLE_INTERVAL_MS = 500;
const CLIENT_TELEMETRY_INTERVAL_MS = 1000;
const DEBUG_RENDERER_SOFTWARE_PATTERN = /swiftshader|software|llvmpipe/i;

interface UseLiveStreamOptions {
  canvasElement: HTMLCanvasElement | null;
  simulator: SimulatorMetadata | null;
}

interface UseLiveStreamResult {
  deviceNaturalSize: Size | null;
  error: string;
  fps: number;
  hasFrame: boolean;
  runtimeInfo: StreamRuntimeInfo;
  status: StreamStatus;
  stats: StreamStats;
}

function createDefaultRuntimeInfo(): StreamRuntimeInfo {
  return {
    gpuLikelyHardware: null,
    gpuRenderer: "",
    gpuVendor: "",
    renderBackend: "Unavailable",
    streamBackend: "Worker / WebTransport",
    webCodecs: false,
    webGL2: false,
    webTransport: false,
  };
}

function detectRuntimeInfo(): StreamRuntimeInfo {
  const runtimeInfo = createDefaultRuntimeInfo();
  runtimeInfo.webCodecs = typeof VideoDecoder === "function";
  runtimeInfo.webTransport = typeof WebTransport === "function";

  try {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2", {
      alpha: false,
      antialias: false,
      depth: false,
      powerPreference: "high-performance",
      stencil: false,
    });
    if (!gl) {
      runtimeInfo.gpuLikelyHardware = false;
      return runtimeInfo;
    }

    runtimeInfo.webGL2 = true;
    runtimeInfo.renderBackend = "WebGL2";

    const debugInfo = gl.getExtension("WEBGL_debug_renderer_info");
    if (debugInfo) {
      runtimeInfo.gpuVendor = String(
        gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL) ?? "",
      );
      runtimeInfo.gpuRenderer = String(
        gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) ?? "",
      );
    }

    const gpuDescriptor =
      `${runtimeInfo.gpuVendor} ${runtimeInfo.gpuRenderer}`.trim();
    runtimeInfo.gpuLikelyHardware = gpuDescriptor
      ? !DEBUG_RENDERER_SOFTWARE_PATTERN.test(gpuDescriptor)
      : true;

    const loseContext = gl.getExtension("WEBGL_lose_context");
    loseContext?.loseContext();
  } catch {
    runtimeInfo.gpuLikelyHardware = false;
  }

  return runtimeInfo;
}

function createClientTelemetryId(): string {
  return (
    window.crypto?.randomUUID?.() ??
    `page-${Math.random().toString(36).slice(2)}`
  );
}

function buildClientTelemetryUrl(): string {
  return new URL("/api/client-stream-stats", window.location.href).toString();
}

export function useLiveStream({
  canvasElement,
  simulator,
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
  const [runtimeInfo, setRuntimeInfo] = useState<StreamRuntimeInfo>(
    createDefaultRuntimeInfo,
  );

  if (!clientTelemetryIdRef.current) {
    clientTelemetryIdRef.current = createClientTelemetryId();
  }

  useEffect(() => {
    setRuntimeInfo(detectRuntimeInfo());
  }, []);

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
    if (!canvasElement || workerClientRef.current) {
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
            message.status.state === "idle"
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
    } catch (error) {
      setError(
        error instanceof Error
          ? error.message
          : "Unable to attach the stream canvas.",
      );
      setStatus({
        error:
          error instanceof Error
            ? error.message
            : "Unable to attach the stream canvas.",
        state: "error",
      });
      workerClient.destroy();
      return;
    }

    return () => {
      workerClient.destroy();
      workerClientRef.current = null;
    };
  }, [canvasElement]);

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

    if (!simulator?.isBooted) {
      workerClient.disconnect();
      workerClient.clear();
      return;
    }

    workerClient.connect(buildStreamTarget(simulator.udid));
    return () => {
      workerClient.disconnect();
    };
  }, [canvasElement, simulator?.isBooted, simulator?.udid]);

  useEffect(() => {
    if (!simulator?.udid) {
      return;
    }

    const postTelemetry = () => {
      const latestStats = latestStatsRef.current;
      const latestStatus = latestStatusRef.current;
      void fetch(buildClientTelemetryUrl(), {
        body: JSON.stringify({
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
        }),
        cache: "no-store",
        headers: { "content-type": "application/json" },
        method: "POST",
      }).catch(() => {
        // Diagnostic only; UI state should never depend on telemetry.
      });
    };

    postTelemetry();
    const intervalId = window.setInterval(
      postTelemetry,
      CLIENT_TELEMETRY_INTERVAL_MS,
    );
    return () => {
      window.clearInterval(intervalId);
    };
  }, [simulator?.udid]);

  return {
    deviceNaturalSize,
    error,
    fps,
    hasFrame: status.state === "streaming" || stats.decodedFrames > 0,
    runtimeInfo,
    stats,
    status,
  };
}
