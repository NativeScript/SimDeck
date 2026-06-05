import { MixerHorizontalIcon as MenuIcon } from "@radix-ui/react-icons";
import type { RefObject } from "react";

import type { SimulatorMetadata } from "../../api/types";
import type {
  StreamConfig,
  StreamEncoder,
  StreamFps,
  StreamQualityPreset,
  StreamTransport,
} from "../stream/streamTypes";
import { simulatorHasFixedOrientation } from "./simulatorDisplay";

interface SimulatorMenuProps {
  captureBusy: boolean;
  debugVisible: boolean;
  canInstallApp: boolean;
  menuOpen: boolean;
  menuRef: RefObject<HTMLDivElement | null>;
  onBoot: () => void;
  onCaptureScreenshot: () => void;
  onCaptureScreenshotWithBezel: () => void;
  onCloseMenu: () => void;
  onDismissKeyboard: () => void;
  onHome: () => void;
  onInstallAppPrompt: () => void;
  onOpenCameraSimulation: () => void;
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
  onToggleMenu: () => void;
  onToggleRecording: () => void;
  onToggleSoftwareKeyboard: () => void;
  onToggleTouchOverlay: () => void;
  recordingActive: boolean;
  recordingStopping: boolean;
  remoteStream?: boolean;
  selectedSimulator: SimulatorMetadata | null;
  showBootButton: boolean;
  showStopButton: boolean;
  streamConfig: StreamConfig;
  streamTransport: StreamTransport;
  touchOverlayVisible: boolean;
}

export function SimulatorMenu({
  captureBusy,
  debugVisible,
  canInstallApp,
  menuOpen,
  menuRef,
  onBoot,
  onCaptureScreenshot,
  onCaptureScreenshotWithBezel,
  onCloseMenu,
  onDismissKeyboard,
  onHome,
  onInstallAppPrompt,
  onOpenCameraSimulation,
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
  onToggleMenu,
  onToggleRecording,
  onToggleSoftwareKeyboard,
  onToggleTouchOverlay,
  recordingActive,
  recordingStopping,
  remoteStream = false,
  selectedSimulator,
  showBootButton,
  showStopButton,
  streamConfig,
  streamTransport,
  touchOverlayVisible,
}: SimulatorMenuProps) {
  const fpsOptions = remoteStream
    ? REMOTE_STREAM_FPS_OPTIONS
    : LOCAL_STREAM_FPS_OPTIONS;
  const qualityOptions = STREAM_QUALITY_OPTIONS;
  const activeQualityOption = qualityOptions.some(
    (option) => option.value === streamConfig.quality,
  )
    ? []
    : [
        {
          label: streamQualityOptionLabel(streamConfig.quality),
          value: streamConfig.quality,
        },
      ];
  const activeFpsOption = fpsOptions.some(
    (option) => option.value === streamConfig.fps,
  )
    ? []
    : [{ label: String(streamConfig.fps), value: streamConfig.fps }];
  const canRotateSelectedSimulator =
    selectedSimulator != null &&
    !simulatorHasFixedOrientation(selectedSimulator);
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
        <div
          className="menu-popover"
          onPointerDown={(event) => event.stopPropagation()}
        >
          {selectedSimulator ? (
            <>
              <div className="menu-section">
                <div className="menu-section-heading">
                  <span className="menu-section-title">Stream</span>
                  <span className="menu-section-meta">
                    {formatStreamConfigSummary(streamConfig, streamTransport)}
                  </span>
                </div>
                <label className="menu-field">
                  <span className="menu-field-label">Transport</span>
                  <select
                    className="menu-select"
                    onChange={(event) =>
                      onStreamTransportChange(
                        event.currentTarget.value as StreamTransport,
                      )
                    }
                    value={streamTransport}
                  >
                    {STREAM_TRANSPORTS.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </label>
                <div aria-label="Encoder" className="menu-segment">
                  {STREAM_ENCODERS.map((option) => (
                    <button
                      className={`menu-option ${streamConfig.encoder === option.value ? "active" : ""}`}
                      key={option.value}
                      onClick={() => onStreamEncoderChange(option.value)}
                      type="button"
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
                <div aria-label="Frame rate" className="menu-segment">
                  {[...activeFpsOption, ...fpsOptions].map((option) => (
                    <button
                      className={`menu-option ${streamConfig.fps === option.value ? "active" : ""}`}
                      key={option.value}
                      onClick={() => onStreamFpsChange(option.value)}
                      type="button"
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
                <label className="menu-field">
                  <span className="menu-field-label">Resolution</span>
                  <select
                    className="menu-select"
                    onChange={(event) =>
                      onStreamQualityChange(
                        event.currentTarget.value as StreamQualityPreset,
                      )
                    }
                    value={streamConfig.quality}
                  >
                    {[...activeQualityOption, ...qualityOptions].map(
                      (option) => (
                        <option key={option.value} value={option.value}>
                          {option.label}
                        </option>
                      ),
                    )}
                  </select>
                </label>
              </div>
              <div className="menu-divider" />
              <div className="menu-actions">
                {showBootButton ? (
                  <button
                    className="menu-action mobile-menu-action"
                    onClick={() => {
                      onBoot();
                      onCloseMenu();
                    }}
                  >
                    Boot
                  </button>
                ) : null}
                {showStopButton ? (
                  <button
                    className="menu-action mobile-menu-action"
                    onClick={() => {
                      onShutdown();
                      onCloseMenu();
                    }}
                  >
                    Stop
                  </button>
                ) : null}
                <button className="menu-action" onClick={onOpenUrlPrompt}>
                  Open URL…
                </button>
                <button
                  className="menu-action"
                  disabled={!canInstallApp}
                  onClick={() => {
                    onInstallAppPrompt();
                    onCloseMenu();
                  }}
                >
                  Install App…
                </button>
                <button className="menu-action" onClick={onOpenBundlePrompt}>
                  Launch Bundle…
                </button>
                <button
                  className="menu-action"
                  disabled={captureBusy}
                  onClick={() => {
                    onCaptureScreenshot();
                    onCloseMenu();
                  }}
                >
                  Screenshot
                </button>
                <button
                  className="menu-action"
                  disabled={captureBusy}
                  onClick={() => {
                    onCaptureScreenshotWithBezel();
                    onCloseMenu();
                  }}
                >
                  Screenshot With Bezel
                </button>
                <button
                  className="menu-action"
                  disabled={captureBusy || recordingStopping}
                  onClick={() => {
                    onToggleRecording();
                    onCloseMenu();
                  }}
                >
                  {recordingStopping
                    ? "Stopping Recording"
                    : recordingActive
                      ? "Stop Recording"
                      : "Start Recording"}
                </button>
                <button
                  className="menu-action"
                  onClick={() => {
                    onOpenCameraSimulation();
                    onCloseMenu();
                  }}
                >
                  Camera Simulation…
                </button>
                <button
                  className="menu-action mobile-menu-action"
                  onClick={() => {
                    onHome();
                    onCloseMenu();
                  }}
                >
                  Home
                </button>
                <button
                  className="menu-action mobile-menu-action"
                  onClick={() => {
                    onOpenAppSwitcher();
                    onCloseMenu();
                  }}
                >
                  App Switcher
                </button>
                {canRotateSelectedSimulator ? (
                  <button
                    className="menu-action mobile-menu-action"
                    onClick={() => {
                      onRotateRight();
                      onCloseMenu();
                    }}
                  >
                    Rotate Right
                  </button>
                ) : null}
                <button
                  className="menu-action"
                  onClick={() => {
                    onDismissKeyboard();
                    onCloseMenu();
                  }}
                >
                  Dismiss Keyboard
                </button>
                <button
                  className="menu-action"
                  onClick={() => {
                    onToggleSoftwareKeyboard();
                    onCloseMenu();
                  }}
                >
                  Toggle Software Keyboard
                </button>
                <button className="menu-action" onClick={onToggleTouchOverlay}>
                  {touchOverlayVisible
                    ? "Hide Touch Overlay"
                    : "Show Touch Overlay"}
                </button>
                <button
                  className="menu-action mobile-menu-action"
                  onClick={() => {
                    onToggleAppearance();
                    onCloseMenu();
                  }}
                >
                  Toggle Appearance
                </button>
                <button className="menu-action" onClick={onToggleDebug}>
                  {debugVisible ? "Hide Debug Info" : "Show Debug Info"}
                </button>
              </div>
            </>
          ) : (
            <p className="list-empty">No simulator selected</p>
          )}
        </div>
      ) : null}
    </div>
  );
}

const STREAM_ENCODERS: Array<{ label: string; value: StreamEncoder }> = [
  { label: "Auto", value: "auto" },
  { label: "Hardware", value: "hardware" },
  { label: "Software", value: "software" },
];

const STREAM_TRANSPORTS: Array<{ label: string; value: StreamTransport }> = [
  { label: "Auto", value: "auto" },
  { label: "WebRTC", value: "webrtc" },
];

const LOCAL_STREAM_FPS_OPTIONS: Array<{ label: string; value: StreamFps }> = [
  { label: "30", value: 30 },
  { label: "60", value: 60 },
  { label: "120", value: 120 },
];

const REMOTE_STREAM_FPS_OPTIONS: Array<{ label: string; value: StreamFps }> = [
  { label: "15", value: 15 },
  { label: "30", value: 30 },
  { label: "60", value: 60 },
];

const STREAM_QUALITY_OPTIONS: Array<{
  label: string;
  value: StreamQualityPreset;
}> = [
  { label: "Auto", value: "auto" },
  { label: "Full res", value: "full" },
  { label: "1280", value: "balanced" },
  { label: "Smooth", value: "smooth" },
  { label: "1080", value: "economy" },
  { label: "720", value: "low" },
  { label: "540", value: "tiny" },
];

const STREAM_QUALITY_LABELS: Partial<Record<StreamQualityPreset, string>> = {
  auto: "Auto",
  balanced: "1280px",
  economy: "1080px",
  full: "Full res",
  low: "720px",
  quality: "Full+",
  smooth: "Smooth",
  tiny: "540px",
};

function streamQualityOptionLabel(quality: StreamQualityPreset): string {
  return STREAM_QUALITY_LABELS[quality] ?? quality;
}

function formatStreamConfigSummary(
  streamConfig: StreamConfig,
  transport: StreamTransport,
): string {
  const transportLabel = transport.toUpperCase();
  const resolution =
    STREAM_QUALITY_LABELS[streamConfig.quality] ??
    (typeof streamConfig.maxEdge === "number" && streamConfig.maxEdge > 0
      ? `${streamConfig.maxEdge}px`
      : "Full res");
  return `${transportLabel} / ${resolution} / ${streamConfig.fps} fps`;
}
