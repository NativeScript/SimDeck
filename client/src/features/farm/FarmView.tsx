import { useCallback, useMemo, useState } from "react";

import { accessTokenFromLocation } from "../../api/client";
import { apiUrl } from "../../api/config";
import { bootSimulator, shutdownSimulator } from "../../api/controls";
import type { SimulatorMetadata } from "../../api/types";
import { simulatorRuntimeLabel } from "../simulators/simulatorDisplay";
import { useSimulatorList } from "../simulators/useSimulatorList";
import { useLiveStream } from "../stream/useLiveStream";

type FarmViewMode = "grid" | "list" | "wall";

function simulatorFamily(simulator: SimulatorMetadata): string {
  const text = `${simulator.name} ${simulator.deviceTypeName ?? ""}`.toLowerCase();
  if (text.includes("ipad")) return "iPad";
  if (text.includes("watch")) return "Watch";
  if (text.includes("tv")) return "TV";
  return "iPhone";
}

function screenshotUrl(udid: string): string {
  const url = new URL(
    apiUrl(`/api/simulators/${encodeURIComponent(udid)}/screenshot.png`),
    window.location.href,
  );
  const token = accessTokenFromLocation();
  if (token) {
    url.searchParams.set("simdeckToken", token);
  }
  url.searchParams.set("stamp", String(Date.now()));
  return url.toString();
}

export function FarmView() {
  const { isLoading, refresh, simulators } = useSimulatorList();
  const [search, setSearch] = useState("");
  const [family, setFamily] = useState("all");
  const [state, setState] = useState("all");
  const [view, setView] = useState<FarmViewMode>("grid");
  const [selectedUDID, setSelectedUDID] = useState("");
  const [busyUDID, setBusyUDID] = useState("");

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return simulators
      .filter((simulator) => {
        if (family !== "all" && simulatorFamily(simulator) !== family) {
          return false;
        }
        if (state === "booted" && !simulator.isBooted) {
          return false;
        }
        if (state === "shutdown" && simulator.isBooted) {
          return false;
        }
        if (!needle) {
          return true;
        }
        return [
          simulator.name,
          simulator.udid,
          simulatorRuntimeLabel(simulator),
          simulator.deviceTypeName,
        ]
          .filter(Boolean)
          .some((value) => value!.toLowerCase().includes(needle));
      })
      .sort((a, b) => Number(b.isBooted) - Number(a.isBooted) || a.name.localeCompare(b.name));
  }, [family, search, simulators, state]);

  const selectedSimulator =
    filtered.find((simulator) => simulator.udid === selectedUDID) ??
    filtered.find((simulator) => simulator.isBooted) ??
    filtered[0] ??
    null;

  async function runLifecycle(
    simulator: SimulatorMetadata,
    action: "boot" | "shutdown",
  ) {
    setBusyUDID(simulator.udid);
    try {
      if (action === "boot") {
        await bootSimulator(simulator.udid);
        setSelectedUDID(simulator.udid);
      } else {
        await shutdownSimulator(simulator.udid);
      }
      await refresh();
    } finally {
      setBusyUDID("");
    }
  }

  return (
    <div className="farm-app">
      <header className="farm-header">
        <div className="farm-title">
          <strong>SimDeck Farm</strong>
          <span>
            {filtered.filter((simulator) => simulator.isBooted).length} live /{" "}
            {filtered.length} shown
          </span>
        </div>
        <div className="farm-controls">
          <input
            aria-label="Search simulators"
            className="farm-search"
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search simulators"
            value={search}
          />
          <select
            aria-label="Family"
            className="farm-select"
            onChange={(event) => setFamily(event.target.value)}
            value={family}
          >
            <option value="all">All families</option>
            <option value="iPhone">iPhone</option>
            <option value="iPad">iPad</option>
            <option value="Watch">Watch</option>
            <option value="TV">TV</option>
          </select>
          <select
            aria-label="State"
            className="farm-select"
            onChange={(event) => setState(event.target.value)}
            value={state}
          >
            <option value="all">All states</option>
            <option value="booted">Booted</option>
            <option value="shutdown">Shutdown</option>
          </select>
          <div className="farm-segments" role="group">
            {(["grid", "wall", "list"] as FarmViewMode[]).map((mode) => (
              <button
                className={view === mode ? "active" : ""}
                key={mode}
                onClick={() => setView(mode)}
              >
                {mode}
              </button>
            ))}
          </div>
          <a className="farm-link" href="/">
            Single
          </a>
        </div>
      </header>
      <main className={`farm-main farm-${view}`}>
        <section className="farm-tiles" aria-busy={isLoading}>
          {filtered.map((simulator) => (
            <FarmTile
              busy={busyUDID === simulator.udid}
              isFocused={simulator.udid === selectedSimulator?.udid}
              key={simulator.udid}
              onBoot={() => void runLifecycle(simulator, "boot")}
              onSelect={() => setSelectedUDID(simulator.udid)}
              onShutdown={() => void runLifecycle(simulator, "shutdown")}
              simulator={simulator}
              view={view}
            />
          ))}
          {!isLoading && filtered.length === 0 ? (
            <div className="farm-empty">No simulators match the current filters.</div>
          ) : null}
        </section>
        <aside className="farm-focus">
          {selectedSimulator ? (
            <FarmFocus simulator={selectedSimulator} />
          ) : (
            <div className="farm-empty">Select a simulator.</div>
          )}
        </aside>
      </main>
    </div>
  );
}

function FarmTile({
  busy,
  isFocused,
  onBoot,
  onSelect,
  onShutdown,
  simulator,
  view,
}: {
  busy: boolean;
  isFocused: boolean;
  onBoot: () => void;
  onSelect: () => void;
  onShutdown: () => void;
  simulator: SimulatorMetadata;
  view: FarmViewMode;
}) {
  const [canvas, setCanvas] = useState<HTMLCanvasElement | null>(null);
  const handleCanvasRef = useCallback((node: HTMLCanvasElement | null) => {
    setCanvas(node);
  }, []);
  const stream = useLiveStream({
    canvasElement: canvas,
    paused: !simulator.isBooted || isFocused,
    simulator,
    streamProfile: "thumb",
  });
  const runtime = simulatorRuntimeLabel(simulator);
  const frameStyle = simulator.isBooted && !stream.hasFrame
    ? { backgroundImage: `url("${screenshotUrl(simulator.udid)}")` }
    : undefined;

  return (
    <article
      className={`farm-tile ${isFocused ? "focused" : ""} ${view === "list" ? "list" : ""}`}
      onClick={onSelect}
    >
      <div className="farm-tile-screen" style={frameStyle}>
        {simulator.isBooted && !isFocused ? (
          <canvas
            aria-label={`${simulator.name} stream`}
            className="farm-canvas"
            key={simulator.udid}
            ref={handleCanvasRef}
          />
        ) : simulator.isBooted ? (
          <span>Focused</span>
        ) : (
          <span>Shutdown</span>
        )}
      </div>
      <div className="farm-tile-meta">
        <div>
          <strong>{simulator.name}</strong>
          <span>{runtime}</span>
        </div>
        <div className="farm-tile-stats">
          <span>{simulatorFamily(simulator)}</span>
          <span>{simulator.isBooted ? `${stream.fps.toFixed(0)} fps` : "off"}</span>
        </div>
      </div>
      <div className="farm-tile-actions">
        {simulator.isBooted ? (
          <button disabled={busy} onClick={(event) => { event.stopPropagation(); onShutdown(); }}>
            Shutdown
          </button>
        ) : (
          <button disabled={busy} onClick={(event) => { event.stopPropagation(); onBoot(); }}>
            Boot
          </button>
        )}
      </div>
    </article>
  );
}

function FarmFocus({ simulator }: { simulator: SimulatorMetadata }) {
  const [canvas, setCanvas] = useState<HTMLCanvasElement | null>(null);
  const handleCanvasRef = useCallback((node: HTMLCanvasElement | null) => {
    setCanvas(node);
  }, []);
  const stream = useLiveStream({
    canvasElement: canvas,
    paused: !simulator.isBooted,
    simulator,
    streamProfile: "focus",
  });

  return (
    <div className="farm-focus-inner">
      <div className="farm-focus-head">
        <div>
          <strong>{simulator.name}</strong>
          <span>{simulatorRuntimeLabel(simulator)}</span>
        </div>
        <div className="farm-focus-stats">
          <span>{stream.status.state}</span>
          <span>{stream.fps.toFixed(1)} fps</span>
        </div>
      </div>
      <div
        className="farm-focus-screen"
        style={
          simulator.isBooted && !stream.hasFrame
            ? { backgroundImage: `url("${screenshotUrl(simulator.udid)}")` }
            : undefined
        }
      >
        {simulator.isBooted ? (
          <canvas
            className="farm-focus-canvas"
            ref={handleCanvasRef}
          />
        ) : (
          <span>Boot this simulator to stream it.</span>
        )}
      </div>
      <dl className="farm-focus-detail">
        <div>
          <dt>UDID</dt>
          <dd>{simulator.udid}</dd>
        </div>
        <div>
          <dt>Stream</dt>
          <dd>{stream.stats.width && stream.stats.height ? `${stream.stats.width}x${stream.stats.height}` : "Waiting"}</dd>
        </div>
      </dl>
    </div>
  );
}
