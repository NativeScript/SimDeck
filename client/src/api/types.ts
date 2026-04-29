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
  deviceTypeName?: string;
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
  hasScreenMask?: boolean;
}

export interface AccessibilityFrame {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface AccessibilitySourceLocation {
  column?: number | null;
  file?: string | null;
  kind?: string | null;
  line?: number | null;
  offset?: number | null;
}

export interface AccessibilityNode {
  AXFrame?: string | null;
  AXIdentifier?: string | null;
  AXLabel?: string | null;
  AXUniqueId?: string | null;
  AXValue?: string | null;
  alpha?: number | null;
  backgroundColor?: Record<string, unknown> | null;
  bounds?: AccessibilityFrame | null;
  className?: string | null;
  children?: AccessibilityNode[];
  control?: Record<string, unknown> | null;
  content_required?: boolean | null;
  custom_actions?: string[] | null;
  debugDescription?: string | null;
  enabled?: boolean | null;
  frame?: AccessibilityFrame | null;
  frameInScreen?: AccessibilityFrame | null;
  help?: string | null;
  imageName?: string | null;
  inspectorId?: string | null;
  isHidden?: boolean | null;
  isOpaque?: boolean | null;
  isUserInteractionEnabled?: boolean | null;
  moduleName?: string | null;
  nativeScript?: Record<string, unknown> | null;
  pid?: number | null;
  placeholder?: string | null;
  reactNative?: Record<string, unknown> | null;
  role?: string | null;
  role_description?: string | null;
  scroll?: Record<string, unknown> | null;
  source?:
    | "native-ax"
    | "in-app-inspector"
    | "nativescript"
    | "react-native"
    | "swiftui"
    | string
    | null;
  sourceColumn?: number | null;
  sourceFile?: string | null;
  sourceLine?: number | null;
  sourceLocation?: AccessibilitySourceLocation | null;
  sourceLocations?: AccessibilitySourceLocation[] | null;
  subrole?: string | null;
  swiftUI?: Record<string, unknown> | null;
  text?: string | null;
  title?: string | null;
  type?: string | null;
  uikit?: Record<string, unknown> | null;
  uikitId?: string | null;
  uikitScript?: Record<string, unknown> | null;
  viewController?: Record<string, unknown> | null;
}

export type AccessibilitySource =
  | "native-ax"
  | "in-app-inspector"
  | "nativescript"
  | "react-native"
  | "swiftui";
export type AccessibilitySourcePreference = AccessibilitySource | "auto";

export interface AccessibilityTreeResponse {
  availableSources?: AccessibilitySource[];
  fallbackReason?: string;
  fallbackSource?: "native-ax";
  inspector?: Record<string, unknown>;
  roots: AccessibilityNode[];
  source: AccessibilitySource;
}

export interface SimulatorLogEntry {
  category: string;
  level: string;
  message: string;
  pid: number | null;
  process: string;
  subsystem: string;
  timestamp: string;
}

export interface SimulatorLogsResponse {
  entries: SimulatorLogEntry[];
}

export interface InspectorRequestResponse<T = unknown> {
  inspector?: Record<string, unknown>;
  result: T;
}

export interface UIKitScriptResult {
  className?: string | null;
  id: string;
  ok: boolean;
  result?: unknown;
  script?: string;
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
