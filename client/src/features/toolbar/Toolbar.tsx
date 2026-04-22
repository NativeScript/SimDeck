import type { RefObject } from "react";

import type { SimulatorMetadata } from "../../api/types";
import type {
  StreamRuntimeInfo,
  StreamStats,
  StreamStatus
} from "../stream/streamTypes";
import { SimulatorMenu } from "../simulators/SimulatorMenu";

interface ToolbarProps {
  debugVisible: boolean;
  error: string;
  filteredSimulators: SimulatorMetadata[];
  fps: number;
  isLoading: boolean;
  onBoot: () => void;
  onChangeSearch: (value: string) => void;
  onHome: () => void;
  onOpenBundlePrompt: () => void;
  onOpenUrlPrompt: () => void;
  onRotateRight: () => void;
  onShutdown: () => void;
  onToggleDebug: () => void;
  onToggleMenu: () => void;
  runtimeInfo: StreamRuntimeInfo;
  search: string;
  selectedSimulator: SimulatorMetadata | null;
  selectedSimulatorIdentifier: string;
  setSelectedUDID: (udid: string) => void;
  menuOpen: boolean;
  menuRef: RefObject<HTMLDivElement | null>;
  closeMenu: () => void;
  stats: StreamStats;
  status: StreamStatus;
}

export function Toolbar({
  closeMenu,
  debugVisible,
  error,
  filteredSimulators,
  fps,
  isLoading,
  menuOpen,
  menuRef,
  onBoot,
  onChangeSearch,
  onHome,
  onOpenBundlePrompt,
  onOpenUrlPrompt,
  onRotateRight,
  onShutdown,
  onToggleDebug,
  onToggleMenu,
  runtimeInfo,
  search,
  selectedSimulator,
  selectedSimulatorIdentifier,
  setSelectedUDID,
  stats,
  status
}: ToolbarProps) {
  return (
    <header className="toolbar">
      <div className="toolbar-left">
        <SimulatorMenu
          debugVisible={debugVisible}
          filteredSimulators={filteredSimulators}
          fps={fps}
          isLoading={isLoading}
          menuOpen={menuOpen}
          menuRef={menuRef}
          onChangeSearch={onChangeSearch}
          onCloseMenu={closeMenu}
          onOpenBundlePrompt={onOpenBundlePrompt}
          onOpenUrlPrompt={onOpenUrlPrompt}
          onToggleDebug={onToggleDebug}
          onToggleMenu={onToggleMenu}
          runtimeInfo={runtimeInfo}
          search={search}
          selectedSimulator={selectedSimulator}
          setSelectedUDID={setSelectedUDID}
          stats={stats}
          status={status}
        />
        {selectedSimulator ? (
          <div className="toolbar-sim-info">
            <div className="toolbar-sim-copy">
              <div className="toolbar-sim-title-row">
                <span className="toolbar-sim-name">{selectedSimulator.name}</span>
                {selectedSimulator.isBooted ? (
                  <span className="state-dot booted toolbar-status-dot" />
                ) : null}
              </div>
              <span className="toolbar-sim-detail">{selectedSimulatorIdentifier}</span>
            </div>
          </div>
        ) : (
          <span className="toolbar-sim-name muted">
            {isLoading ? "Loading…" : "No simulator selected"}
          </span>
        )}
      </div>

      <div className="toolbar-right">
        {selectedSimulator ? (
          <div className="toolbar-actions">
            <button
              aria-label="Boot"
              className="tbtn icon-btn accent"
              onClick={onBoot}
              title="Boot"
            >
              <PlayIcon />
            </button>
            <button
              aria-label="Stop"
              className="tbtn icon-btn"
              onClick={onShutdown}
              title="Stop"
            >
              <StopIcon />
            </button>
            <button
              aria-label="Home"
              className="tbtn icon-btn"
              onClick={onHome}
              title="Home"
            >
              <HomeIcon />
            </button>
            <button
              aria-label="Rotate Right"
              className="tbtn icon-btn"
              onClick={onRotateRight}
              title="Rotate Right"
            >
              <RotateRightIcon />
            </button>
          </div>
        ) : null}
        {error ? <span className="error-msg">{error}</span> : null}
      </div>
    </header>
  );
}

function PlayIcon() {
  return (
    <svg fill="currentColor" height="16" viewBox="0 0 16 16" width="16">
      <path d="M5 3.5v9l7-4.5z" />
    </svg>
  );
}

function StopIcon() {
  return (
    <svg fill="currentColor" height="16" viewBox="0 0 16 16" width="16">
      <path d="M4 4h8v8H4z" />
    </svg>
  );
}

function HomeIcon() {
  return (
    <svg fill="currentColor" height="16" viewBox="0 0 16 16" width="16">
      <path d="M8 3l5 4.2V13H9.6V9.6H6.4V13H3V7.2z" />
    </svg>
  );
}

function RotateRightIcon() {
  return (
    <svg fill="currentColor" height="16" viewBox="0 0 16 16" width="16">
      <path d="M10.75 1.75h1.75v3.5h-1.5V4.34a4.5 4.5 0 1 0 .8 5.66l1.23.87a6 6 0 1 1-1.28-7.12v-.99z" />
      <path d="M6.1 4.25h2.8a.6.6 0 0 1 .6.6v5.3a.6.6 0 0 1-.6.6H6.1a.6.6 0 0 1-.6-.6v-5.3a.6.6 0 0 1 .6-.6zm.9 1.1v4.3h1.5v-4.3H7z" />
    </svg>
  );
}
