import { useEffect, useState, type RefObject } from "react";

import type { SimulatorMetadata, VideoCodecMode } from "../../api/types";
import type { StreamTransportMode } from "../stream/streamWorkerClient";
import { SimulatorMenu } from "../simulators/SimulatorMenu";

interface ToolbarProps {
  debugVisible: boolean;
  error: string;
  filteredSimulators: SimulatorMetadata[];
  hierarchyVisible: boolean;
  isLoading: boolean;
  onBoot: () => void;
  onChangeSearch: (value: string) => void;
  onDismissKeyboard: () => void;
  onHome: () => void;
  onOpenAppSwitcher: () => void;
  onOpenBundlePrompt: () => void;
  onOpenUrlPrompt: () => void;
  onRotateLeft: () => void;
  onRotateRight: () => void;
  onShutdown: () => void;
  onToggleAppearance: () => void;
  onToggleDebug: () => void;
  onToggleHierarchy: () => void;
  onToggleMenu: () => void;
  onToggleTouchOverlay: () => void;
  search: string;
  selectedSimulator: SimulatorMetadata | null;
  selectedSimulatorIdentifier: string;
  setSelectedUDID: (udid: string) => void;
  touchOverlayVisible: boolean;
  menuOpen: boolean;
  menuRef: RefObject<HTMLDivElement | null>;
  closeMenu: () => void;
  onChangeTransportMode: (mode: StreamTransportMode) => void;
  onChangeVideoCodec: (codec: VideoCodecMode) => void;
  streamTransportMode: StreamTransportMode;
  videoCodec: VideoCodecMode;
}

export function Toolbar({
  closeMenu,
  debugVisible,
  error,
  filteredSimulators,
  hierarchyVisible,
  isLoading,
  menuOpen,
  menuRef,
  onBoot,
  onChangeSearch,
  onChangeTransportMode,
  onChangeVideoCodec,
  onDismissKeyboard,
  onHome,
  onOpenAppSwitcher,
  onOpenBundlePrompt,
  onOpenUrlPrompt,
  onRotateLeft,
  onRotateRight,
  onShutdown,
  onToggleAppearance,
  onToggleDebug,
  onToggleHierarchy,
  onToggleMenu,
  onToggleTouchOverlay,
  search,
  selectedSimulator,
  selectedSimulatorIdentifier,
  setSelectedUDID,
  streamTransportMode,
  touchOverlayVisible,
  videoCodec,
}: ToolbarProps) {
  const [errorCopied, setErrorCopied] = useState(false);

  useEffect(() => {
    setErrorCopied(false);
  }, [error]);

  async function copyError() {
    if (!error) {
      return;
    }
    try {
      await navigator.clipboard.writeText(error);
      setErrorCopied(true);
      window.setTimeout(() => setErrorCopied(false), 1200);
    } catch {
      setErrorCopied(false);
    }
  }

  return (
    <header className="toolbar">
      <div className="toolbar-left">
        <button
          aria-label="Toggle View Hierarchy"
          className={`tbtn icon-btn ${hierarchyVisible ? "active" : ""}`}
          onClick={onToggleHierarchy}
          title="Toggle View Hierarchy"
          type="button"
        >
          <HierarchyIcon />
        </button>
        <SimulatorMenu
          debugVisible={debugVisible}
          filteredSimulators={filteredSimulators}
          isLoading={isLoading}
          menuOpen={menuOpen}
          menuRef={menuRef}
          onChangeTransportMode={onChangeTransportMode}
          onChangeVideoCodec={onChangeVideoCodec}
          onChangeSearch={onChangeSearch}
          onCloseMenu={closeMenu}
          onOpenBundlePrompt={onOpenBundlePrompt}
          onOpenUrlPrompt={onOpenUrlPrompt}
          onToggleDebug={onToggleDebug}
          onToggleMenu={onToggleMenu}
          search={search}
          selectedSimulator={selectedSimulator}
          setSelectedUDID={setSelectedUDID}
          streamTransportMode={streamTransportMode}
          videoCodec={videoCodec}
        />
        {selectedSimulator ? (
          <div className="toolbar-sim-info">
            <div className="toolbar-sim-copy">
              <div className="toolbar-sim-title-row">
                <span className="toolbar-sim-name">
                  {selectedSimulator.name}
                </span>
                {selectedSimulator.isBooted ? (
                  <span className="state-dot booted toolbar-status-dot" />
                ) : null}
              </div>
              <span className="toolbar-sim-detail">
                {selectedSimulatorIdentifier}
              </span>
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
              aria-label="App Switcher"
              className="tbtn icon-btn"
              onClick={onOpenAppSwitcher}
              title="App Switcher"
            >
              <AppSwitcherIcon />
            </button>
            <button
              aria-label="Dismiss Keyboard"
              className="tbtn icon-btn"
              onClick={onDismissKeyboard}
              title="Dismiss Keyboard"
            >
              <KeyboardDismissIcon />
            </button>
            <button
              aria-label="Toggle Touch Overlay"
              className={`tbtn icon-btn ${touchOverlayVisible ? "active" : ""}`}
              onClick={onToggleTouchOverlay}
              title="Toggle Touch Overlay"
            >
              <TouchOverlayIcon />
            </button>
            <button
              aria-label="Toggle Appearance"
              className="tbtn icon-btn"
              onClick={onToggleAppearance}
              title="Toggle Appearance"
            >
              <AppearanceIcon />
            </button>
            <button
              aria-label="Rotate Left"
              className="tbtn icon-btn"
              onClick={onRotateLeft}
              title="Rotate Left"
            >
              <RotateLeftIcon />
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
        {error ? (
          <button
            className={`error-msg ${errorCopied ? "copied" : ""}`}
            onClick={copyError}
            title={errorCopied ? "Copied" : "Copy error"}
            type="button"
          >
            {errorCopied ? "Copied" : error}
          </button>
        ) : null}
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

function AppSwitcherIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <rect
        height="8.5"
        rx="1.6"
        stroke="currentColor"
        strokeWidth="1.25"
        width="6.5"
        x="2.25"
        y="4.25"
      />
      <path
        d="M7.25 3.25h4.1c.9 0 1.65.74 1.65 1.65v6.35"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="1.25"
      />
    </svg>
  );
}

function KeyboardDismissIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <rect
        height="6"
        rx="1.2"
        stroke="currentColor"
        strokeWidth="1.25"
        width="11"
        x="2.5"
        y="3"
      />
      <path
        d="M4.5 5.2h.1M6.8 5.2h.1M9.1 5.2h.1M11.4 5.2h.1M4.5 7h7"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="1.25"
      />
      <path
        d="m5.5 11 2.5 2.5 2.5-2.5M8 9.7v3.4"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.25"
      />
    </svg>
  );
}

function TouchOverlayIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <circle cx="8" cy="8" r="2.1" fill="currentColor" />
      <circle cx="8" cy="8" r="5.1" stroke="currentColor" strokeWidth="1.2" />
    </svg>
  );
}

function AppearanceIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M8 2.2v11.6M8 13.8a5.8 5.8 0 0 0 0-11.6 5.8 5.8 0 0 0 0 11.6z"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="1.35"
      />
      <path d="M8 3.5a4.5 4.5 0 0 1 0 9z" fill="currentColor" />
    </svg>
  );
}

function RotateLeftIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M5.2 4H3.3V2.1M3.5 4A5.4 5.4 0 1 1 2.9 10"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.45"
      />
    </svg>
  );
}

function RotateRightIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M10.8 4h1.9V2.1M12.5 4a5.4 5.4 0 1 0 .6 6"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.45"
      />
    </svg>
  );
}

function HierarchyIcon() {
  return (
    <svg fill="none" height="16" viewBox="0 0 16 16" width="16">
      <path
        d="M3 2.5h4v3H3zM9 2.5h4v3H9zM3 10.5h4v3H3zM9 10.5h4v3H9zM5 5.5v2.25h6V5.5M5 10.5V7.75M11 10.5V7.75"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.35"
      />
    </svg>
  );
}
