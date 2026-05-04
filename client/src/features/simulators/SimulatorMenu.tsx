import type { RefObject } from "react";

import type { SimulatorMetadata } from "../../api/types";
import type {
  StreamConfig,
  StreamEncoder,
  StreamFps,
  StreamQualityPreset,
} from "../stream/streamTypes";
import { SimulatorRow } from "./SimulatorRow";

interface SimulatorMenuProps {
  debugVisible: boolean;
  filteredSimulators: SimulatorMetadata[];
  hideSimulatorSelection?: boolean;
  isLoading: boolean;
  menuOpen: boolean;
  menuRef: RefObject<HTMLDivElement | null>;
  onChangeSearch: (value: string) => void;
  onCloseMenu: () => void;
  onDismissKeyboard: () => void;
  onOpenBundlePrompt: () => void;
  onOpenUrlPrompt: () => void;
  onRotateLeft: () => void;
  onStreamEncoderChange: (encoder: StreamEncoder) => void;
  onStreamFpsChange: (fps: StreamFps) => void;
  onStreamQualityChange: (quality: StreamQualityPreset) => void;
  onToggleAppearance: () => void;
  onToggleDebug: () => void;
  onToggleMenu: () => void;
  onToggleTouchOverlay: () => void;
  remoteStream?: boolean;
  search: string;
  selectedSimulator: SimulatorMetadata | null;
  setSelectedUDID: (udid: string) => void;
  streamConfig: StreamConfig;
  touchOverlayVisible: boolean;
}

export function SimulatorMenu({
  debugVisible,
  filteredSimulators,
  hideSimulatorSelection = false,
  isLoading,
  menuOpen,
  menuRef,
  onChangeSearch,
  onCloseMenu,
  onDismissKeyboard,
  onOpenBundlePrompt,
  onOpenUrlPrompt,
  onRotateLeft,
  onStreamEncoderChange,
  onStreamFpsChange,
  onStreamQualityChange,
  onToggleAppearance,
  onToggleDebug,
  onToggleMenu,
  onToggleTouchOverlay,
  remoteStream = false,
  search,
  selectedSimulator,
  setSelectedUDID,
  streamConfig,
  touchOverlayVisible,
}: SimulatorMenuProps) {
  const fpsOptions = remoteStream
    ? REMOTE_STREAM_FPS_OPTIONS
    : LOCAL_STREAM_FPS_OPTIONS;
  const activeFpsOption = fpsOptions.some(
    (option) => option.value === streamConfig.fps,
  )
    ? []
    : [{ label: String(streamConfig.fps), value: streamConfig.fps }];
  const activeQualityOption = STREAM_QUALITY_OPTIONS.some(
    (option) => option.value === streamConfig.quality,
  )
    ? []
    : [{ label: streamConfig.quality, value: streamConfig.quality }];

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
          {!hideSimulatorSelection ? (
            <>
              <input
                className="sidebar-search"
                onChange={(event) => onChangeSearch(event.target.value)}
                placeholder="Search simulators..."
                value={search}
              />
              <div className="sim-list">
                {isLoading ? <p className="list-empty">Loading...</p> : null}
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
            </>
          ) : null}
          {selectedSimulator ? (
            <>
              <div className="menu-divider" />
              <div className="menu-section">
                <div className="menu-section-heading">
                  <span className="menu-section-title">Stream</span>
                  <span className="menu-section-meta">
                    {formatStreamConfigSummary(streamConfig)}
                  </span>
                </div>
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
                <div aria-label="Quality" className="menu-segment">
                  {[...activeQualityOption, ...STREAM_QUALITY_OPTIONS].map(
                    (option) => (
                      <button
                        className={`menu-option ${streamConfig.quality === option.value ? "active" : ""}`}
                        key={option.value}
                        onClick={() => onStreamQualityChange(option.value)}
                        type="button"
                      >
                        {option.label}
                      </button>
                    ),
                  )}
                </div>
              </div>
              <div className="menu-divider" />
              <div className="menu-actions">
                <button className="menu-action" onClick={onOpenUrlPrompt}>
                  Open URL…
                </button>
                <button className="menu-action" onClick={onOpenBundlePrompt}>
                  Launch Bundle…
                </button>
                <button
                  className="menu-action"
                  onClick={() => {
                    onDismissKeyboard();
                    onCloseMenu();
                  }}
                >
                  Dismiss Keyboard
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
                <button
                  className="menu-action mobile-menu-action"
                  onClick={() => {
                    onRotateLeft();
                    onCloseMenu();
                  }}
                >
                  Rotate Left
                </button>
              </div>
            </>
          ) : null}
          <div className="menu-actions">
            <button className="menu-action" onClick={onToggleDebug}>
              {debugVisible ? "Hide Debug Info" : "Show Debug Info"}
            </button>
          </div>
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
  { label: "Quality", value: "quality" },
  { label: "Smooth", value: "smooth" },
  { label: "Balanced", value: "balanced" },
  { label: "Fast", value: "fast" },
  { label: "Economy", value: "economy" },
];

function MenuIcon() {
  return (
    <svg fill="currentColor" height="16" viewBox="0 0 16 16" width="16">
      <path d="M2 3.5h12v1.5H2zm0 3.75h12v1.5H2zm0 3.75h12v1.5H2z" />
    </svg>
  );
}

function formatStreamConfigSummary(streamConfig: StreamConfig): string {
  const resolution =
    typeof streamConfig.maxEdge === "number" && streamConfig.maxEdge > 0
      ? `${streamConfig.maxEdge}px`
      : "Full res";
  return `${resolution} / ${streamConfig.fps} fps`;
}
