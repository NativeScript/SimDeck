import { useEffect, useRef, useState } from "react";

import { apiHeaders } from "../../api/client";
import { apiUrl } from "../../api/config";
import type { SimulatorMetadata } from "../../api/types";
import type { Size } from "../viewport/types";
import { createEmptyStreamStats } from "./stats";
import {
  buildStreamTarget,
  sendStreamClientStats,
  StreamWorkerClient,
  type StreamBackend,
  type VisualArtifactSample,
} from "./streamWorkerClient";
import type {
  StreamRuntimeInfo,
  StreamConfig,
  StreamStats,
  StreamStatus,
  StreamTransport,
  WorkerToMainMessage,
} from "./streamTypes";

const FPS_SAMPLE_INTERVAL_MS = 1000;
const CLIENT_TELEMETRY_INTERVAL_MS = 1000;
const REMOTE_CLIENT_TELEMETRY_INTERVAL_MS = 5000;
const CLIENT_TELEMETRY_ID_STORAGE_KEY = "simdeck.streamClientId";
const VISUAL_ARTIFACT_TELEMETRY_INTERVAL_MS = 30000;

interface UseLiveStreamOptions {
  canvasElement: HTMLCanvasElement | null;
  paused?: boolean;
  remote?: boolean;
  simulator: SimulatorMetadata | null;
  streamConfig?: StreamConfig;
  streamConfigApplyKey?: number;
  streamTransport?: StreamTransport;
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
  try {
    const stored = window.sessionStorage.getItem(
      CLIENT_TELEMETRY_ID_STORAGE_KEY,
    );
    if (stored) {
      return stored;
    }
  } catch {
    // Some embedded browsers can deny sessionStorage; fall back to an in-memory ID.
  }
  const id =
    window.crypto?.randomUUID?.() ??
    `page-${Math.random().toString(36).slice(2)}`;
  try {
    window.sessionStorage.setItem(CLIENT_TELEMETRY_ID_STORAGE_KEY, id);
  } catch {
    // Best effort only.
  }
  return id;
}

function buildClientTelemetryUrl(): string {
  return new URL(
    apiUrl("/api/client-stream-stats"),
    window.location.href,
  ).toString();
}

function currentClientBundle(): string {
  return (
    Array.from(document.scripts)
      .map((script) => script.src)
      .find((src) => /\/assets\/index-[^/]+\.js(?:$|\?)/.test(src))
      ?.split("/")
      .pop() ?? ""
  );
}

export function useLiveStream({
  canvasElement,
  paused = false,
  remote = false,
  simulator,
  streamConfig,
  streamConfigApplyKey = 0,
  streamTransport = "auto",
}: UseLiveStreamOptions): UseLiveStreamResult {
  const clientTelemetryIdRef = useRef("");
  const workerClientRef = useRef<StreamWorkerClient | null>(null);
  const latestDecodedFramesRef = useRef(0);
  const latestFpsRef = useRef(0);
  const latestRenderedFramesRef = useRef(0);
  const latestStatsRef = useRef<StreamStats>(createEmptyStreamStats());
  const latestStatusRef = useRef<StreamStatus>({ state: "idle" });
  const retainedFrameRef = useRef(false);
  const previousSimulatorUdidRef = useRef<string | undefined>(simulator?.udid);
  const connectedStreamTargetKeyRef = useRef("");
  const latestVisualArtifactRef = useRef<VisualArtifactSample | null>(null);
  const latestVisualArtifactSampleCountRef = useRef(0);
  const lastVisualArtifactSampleAtRef = useRef(0);
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
    if (!canvasElement) {
      return;
    }

    let workerClient = workerClientRef.current;
    if (!workerClient) {
      workerClient = new StreamWorkerClient((message: WorkerToMainMessage) => {
        if (message.type === "stats") {
          if (
            message.stats.decodedFrames > 0 ||
            message.stats.renderedFrames > 0
          ) {
            retainedFrameRef.current = true;
          }
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
      });
      workerClientRef.current = workerClient;
    }

    try {
      workerClient.attachCanvas(canvasElement);
    } catch (attachError) {
      const message =
        attachError instanceof Error
          ? attachError.message
          : "Unable to attach the stream canvas.";
      setError(message);
      setStatus({ error: message, state: "error" });
      workerClient.destroy();
      workerClientRef.current = null;
      return;
    }
  }, [canvasElement]);

  useEffect(() => {
    return () => {
      workerClientRef.current?.destroy();
      workerClientRef.current = null;
      connectedStreamTargetKeyRef.current = "";
    };
  }, []);

  useEffect(() => {
    latestDecodedFramesRef.current = stats.decodedFrames;
    latestRenderedFramesRef.current = stats.renderedFrames;
    latestStatsRef.current = stats;
  }, [stats]);

  useEffect(() => {
    latestStatusRef.current = status;
  }, [status]);

  useEffect(() => {
    latestFpsRef.current = fps;
  }, [fps]);

  useEffect(() => {
    const previousUdid = previousSimulatorUdidRef.current;
    const nextUdid = simulator?.udid;
    if (previousUdid === nextUdid) {
      return;
    }
    previousSimulatorUdidRef.current = nextUdid;
    retainedFrameRef.current = false;
    if (previousUdid && nextUdid) {
      setStreamCanvasRevision((current) => current + 1);
    }
  }, [simulator?.udid]);

  useEffect(() => {
    let lastSampleDecodedFrames = latestDecodedFramesRef.current;
    let lastSampleRenderedFrames = latestRenderedFramesRef.current;
    let lastSampleAt = performance.now();
    setFps(0);

    const intervalId = window.setInterval(() => {
      const now = performance.now();
      const decodedFrames = latestDecodedFramesRef.current;
      const renderedFrames = latestRenderedFramesRef.current;
      const elapsedMs = now - lastSampleAt;
      if (elapsedMs <= 0) {
        return;
      }

      const decodedDelta = decodedFrames - lastSampleDecodedFrames;
      const renderedDelta = renderedFrames - lastSampleRenderedFrames;
      const frameDelta = decodedDelta > 0 ? decodedDelta : renderedDelta;
      const nextFps = Math.max(0, (frameDelta * 1000) / elapsedMs);
      setFps((current) =>
        current <= 0 ? nextFps : current * 0.65 + nextFps * 0.35,
      );
      lastSampleDecodedFrames = decodedFrames;
      lastSampleRenderedFrames = renderedFrames;
      lastSampleAt = now;
    }, FPS_SAMPLE_INTERVAL_MS);

    return () => {
      clearInterval(intervalId);
    };
  }, [simulator?.udid]);

  useEffect(() => {
    const workerClient = workerClientRef.current;
    if (!workerClient || !canvasElement) {
      return;
    }

    if (paused || !simulator?.isBooted) {
      setDeviceNaturalSize(null);
      setStats(createEmptyStreamStats());
      setStatus({ state: "idle" });
      setError("");
      setFps(0);
      retainedFrameRef.current = false;
      if (connectedStreamTargetKeyRef.current) {
        workerClient.disconnect();
      }
      connectedStreamTargetKeyRef.current = "";
      workerClient.clear();
      return;
    }

    const targetKey = [
      simulator.udid,
      remote ? "remote" : "local",
      streamTransport,
    ].join("|");
    if (connectedStreamTargetKeyRef.current === targetKey) {
      return;
    }
    setDeviceNaturalSize(null);
    setStats(createEmptyStreamStats());
    setStatus({ state: "idle" });
    setError("");
    setFps(0);
    connectedStreamTargetKeyRef.current = targetKey;
    workerClient.connect(
      buildStreamTarget(simulator.udid, {
        clientId: clientTelemetryIdRef.current,
        remote,
        streamConfig,
        transport: streamTransport,
      }),
    );
  }, [
    canvasElement,
    simulator?.isBooted,
    simulator?.udid,
    paused,
    remote,
    streamTransport,
  ]);

  useEffect(() => {
    if (
      streamConfigApplyKey <= 0 ||
      paused ||
      !simulator?.isBooted ||
      !streamConfig
    ) {
      return;
    }
    workerClientRef.current?.applyStreamConfig(streamConfig);
  }, [
    paused,
    simulator?.isBooted,
    streamConfigApplyKey,
    streamConfig?.encoder,
    streamConfig?.fps,
    streamConfig?.maxEdge,
    streamConfig?.quality,
  ]);

  useEffect(() => {
    if (!simulator?.udid) {
      return;
    }

    const postTelemetry = async () => {
      const latestStats = latestStatsRef.current;
      const latestStatus = latestStatusRef.current;
      const now = Date.now();
      if (
        !remote &&
        now - lastVisualArtifactSampleAtRef.current >=
          VISUAL_ARTIFACT_TELEMETRY_INTERVAL_MS
      ) {
        lastVisualArtifactSampleAtRef.current = now;
        const visualSample = await workerClientRef.current
          ?.collectVisualArtifactSample?.(simulator.udid)
          .catch(() => null);
        if (visualSample) {
          latestVisualArtifactRef.current = visualSample;
          latestVisualArtifactSampleCountRef.current += 1;
        }
      }
      const latestVisualArtifact = latestVisualArtifactRef.current;
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
        clientBundle: currentClientBundle(),
        visualBadPixelRatio: latestVisualArtifact?.badPixelRatio,
        visualMaxPixelDiff: latestVisualArtifact?.maxPixelDiff,
        visualMaxTileDiff: latestVisualArtifact?.maxTileMeanDiff,
        visualMeanDiff: latestVisualArtifact?.meanDiff,
        visualSampleCount: latestVisualArtifactSampleCountRef.current,
        visibilityState: document.visibilityState,
      };
      if (
        sendStreamClientStats(payload) ||
        remote ||
        streamTransport === "h264" ||
        streamTransport === "webrtc" ||
        streamTransport === "auto"
      ) {
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
    void postTelemetry();
    const intervalId = window.setInterval(postTelemetry, intervalMs);
    return () => {
      window.clearInterval(intervalId);
    };
  }, [remote, simulator?.udid, streamTransport]);

  return {
    deviceNaturalSize,
    error,
    fps,
    hasFrame:
      status.state === "streaming" ||
      stats.decodedFrames > 0 ||
      retainedFrameRef.current,
    runtimeInfo,
    stats,
    status,
    streamBackend: stats.codec === "h264-ws" ? "h264-ws" : "webrtc",
    streamCanvasKey: `stream-${streamCanvasRevision}`,
  };
}
