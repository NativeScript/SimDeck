import type { RefObject } from "react";

import type { SimulatorMetadata } from "../../api/types";
import type {
  StreamRuntimeInfo,
  StreamStats,
  StreamStatus
} from "../stream/streamTypes";
import { DebugPanel } from "../toolbar/DebugPanel";
import { SimulatorRow } from "./SimulatorRow";

interface SimulatorMenuProps {
  debugVisible: boolean;
  filteredSimulators: SimulatorMetadata[];
  fps: number;
  isLoading: boolean;
  menuOpen: boolean;
  menuRef: RefObject<HTMLDivElement | null>;
  onChangeSearch: (value: string) => void;
  onCloseMenu: () => void;
  onOpenBundlePrompt: () => void;
  onOpenUrlPrompt: () => void;
  onToggleDebug: () => void;
  onToggleMenu: () => void;
  runtimeInfo: StreamRuntimeInfo;
  search: string;
  selectedSimulator: SimulatorMetadata | null;
  setSelectedUDID: (udid: string) => void;
  stats: StreamStats;
  status: StreamStatus;
}

export function SimulatorMenu({
  debugVisible,
  filteredSimulators,
  fps,
  isLoading,
  menuOpen,
  menuRef,
  onChangeSearch,
  onCloseMenu,
  onOpenBundlePrompt,
  onOpenUrlPrompt,
  onToggleDebug,
  onToggleMenu,
  runtimeInfo,
  search,
  selectedSimulator,
  setSelectedUDID,
  stats,
  status
}: SimulatorMenuProps) {
  return (
    <div className="menu-wrap" ref={menuRef}>
      <button
        className={`tbtn ${menuOpen ? "active" : ""}`}
        onClick={(event) => {
          event.stopPropagation();
          onToggleMenu();
        }}
        title="Open menu"
      >
        <MenuIcon />
      </button>
      {menuOpen ? (
        <div className="menu-popover" onPointerDown={(event) => event.stopPropagation()}>
          <input
            className="sidebar-search"
            onChange={(event) => onChangeSearch(event.target.value)}
            placeholder="Search simulators…"
            value={search}
          />
          <div className="sim-list">
            {isLoading ? <p className="list-empty">Loading…</p> : null}
            {!isLoading && filteredSimulators.length === 0 ? (
              <p className="list-empty">No matches</p>
            ) : null}
            {filteredSimulators.map((simulator) => (
              <SimulatorRow
                isSelected={simulator.udid === selectedSimulator?.udid}
                key={simulator.udid}
                onSelect={() => {
                  setSelectedUDID(simulator.udid);
                  onCloseMenu();
                }}
                simulator={simulator}
              />
            ))}
          </div>
          {selectedSimulator ? (
            <>
              <div className="menu-divider" />
              <div className="menu-actions">
                <button className="menu-action" onClick={onOpenUrlPrompt}>
                  Open URL…
                </button>
                <button className="menu-action" onClick={onOpenBundlePrompt}>
                  Launch Bundle…
                </button>
              </div>
            </>
          ) : null}
          <div className="menu-divider" />
          <div className="menu-actions">
            <button className="menu-action" onClick={onToggleDebug}>
              {debugVisible ? "Hide Debug Info" : "Show Debug Info"}
            </button>
            {debugVisible ? (
              <div className="menu-debug-panel">
                <DebugPanel
                  fps={fps}
                  inline
                  runtimeInfo={runtimeInfo}
                  stats={stats}
                  status={status}
                />
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function MenuIcon() {
  return (
    <svg fill="currentColor" height="16" viewBox="0 0 16 16" width="16">
      <path d="M2 3.5h12v1.5H2zm0 3.75h12v1.5H2zm0 3.75h12v1.5H2z" />
    </svg>
  );
}
