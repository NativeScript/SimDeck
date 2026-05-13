import { useEffect, useMemo, useState } from "react";
import type { CSSProperties, PointerEvent } from "react";

import {
  fetchSimulatorPerformance,
  sampleSimulatorProcess,
} from "../../api/simulators";
import type {
  PerformanceProcess,
  PerformanceSample,
  SimulatorMetadata,
  SimulatorPerformanceResponse,
  StackSampleReport,
} from "../../api/types";

const PERFORMANCE_REFRESH_MS = 1500;
const PERFORMANCE_WINDOW_MS = 120_000;

interface PerformancePanelProps {
  selectedSimulator: SimulatorMetadata | null;
  visible: boolean;
}

export function PerformancePanel({
  selectedSimulator,
  visible,
}: PerformancePanelProps) {
  const udid = selectedSimulator?.udid ?? "";
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const [followForeground, setFollowForeground] = useState(true);
  const [performance, setPerformance] =
    useState<SimulatorPerformanceResponse | null>(null);
  const [error, setError] = useState("");
  const [sample, setSample] = useState<StackSampleReport | null>(null);
  const [sampling, setSampling] = useState(false);

  useEffect(() => {
    setSelectedPid(null);
    setFollowForeground(true);
    setPerformance(null);
    setSample(null);
    setError("");
  }, [udid]);

  useEffect(() => {
    if (!visible || !udid || !selectedSimulator?.isBooted) {
      return;
    }

    let cancelled = false;
    let timer: number | undefined;
    async function refresh() {
      try {
        const next = await fetchSimulatorPerformance(udid, {
          pid: followForeground ? null : selectedPid,
          windowMs: PERFORMANCE_WINDOW_MS,
        });
        if (cancelled) {
          return;
        }
        setPerformance(next);
        setError("");
        if (followForeground) {
          setSelectedPid(next.selectedPid ?? null);
        } else if (
          selectedPid != null &&
          !next.processes.some((process) => process.pid === selectedPid)
        ) {
          setFollowForeground(true);
          setSelectedPid(next.selectedPid ?? null);
        } else if (selectedPid == null && next.selectedPid != null) {
          setSelectedPid(next.selectedPid);
        }
      } catch (refreshError) {
        if (!cancelled) {
          setError(errorMessage(refreshError));
        }
      } finally {
        if (!cancelled) {
          timer = window.setTimeout(refresh, PERFORMANCE_REFRESH_MS);
        }
      }
    }

    void refresh();
    return () => {
      cancelled = true;
      if (timer != null) {
        window.clearTimeout(timer);
      }
    };
  }, [followForeground, selectedPid, selectedSimulator?.isBooted, udid, visible]);

  const current = performance?.current ?? null;
  const selectedProcess = useMemo(
    () =>
      performance?.processes.find(
        (process) => process.pid === (selectedPid ?? performance.selectedPid),
      ) ?? null,
    [performance, selectedPid],
  );

  async function runSample() {
    const pid = selectedPid ?? performance?.selectedPid ?? null;
    if (!udid || pid == null) {
      return;
    }
    setSampling(true);
    setSample(null);
    setError("");
    try {
      const response = await sampleSimulatorProcess(udid, pid, 3);
      setSample(response.sample);
    } catch (sampleError) {
      setError(errorMessage(sampleError));
    } finally {
      setSampling(false);
    }
  }

  if (!selectedSimulator) {
    return <div className="performance-empty">Select a simulator.</div>;
  }
  if (!selectedSimulator.isBooted) {
    return <div className="performance-empty">Boot the simulator.</div>;
  }

  return (
    <div className="performance-panel">
      <div className="performance-process-list">
        {performance?.processes.length ? (
          performance.processes.map((process) => (
            <ProcessButton
              key={process.pid}
              onSelect={() => {
                setSelectedPid(process.pid);
                setFollowForeground(process.isForeground);
                setSample(null);
              }}
              process={process}
              selected={
                process.pid === (selectedPid ?? performance.selectedPid)
              }
            />
          ))
        ) : (
          <div className="performance-empty compact">
            {error || "Waiting for an app process."}
          </div>
        )}
      </div>

      {error && performance?.processes.length ? (
        <div className="performance-error">{error}</div>
      ) : null}

      {current ? (
        <>
          <div className="performance-summary">
            <Metric label="CPU" value={formatPercent(current.cpuPercent)} />
            <Metric
              label="Memory"
              value={formatBytes(memoryDisplayBytes(current))}
            />
            <Metric
              label="Peak"
              value={formatBytes(current.memoryPeakFootprintBytes)}
            />
            <Metric
              label="Disk"
              value={formatRate(current.diskWriteBytesPerSecond)}
            />
            <Metric
              label="Down"
              value={formatRate(current.networkReceivedBytesPerSecond)}
            />
            <Metric
              label="Up"
              value={formatRate(current.networkSentBytesPerSecond)}
            />
          </div>

          <div className={`performance-hang state-${current.hang.state}`}>
            <span>{hangLabel(current.hang.state)}</span>
            <span>{current.hang.reason}</span>
          </div>

          <Timeline
            label="Memory"
            samples={performance?.history ?? []}
            value={(sample) => memoryDisplayBytes(sample) ?? 0}
            valueLabel={formatBytes}
          />
          <Timeline
            label="CPU"
            samples={performance?.history ?? []}
            value={(sample) => sample.cpuPercent}
            valueLabel={formatPercent}
          />
          <Timeline
            label="Disk Writes"
            samples={performance?.history ?? []}
            value={(sample) => sample.diskWriteBytesPerSecond ?? 0}
            valueLabel={formatRate}
          />
          <Timeline
            label="Network Down"
            samples={performance?.history ?? []}
            value={(sample) => sample.networkReceivedBytesPerSecond ?? 0}
            valueLabel={formatRate}
          />
          <Timeline
            label="Network Up"
            samples={performance?.history ?? []}
            value={(sample) => sample.networkSentBytesPerSecond ?? 0}
            valueLabel={formatRate}
          />

          <section className="performance-section">
            <div className="performance-section-title">Network</div>
            <div className="performance-network-line">
              <span>
                {formatRate(current.networkReceivedBytesPerSecond)} down /{" "}
                {formatRate(current.networkSentBytesPerSecond)} up
              </span>
              <span>
                {formatBytes(current.networkReceivedBytes)} received /{" "}
                {formatBytes(current.networkSentBytes)} sent
              </span>
              <span>
                {current.networkConnectionCount == null
                  ? "Connection details unavailable"
                  : `${current.networkConnectionCount} connections, ${current.networkEstablishedConnectionCount ?? 0} established`}
              </span>
            </div>
            {current.networkEndpoints.length ? (
              <div className="performance-endpoints">
                {current.networkEndpoints.map((endpoint) => (
                  <div key={endpoint}>{endpoint}</div>
                ))}
              </div>
            ) : null}
          </section>

          <section className="performance-section">
            <div className="performance-section-header">
              <div>
                <div className="performance-section-title">CPU Sample</div>
                <div className="performance-section-subtitle">
                  {selectedProcess
                    ? `${selectedProcess.process} (${selectedProcess.pid})`
                    : `PID ${current.pid}`}
                </div>
              </div>
              <button
                className="performance-sample-button"
                disabled={sampling}
                onClick={runSample}
                type="button"
              >
                {sampling ? "Sampling" : "Sample"}
              </button>
            </div>
            {sample ? (
              <pre className="performance-sample-report">
                {sample.report || sample.stderr}
              </pre>
            ) : null}
          </section>

          <section className="performance-section">
            <div className="performance-section-title">Crashes</div>
            {performance?.events.length ? (
              <div className="performance-events">
                {performance.events.map((event, index) => (
                  <div
                    className="performance-event"
                    key={`${event.timestamp}-${index}`}
                  >
                    <span>{event.level}</span>
                    <span>{event.message}</span>
                  </div>
                ))}
              </div>
            ) : (
              <div className="performance-muted">
                No recent crash or termination signals.
              </div>
            )}
          </section>
        </>
      ) : null}
    </div>
  );
}

function ProcessButton({
  onSelect,
  process,
  selected,
}: {
  onSelect: () => void;
  process: PerformanceProcess;
  selected: boolean;
}) {
  return (
    <button
      className={`performance-process ${selected ? "selected" : ""}`}
      onClick={onSelect}
      title={process.command}
      type="button"
    >
      <span className="performance-process-name">{process.process}</span>
      <span className="performance-process-meta">
        {process.pid} / {process.role}
        {process.isForeground ? " / frontmost" : ""}
      </span>
    </button>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="performance-metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function Timeline({
  label,
  samples,
  value,
  valueLabel,
}: {
  label: string;
  samples: PerformanceSample[];
  value: (sample: PerformanceSample) => number;
  valueLabel: (value: number | null | undefined) => string;
}) {
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const values = samples.map(value);
  const latest = values.at(-1) ?? 0;
  const max = Math.max(...values, 1);
  const coordinates = values.map((item, index) => ({
    x: values.length <= 1 ? 0 : (index / (values.length - 1)) * 100,
    y: 42 - (Math.max(0, item) / max) * 36,
  }));
  const points = coordinates
    .map((point) => `${round(point.x)},${round(point.y)}`)
    .join(" ");
  const activeIndex =
    hoverIndex == null || hoverIndex >= samples.length ? null : hoverIndex;
  const activePoint = activeIndex == null ? null : coordinates[activeIndex];
  const activeSample = activeIndex == null ? null : samples[activeIndex];

  function handlePointerMove(event: PointerEvent<SVGSVGElement>) {
    if (samples.length === 0) {
      setHoverIndex(null);
      return;
    }
    const rect = event.currentTarget.getBoundingClientRect();
    const position = clamp((event.clientX - rect.left) / rect.width, 0, 1);
    setHoverIndex(Math.round(position * (samples.length - 1)));
  }

  return (
    <section className="performance-chart">
      <div className="performance-chart-head">
        <span>{label}</span>
        <strong>{valueLabel(latest)}</strong>
      </div>
      <svg
        aria-label={`${label} timeline`}
        className="performance-chart-svg"
        onPointerLeave={() => setHoverIndex(null)}
        onPointerMove={handlePointerMove}
        preserveAspectRatio="none"
        viewBox="0 0 100 44"
      >
        <line x1="0" x2="100" y1="42" y2="42" />
        {points ? <polyline points={points} /> : null}
        {activePoint ? (
          <>
            <line
              className="performance-chart-marker"
              x1={activePoint.x}
              x2={activePoint.x}
              y1="4"
              y2="42"
            />
            <circle
              className="performance-chart-point active"
              cx={activePoint.x}
              cy={activePoint.y}
              r="2.4"
            />
          </>
        ) : null}
      </svg>
      {activePoint && activeSample ? (
        <div
          className="performance-chart-tooltip"
          style={
            {
              "--performance-tooltip-x": `${activePoint.x}%`,
            } as CSSProperties
          }
        >
          <span>{formatSampleTime(activeSample.timestampMs)}</span>
          <strong>{valueLabel(value(activeSample))}</strong>
        </div>
      ) : null}
    </section>
  );
}

function memoryDisplayBytes(sample: PerformanceSample): number | null {
  return sample.memoryFootprintBytes ?? sample.memoryResidentBytes ?? null;
}

function formatPercent(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) {
    return "--";
  }
  return `${Math.round(value * 10) / 10}%`;
}

function formatBytes(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) {
    return "--";
  }
  const units = ["B", "KB", "MB", "GB"];
  let next = value;
  let unit = 0;
  while (next >= 1024 && unit < units.length - 1) {
    next /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? Math.round(next) : Math.round(next * 10) / 10} ${units[unit]}`;
}

function formatRate(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) {
    return "--";
  }
  return `${formatBytes(value)}/s`;
}

function hangLabel(state: string): string {
  if (state === "busy") {
    return "Busy";
  }
  if (state === "quiet") {
    return "Quiet";
  }
  if (state === "responsive") {
    return "Responsive";
  }
  return "Unknown";
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function formatSampleTime(timestampMs: number): string {
  if (!Number.isFinite(timestampMs) || timestampMs <= 0) {
    return "";
  }
  return new Date(timestampMs).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
