export interface PrivateDisplayInfo {
  displayReady: boolean;
  displayStatus: string;
  displayWidth: number;
  displayHeight: number;
  frameSequence: number;
}

export interface SimulatorMetadata {
  udid: string;
  name: string;
  runtimeName?: string;
  runtimeIdentifier?: string;
  deviceTypeIdentifier?: string;
  isBooted: boolean;
  privateDisplay?: PrivateDisplayInfo;
}

export interface SimulatorsResponse {
  simulators: SimulatorMetadata[];
}

export interface SimulatorResponse {
  simulator: SimulatorMetadata;
}

export interface ChromeProfile {
  totalWidth: number;
  totalHeight: number;
  screenX: number;
  screenY: number;
  screenWidth: number;
  screenHeight: number;
  cornerRadius: number;
}

export type TouchPhase = "began" | "moved" | "ended" | "cancelled";

export interface TouchPayload {
  x: number;
  y: number;
  phase: TouchPhase;
}

export interface KeyPayload {
  keyCode: number;
  modifiers: number;
}

export interface LaunchPayload {
  bundleId: string;
}

export interface OpenUrlPayload {
  url: string;
}
