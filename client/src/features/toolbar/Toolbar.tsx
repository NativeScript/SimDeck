import {
  BoxModelIcon as DevToolsIcon,
  CardStackIcon as AppSwitcherIcon,
  Half2Icon as AppearanceIcon,
  HomeIcon,
  LayersIcon as HierarchyIcon,
  PlayIcon,
  ReloadIcon as RotateRightIcon,
  StopIcon,
} from "@radix-ui/react-icons";
import { useEffect, useState, type RefObject } from "react";

import type { SimulatorMetadata } from "../../api/types";
import type {
  StreamConfig,
  StreamEncoder,
  StreamFps,
  StreamQualityPreset,
  StreamTransport,
} from "../stream/streamTypes";
import { SimulatorMenu } from "../simulators/SimulatorMenu";

interface ToolbarProps {
  debugVisible: boolean;
  devToolsVisible: boolean;
  error: string;
  filteredSimulators: SimulatorMetadata[];
  hierarchyVisible: boolean;
  hideSimulatorSelection?: boolean;
  isLoading: boolean;
  onBoot: () => void;
  onChangeSearch: (value: string) => void;
  onDismissKeyboard: () => void;
  onHome: () => void;
  onOpenAppSwitcher: () => void;
  onOpenBundlePrompt: () => void;
  onOpenUrlPrompt: () => void;
  onRotateRight: () => void;
  onShutdown: () => void;
  onStreamEncoderChange: (encoder: StreamEncoder) => void;
  onStreamFpsChange: (fps: StreamFps) => void;
  onStreamQualityChange: (quality: StreamQualityPreset) => void;
  onStreamTransportChange: (transport: StreamTransport) => void;
  onToggleAppearance: () => void;
  onToggleDebug: () => void;
  onToggleDevTools: () => void;
  onToggleHierarchy: () => void;
  onToggleMenu: () => void;
  onToggleTouchOverlay: () => void;
  remoteStream?: boolean;
  search: string;
  selectedSimulator: SimulatorMetadata | null;
  selectedSimulatorIdentifier: string;
  setSelectedUDID: (udid: string) => void;
  showBootButton: boolean;
  showStopButton: boolean;
  streamConfig: StreamConfig;
  streamTransport: StreamTransport;
  touchOverlayVisible: boolean;
  menuOpen: boolean;
  menuRef: RefObject<HTMLDivElement | null>;
  closeMenu: () => void;
}

export function Toolbar({
  closeMenu,
  debugVisible,
  devToolsVisible,
  error,
  filteredSimulators,
  hierarchyVisible,
  hideSimulatorSelection = false,
  isLoading,
  menuOpen,
  menuRef,
  onBoot,
  onChangeSearch,
  onDismissKeyboard,
  onHome,
  onOpenAppSwitcher,
  onOpenBundlePrompt,
  onOpenUrlPrompt,
  onRotateRight,
  onShutdown,
  onStreamEncoderChange,
  onStreamFpsChange,
  onStreamQualityChange,
  onStreamTransportChange,
  onToggleAppearance,
  onToggleDebug,
  onToggleDevTools,
  onToggleHierarchy,
  onToggleMenu,
  onToggleTouchOverlay,
  remoteStream = false,
  search,
  selectedSimulator,
  selectedSimulatorIdentifier,
  setSelectedUDID,
  showBootButton,
  showStopButton,
  streamConfig,
  streamTransport,
  touchOverlayVisible,
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
          hideSimulatorSelection={hideSimulatorSelection}
          isLoading={isLoading}
          menuOpen={menuOpen}
          menuRef={menuRef}
          onChangeSearch={onChangeSearch}
          onCloseMenu={closeMenu}
          onDismissKeyboard={onDismissKeyboard}
          onOpenBundlePrompt={onOpenBundlePrompt}
          onOpenUrlPrompt={onOpenUrlPrompt}
          onStreamEncoderChange={onStreamEncoderChange}
          onStreamFpsChange={onStreamFpsChange}
          onStreamQualityChange={onStreamQualityChange}
          onStreamTransportChange={onStreamTransportChange}
          onToggleAppearance={onToggleAppearance}
          onToggleDebug={onToggleDebug}
          onToggleMenu={onToggleMenu}
          onToggleTouchOverlay={onToggleTouchOverlay}
          remoteStream={remoteStream}
          search={search}
          selectedSimulator={selectedSimulator}
          setSelectedUDID={setSelectedUDID}
          streamConfig={streamConfig}
          streamTransport={streamTransport}
          touchOverlayVisible={touchOverlayVisible}
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
            {showBootButton ? (
              <button
                aria-label="Boot"
                className="tbtn icon-btn accent"
                onClick={onBoot}
                title="Boot"
              >
                <PlayIcon />
              </button>
            ) : null}
            {showStopButton ? (
              <button
                aria-label="Stop"
                className="tbtn icon-btn"
                onClick={onShutdown}
                title="Stop"
              >
                <StopIcon />
              </button>
            ) : null}
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
              aria-label="Toggle Appearance"
              className="tbtn icon-btn toolbar-mobile-hidden"
              onClick={onToggleAppearance}
              title="Toggle Appearance"
            >
              <AppearanceIcon />
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
        <button
          aria-label="Toggle DevTools"
          className={`tbtn icon-btn ${devToolsVisible ? "active" : ""}`}
          onClick={onToggleDevTools}
          title="Toggle DevTools"
          type="button"
        >
          <DevToolsIcon />
        </button>
      </div>
    </header>
  );
}
