import type {
  StreamRuntimeInfo,
  StreamStats,
  StreamStatus,
} from "../stream/streamTypes";

interface DebugPanelProps {
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

function formatResolution(stats: StreamStats): string {
  if (!stats.width || !stats.height) {
    return "—";
  }
  return `${stats.width}×${stats.height}`;
}

export function DebugPanel({
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
    { label: "Dropped", value: String(stats.droppedFrames) },
    { label: "Reconnects", value: String(stats.reconnects) },
    { label: "Decoded", value: String(stats.decodedFrames) },
    { label: "Rendered", value: String(stats.renderedFrames) },
    { label: "Render", value: formatMs(stats.latestRenderMs) },
    { label: "Frame Gap", value: formatMs(stats.latestFrameGapMs) },
    { label: "Path", value: runtimeInfo.streamBackend },
  ];

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
