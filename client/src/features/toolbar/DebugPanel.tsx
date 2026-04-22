import type {
  StreamRuntimeInfo,
  StreamStats,
  StreamStatus,
} from "../stream/streamTypes";

interface DebugPanelProps {
  fps: number;
  inline?: boolean;
  runtimeInfo: StreamRuntimeInfo;
  stats: StreamStats;
  status: StreamStatus;
}

function formatBoolean(value: boolean): string {
  return value ? "Yes" : "No";
}

function formatFps(value: number): string {
  if (!Number.isFinite(value)) {
    return "0.0";
  }
  return value.toFixed(1);
}

function formatGpuStatus(runtimeInfo: StreamRuntimeInfo): string {
  if (!runtimeInfo.webGL2) {
    return "No";
  }
  if (runtimeInfo.gpuLikelyHardware == null) {
    return "Unknown";
  }
  return runtimeInfo.gpuLikelyHardware ? "Yes" : "No";
}

function formatResolution(stats: StreamStats): string {
  if (!stats.width || !stats.height) {
    return "—";
  }
  return `${stats.width}×${stats.height}`;
}

function formatValue(value: string | number | undefined): string {
  if (value == null || value === "") {
    return "—";
  }
  return String(value);
}

export function DebugPanel({
  fps,
  inline = false,
  runtimeInfo,
  stats,
  status,
}: DebugPanelProps) {
  const rows: Array<{ label: string; value: string }> = [
    { label: "State", value: status.state },
    { label: "FPS", value: formatFps(fps) },
    { label: "Codec", value: formatValue(stats.codec) },
    { label: "Resolution", value: formatResolution(stats) },
    { label: "Packets", value: String(stats.receivedPackets) },
    { label: "Dropped", value: String(stats.droppedFrames) },
    { label: "Reconnects", value: String(stats.reconnects) },
    { label: "Frame Seq", value: String(stats.frameSequence) },
    { label: "Decoded", value: String(stats.decodedFrames) },
    { label: "Path", value: runtimeInfo.streamBackend },
    { label: "Renderer", value: runtimeInfo.renderBackend },
    { label: "GPU", value: formatGpuStatus(runtimeInfo) },
    { label: "WebGL2", value: formatBoolean(runtimeInfo.webGL2) },
    { label: "WebCodecs", value: formatBoolean(runtimeInfo.webCodecs) },
    { label: "WebTransport", value: formatBoolean(runtimeInfo.webTransport) },
  ];

  const gpuRenderer = runtimeInfo.gpuRenderer.trim();
  const gpuVendor = runtimeInfo.gpuVendor.trim();

  return (
    <section
      aria-label="Stream debug info"
      className={`debug-panel ${inline ? "debug-panel-inline" : "debug-panel-popover"}`}
    >
      <div className="debug-panel-header">Debug Info</div>
      <dl className="debug-grid">
        {rows.map((row) => (
          <div className="debug-row" key={row.label}>
            <dt className="debug-label">{row.label}</dt>
            <dd className="debug-value">{row.value}</dd>
          </div>
        ))}
        {gpuVendor ? (
          <div className="debug-row debug-row-wide">
            <dt className="debug-label">GPU Vendor</dt>
            <dd className="debug-value debug-value-wrap">{gpuVendor}</dd>
          </div>
        ) : null}
        {gpuRenderer ? (
          <div className="debug-row debug-row-wide">
            <dt className="debug-label">GPU Renderer</dt>
            <dd className="debug-value debug-value-wrap">{gpuRenderer}</dd>
          </div>
        ) : null}
        {status.detail ? (
          <div className="debug-row debug-row-wide">
            <dt className="debug-label">Detail</dt>
            <dd className="debug-value debug-value-wrap">{status.detail}</dd>
          </div>
        ) : null}
      </dl>
    </section>
  );
}
