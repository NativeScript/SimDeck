import {
  BoxModelIcon as DevToolsIcon,
  CardStackIcon as AppSwitcherIcon,
  Half2Icon as AppearanceIcon,
  HomeIcon,
  LayersIcon as HierarchyIcon,
  Link2Icon as OpenUrlIcon,
  PlayIcon,
  RotateCounterClockwiseIcon as RotateLeftIcon,
  StopIcon,
} from "@radix-ui/react-icons";
import { useEffect, useState, type RefObject } from "react";

import type { SimulatorMetadata } from "../../api/types";
import type { AccessibilitySkeletonMode } from "../accessibility/skeletonMode";
import type {
  StreamConfig,
  StreamEncoder,
  StreamFps,
  StreamQualityPreset,
  StreamTransport,
} from "../stream/streamTypes";
import { simulatorHasFixedOrientation } from "../simulators/simulatorDisplay";
import { SimulatorMenu } from "../simulators/SimulatorMenu";
import { SimulatorPickerMenu } from "../simulators/SimulatorPickerMenu";

interface ToolbarProps {
  accessibilitySkeletonMode: AccessibilitySkeletonMode;
  debugVisible: boolean;
  deviceChromeAvailable: boolean;
  deviceChromeVisible: boolean;
  devToolsVisible: boolean;
  error: string;
  filteredSimulators: SimulatorMetadata[];
  hierarchyVisible: boolean;
  hideSimulatorSelection?: boolean;
  isLoading: boolean;
  canInstallApp: boolean;
  onBoot: () => void;
  onCaptureScreenshot: () => void;
  onCaptureScreenshotWithBezel: () => void;
  onChangeSearch: (value: string) => void;
  onDismissKeyboard: () => void;
  onHome: () => void;
  onInstallAppPrompt: () => void;
  onOpenCameraSimulation: () => void;
  onOpenAppSwitcher: () => void;
  onOpenBundlePrompt: () => void;
  onOpenNewSimulator: () => void;
  onOpenUrlPrompt: () => void;
  onRotateLeft: () => void;
  onRotateRight: () => void;
  onAccessibilitySkeletonModeChange: (mode: AccessibilitySkeletonMode) => void;
  onShutdown: () => void;
  onStreamEncoderChange: (encoder: StreamEncoder) => void;
  onStreamFpsChange: (fps: StreamFps) => void;
  onStreamQualityChange: (quality: StreamQualityPreset) => void;
  onStreamTransportChange: (transport: StreamTransport) => void;
  onToggleAppearance: () => void;
  onToggleDebug: () => void;
  onToggleDeviceChrome: () => void;
  onToggleDevTools: () => void;
  onToggleHierarchy: () => void;
  onToggleMenu: () => void;
  onToggleRecording: () => void;
  onToggleSimulatorMenu: () => void;
  onToggleSoftwareKeyboard: () => void;
  onToggleTouchOverlay: () => void;
  captureBusy: boolean;
  recordingActive: boolean;
  recordingStopping: boolean;
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
  simulatorMenuOpen: boolean;
  simulatorMenuRef: RefObject<HTMLDivElement | null>;
  closeSimulatorMenu: () => void;
}

export function Toolbar({
  accessibilitySkeletonMode,
  captureBusy,
  closeSimulatorMenu,
  closeMenu,
  debugVisible,
  deviceChromeAvailable,
  deviceChromeVisible,
  devToolsVisible,
  error,
  filteredSimulators,
  hierarchyVisible,
  hideSimulatorSelection = false,
  isLoading,
  canInstallApp,
  menuOpen,
  menuRef,
  onBoot,
  onCaptureScreenshot,
  onCaptureScreenshotWithBezel,
  onChangeSearch,
  onDismissKeyboard,
  onHome,
  onInstallAppPrompt,
  onOpenCameraSimulation,
  onOpenAppSwitcher,
  onOpenBundlePrompt,
  onOpenNewSimulator,
  onOpenUrlPrompt,
  onRotateLeft,
  onRotateRight,
  onAccessibilitySkeletonModeChange,
  onShutdown,
  onStreamEncoderChange,
  onStreamFpsChange,
  onStreamQualityChange,
  onStreamTransportChange,
  onToggleAppearance,
  onToggleDebug,
  onToggleDeviceChrome,
  onToggleDevTools,
  onToggleHierarchy,
  onToggleMenu,
  onToggleRecording,
  onToggleSimulatorMenu,
  onToggleSoftwareKeyboard,
  onToggleTouchOverlay,
  recordingActive,
  recordingStopping,
  remoteStream = false,
  search,
  selectedSimulator,
  selectedSimulatorIdentifier,
  setSelectedUDID,
  showBootButton,
  showStopButton,
  streamConfig,
  streamTransport,
  simulatorMenuOpen,
  simulatorMenuRef,
  touchOverlayVisible,
}: ToolbarProps) {
  const [errorCopied, setErrorCopied] = useState(false);
  const canRotateSelectedSimulator =
    selectedSimulator != null &&
    !simulatorHasFixedOrientation(selectedSimulator);

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
          captureBusy={captureBusy}
          debugVisible={debugVisible}
          menuOpen={menuOpen}
          menuRef={menuRef}
          onBoot={onBoot}
          onCaptureScreenshot={onCaptureScreenshot}
          onCaptureScreenshotWithBezel={onCaptureScreenshotWithBezel}
          onCloseMenu={closeMenu}
          onDismissKeyboard={onDismissKeyboard}
          onHome={onHome}
          onInstallAppPrompt={onInstallAppPrompt}
          onOpenCameraSimulation={onOpenCameraSimulation}
          onOpenAppSwitcher={onOpenAppSwitcher}
          onOpenBundlePrompt={onOpenBundlePrompt}
          onOpenUrlPrompt={onOpenUrlPrompt}
          onRotateRight={onRotateRight}
          onAccessibilitySkeletonModeChange={onAccessibilitySkeletonModeChange}
          onShutdown={onShutdown}
          onStreamEncoderChange={onStreamEncoderChange}
          onStreamFpsChange={onStreamFpsChange}
          onStreamQualityChange={onStreamQualityChange}
          onStreamTransportChange={onStreamTransportChange}
          onToggleAppearance={onToggleAppearance}
          onToggleDebug={onToggleDebug}
          onToggleDeviceChrome={onToggleDeviceChrome}
          onToggleMenu={onToggleMenu}
          onToggleRecording={onToggleRecording}
          onToggleSoftwareKeyboard={onToggleSoftwareKeyboard}
          onToggleTouchOverlay={onToggleTouchOverlay}
          recordingActive={recordingActive}
          recordingStopping={recordingStopping}
          remoteStream={remoteStream}
          selectedSimulator={selectedSimulator}
          showBootButton={showBootButton}
          showStopButton={showStopButton}
          canInstallApp={canInstallApp}
          streamConfig={streamConfig}
          streamTransport={streamTransport}
          accessibilitySkeletonMode={accessibilitySkeletonMode}
          deviceChromeAvailable={deviceChromeAvailable}
          deviceChromeVisible={deviceChromeVisible}
          touchOverlayVisible={touchOverlayVisible}
        />
        <SimulatorPickerMenu
          filteredSimulators={filteredSimulators}
          hideSimulatorSelection={hideSimulatorSelection}
          isLoading={isLoading}
          menuOpen={simulatorMenuOpen}
          menuRef={simulatorMenuRef}
          onChangeSearch={onChangeSearch}
          onCloseMenu={closeSimulatorMenu}
          onOpenNewSimulator={onOpenNewSimulator}
          onToggleMenu={onToggleSimulatorMenu}
          search={search}
          selectedSimulator={selectedSimulator}
          selectedSimulatorIdentifier={selectedSimulatorIdentifier}
          setSelectedUDID={setSelectedUDID}
        />
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
              aria-label="Open URL"
              className="tbtn icon-btn toolbar-mobile-hidden"
              onClick={onOpenUrlPrompt}
              title="Open URL"
            >
              <OpenUrlIcon />
            </button>
            <button
              aria-label="Home"
              className="tbtn icon-btn toolbar-mobile-hidden"
              onClick={onHome}
              title="Home"
            >
              <HomeIcon />
            </button>
            <button
              aria-label="App Switcher"
              className="tbtn icon-btn toolbar-mobile-hidden"
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
            {canRotateSelectedSimulator ? (
              <>
                <button
                  aria-label="Rotate Left"
                  className="tbtn icon-btn toolbar-mobile-hidden toolbar-wide-hidden"
                  onClick={onRotateLeft}
                  title="Rotate Left"
                >
                  <RotateLeftIcon />
                </button>
                <button
                  aria-label="Rotate Right"
                  className="tbtn icon-btn toolbar-mobile-hidden"
                  onClick={onRotateRight}
                  title="Rotate Right"
                >
                  <RotateLeftIcon className="rotate-right-icon" />
                </button>
              </>
            ) : null}
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
