import type { RefObject } from "react";

import type { SimulatorMetadata } from "../../api/types";
import type {
  StreamConfig,
  StreamEncoder,
  StreamFps,
  StreamQualityPreset,
  StreamTransport,
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
  onStreamTransportChange: (transport: StreamTransport) => void;
  onToggleAppearance: () => void;
  onToggleDebug: () => void;
  onToggleMenu: () => void;
  onToggleTouchOverlay: () => void;
  remoteStream?: boolean;
  search: string;
  selectedSimulator: SimulatorMetadata | null;
  setSelectedUDID: (udid: string) => void;
  streamConfig: StreamConfig;
  streamTransport: StreamTransport;
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
  onStreamTransportChange,
  onToggleAppearance,
  onToggleDebug,
  onToggleMenu,
  onToggleTouchOverlay,
  remoteStream = false,
  search,
  selectedSimulator,
  setSelectedUDID,
  streamConfig,
  streamTransport,
  touchOverlayVisible,
}: SimulatorMenuProps) {
  const fpsOptions = remoteStream
    ? REMOTE_STREAM_FPS_OPTIONS
    : LOCAL_STREAM_FPS_OPTIONS;
  const qualityOptions =
    streamTransport === "mjpeg"
      ? MJPEG_STREAM_QUALITY_OPTIONS
      : H264_STREAM_QUALITY_OPTIONS;
  const activeQualityOption = qualityOptions.some(
    (option) => option.value === streamConfig.quality,
  )
    ? []
    : [
        {
          label: streamQualityOptionLabel(streamConfig.quality, streamTransport),
          value: streamConfig.quality,
        },
      ];
  const activeFpsOption = fpsOptions.some(
    (option) => option.value === streamConfig.fps,
  )
    ? []
    : [{ label: String(streamConfig.fps), value: streamConfig.fps }];
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
                  <span className="menu-field-label">
                    {streamTransport === "mjpeg"
                      ? "JPEG quality"
                      : "Resolution"}
                  </span>
                  <select
                    className="menu-select"
                    onChange={(event) =>
                      onStreamQualityChange(
                        event.currentTarget.value as StreamQualityPreset,
                      )
                    }
                    value={streamConfig.quality}
                  >
                    {[...activeQualityOption, ...qualityOptions].map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </label>
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
                <button className="menu-action" onClick={onToggleDebug}>
                  {debugVisible ? "Hide Debug Info" : "Show Debug Info"}
                </button>
              </div>
            </>
          ) : null}
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
  { label: "H264 WS", value: "h264" },
  { label: "MJPEG", value: "mjpeg" },
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

const H264_STREAM_QUALITY_OPTIONS: Array<{
  label: string;
  value: StreamQualityPreset;
}> = [
  { label: "Auto", value: "auto" },
  { label: "Full", value: "full" },
  { label: "1280", value: "balanced" },
  { label: "1080", value: "economy" },
  { label: "720", value: "low" },
  { label: "540", value: "tiny" },
];

const MJPEG_STREAM_QUALITY_OPTIONS: Array<{
  label: string;
  value: StreamQualityPreset;
}> = [
  { label: "Auto", value: "auto" },
  { label: "0.82", value: "quality" },
  { label: "0.76", value: "balanced" },
  { label: "0.70", value: "economy" },
  { label: "0.66", value: "low" },
  { label: "0.62", value: "tiny" },
];

const MJPEG_QUALITY_LABELS: Partial<Record<StreamQualityPreset, string>> = {
  auto: "Auto",
  balanced: "JPEG 0.76",
  economy: "JPEG 0.70",
  low: "JPEG 0.66",
  quality: "JPEG 0.82",
  smooth: "JPEG 0.74",
  tiny: "JPEG 0.62",
};

const H264_QUALITY_LABELS: Partial<Record<StreamQualityPreset, string>> = {
  auto: "Auto",
  balanced: "1280px",
  economy: "1080px",
  full: "Full res",
  low: "720px",
  quality: "Full+",
  smooth: "1170px",
  tiny: "540px",
};

function streamQualityOptionLabel(
  quality: StreamQualityPreset,
  transport: StreamTransport,
): string {
  if (transport === "mjpeg") {
    return MJPEG_QUALITY_LABELS[quality] ?? "JPEG quality";
  }
  return H264_QUALITY_LABELS[quality] ?? quality;
}

function MenuIcon() {
  return (
    <svg fill="currentColor" height="16" viewBox="0 0 16 16" width="16">
      <path d="M2 3.5h12v1.5H2zm0 3.75h12v1.5H2zm0 3.75h12v1.5H2z" />
    </svg>
  );
}

function formatStreamConfigSummary(
  streamConfig: StreamConfig,
  transport: StreamTransport,
): string {
  if (transport === "mjpeg") {
    const jpegQuality =
      MJPEG_QUALITY_LABELS[streamConfig.quality] ?? "JPEG quality";
    return `${transport.toUpperCase()} / ${jpegQuality} / ${streamConfig.fps} fps`;
  }
  const transportLabel =
    transport === "h264" ? "H264 WS" : transport.toUpperCase();
  const resolution =
    H264_QUALITY_LABELS[streamConfig.quality] ??
    (typeof streamConfig.maxEdge === "number" && streamConfig.maxEdge > 0
      ? `${streamConfig.maxEdge}px`
      : "Full res");
  return `${transportLabel} / ${resolution} / ${streamConfig.fps} fps`;
}
