import type { EncoderStats } from "../../api/types";
import type {
  StreamRuntimeInfo,
  StreamStats,
  StreamStatus,
} from "../stream/streamTypes";

interface DebugPanelProps {
  encoder?: EncoderStats | null;
  fps: number;
  inline?: boolean;
  onClose?: () => void;
  runtimeInfo: StreamRuntimeInfo;
  stats: StreamStats;
  status: StreamStatus;
}

function formatFps(value: number): string {
  if (!Number.isFinite(value)) {
    return "0.0";
  }
  return value.toFixed(1);
}

function formatMs(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return "—";
  }
  return `${value.toFixed(1)} ms`;
}

function formatUsAsMs(value: number | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    return "—";
  }
  return `${(value / 1000).toFixed(1)} ms`;
}

function formatPercent(value: number | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "—";
  }
  return `${value.toFixed(0)}%`;
}

function formatResolution(stats: StreamStats): string {
  if (!stats.width || !stats.height) {
    return "—";
  }
  return `${stats.width}×${stats.height}`;
}

export function DebugPanel({
  encoder,
  fps,
  inline = false,
  onClose,
  runtimeInfo,
  stats,
  status,
}: DebugPanelProps) {
  const rows: Array<{ label: string; value: string }> = [
    { label: "State", value: status.state },
    { label: "FPS", value: formatFps(fps) },
    { label: "Resolution", value: formatResolution(stats) },
    { label: "Packets", value: String(stats.receivedPackets) },
    { label: "Packet Loss", value: String(stats.packetsLost) },
    { label: "Decode Drops", value: String(stats.decoderDroppedFrames) },
    { label: "Present Drops", value: String(stats.presentationDroppedFrames) },
    { label: "Reconnects", value: String(stats.reconnects) },
    { label: "Reconnect Reason", value: stats.reconnectReason || "—" },
    { label: "ICE Restarts", value: String(stats.iceRestarts) },
    { label: "ICE Restart Reason", value: stats.iceRestartReason || "—" },
    { label: "Decoded", value: String(stats.decodedFrames) },
    { label: "Rendered", value: String(stats.renderedFrames) },
    { label: "Render", value: formatMs(stats.latestRenderMs) },
    { label: "Frame Gap", value: formatMs(stats.latestFrameGapMs) },
    { label: "Path", value: runtimeInfo.streamBackend },
  ];
  if (encoder) {
    rows.push(
      { label: "Encoder", value: encoder.encoderMode ?? "—" },
      { label: "Encoder State", value: encoder.overloadState ?? "—" },
      {
        label: "Encoder Load",
        value: formatPercent(encoder.averageEncoderLoadPercent),
      },
      {
        label: "Encode Latency",
        value: formatUsAsMs(encoder.averageEncodeLatencyUs),
      },
      { label: "Encode Budget", value: formatUsAsMs(encoder.encoderBudgetUs) },
      { label: "Encoder Reason", value: encoder.overloadReason ?? "—" },
      {
        label: "Overload Events",
        value: String(encoder.overloadEvents ?? 0),
      },
    );
  }

  return (
    <section
      aria-label="Stream debug info"
      className={`debug-panel ${inline ? "debug-panel-inline" : "debug-panel-popover"}`}
    >
      <div className="debug-panel-header">
        <span>Debug Info</span>
        {onClose ? (
          <button
            aria-label="Close debug info"
            className="debug-close"
            onClick={onClose}
            title="Close"
            type="button"
          >
            x
          </button>
        ) : null}
      </div>
      <dl className="debug-grid">
        {rows.map((row) => (
          <div className="debug-row" key={row.label}>
            <dt className="debug-label">{row.label}</dt>
            <dd className="debug-value">{row.value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
