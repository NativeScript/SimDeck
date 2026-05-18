export interface EncoderStats {
  activeEncoderMode?: string;
  averageEncodeLatencyUs?: number;
  averageEncoderLoadPercent?: number;
  autoHardwareRetries?: number;
  autoSoftwareFallbackActive?: boolean;
  autoSoftwareFallbackRemainingUs?: number;
  autoSoftwareFallbacks?: number;
  clientForeground?: boolean;
  consecutiveOverBudgetFrames?: number;
  encoderBudgetUs?: number;
  encoderLoadPercent?: number;
  encoderMode?: string;
  hardwareAccelerated?: boolean;
  latestEncodeLatencyUs?: number;
  overloadEvents?: number;
  overloaded?: boolean;
  overloadReason?: string;
  overloadState?: "nominal" | "strained" | "overloaded" | string;
  peakEncodeLatencyUs?: number;
  selectedEncoderId?: string | null;
}

export interface PrivateDisplayInfo {
  displayReady: boolean;
  displayStatus: string;
  displayWidth: number;
  displayHeight: number;
  encoder?: EncoderStats;
  frameSequence: number;
  lastFrameAt?: number;
  rotationQuarterTurns?: number;
}

export interface SimulatorMetadata {
  udid: string;
  name: string;
  platform?: "ios-simulator" | "android-emulator" | string;
  runtimeName?: string;
  runtimeIdentifier?: string;
  deviceTypeName?: string;
  deviceTypeIdentifier?: string;
  pairedWatchUDID?: string;
  pairedWatchName?: string;
  pairedPhoneUDID?: string;
  pairedPhoneName?: string;
  devicePairIdentifier?: string;
  devicePairState?: string;
  isBooted: boolean;
  android?: {
    avdName?: string;
    grpcPort?: number;
    serial?: string;
  };
  privateDisplay?: PrivateDisplayInfo;
}

export interface SimulatorsResponse {
  simulators: SimulatorMetadata[];
}

export interface SimulatorDeviceTypeOption {
  identifier: string;
  name: string;
  productFamily?: string;
  modelIdentifier?: string;
  minRuntimeVersion?: number;
  minRuntimeVersionString?: string;
  maxRuntimeVersion?: number;
  maxRuntimeVersionString?: string;
  supportedRuntimeIdentifiers?: string[];
}

export interface SimulatorRuntimeOption {
  identifier: string;
  name: string;
  platform?: string;
  version?: string;
  buildVersion?: string;
  isAvailable?: boolean;
  supportedDeviceTypeIdentifiers?: string[];
}

export interface AndroidEmulatorDeviceTypeOption {
  identifier: string;
  name: string;
  oem?: string | null;
  tag?: string | null;
}

export interface AndroidEmulatorSystemImageOption {
  identifier: string;
  name: string;
  description?: string;
  apiLevel?: number | null;
  tag?: string;
  abi?: string;
}

export interface AndroidEmulatorCreateOptions {
  deviceTypes: AndroidEmulatorDeviceTypeOption[];
  systemImages: AndroidEmulatorSystemImageOption[];
  unavailableReason?: string;
}

export interface SimulatorCreateOptionsResponse {
  deviceTypes: SimulatorDeviceTypeOption[];
  runtimes: SimulatorRuntimeOption[];
  android?: AndroidEmulatorCreateOptions;
}

export interface CreatePairedWatchRequest {
  name: string;
  deviceTypeIdentifier: string;
  runtimeIdentifier?: string;
}

export interface CreateSimulatorRequest {
  platform?: "ios" | "android" | string;
  name: string;
  deviceTypeIdentifier: string;
  runtimeIdentifier?: string;
  pairedWatch?: CreatePairedWatchRequest;
}

export interface CreateSimulatorResponse {
  ok: boolean;
  created: {
    udid: string;
    pairedWatchUDID?: string;
  };
  simulator: SimulatorMetadata;
  pairedWatchSimulator?: SimulatorMetadata | null;
}

export interface WebKitTarget {
  id: string;
  appId: string;
  appName?: string | null;
  appActive?: boolean;
  pageActive?: boolean;
  pageId: number;
  title?: string | null;
  url?: string | null;
  kind: "safari-page" | "app-web-content" | "web-content-proxy" | string;
  inspectorUrl: string;
  webSocketUrl: string;
}

export interface WebKitTargetDiscovery {
  udid: string;
  socketPath?: string | null;
  targets: WebKitTarget[];
  warnings: string[];
}

export interface ChromeDevToolsTarget {
  id: string;
  appName?: string | null;
  bundleIdentifier?: string | null;
  description: string;
  devtoolsFrontendUrl: string;
  processIdentifier: number;
  source:
    | "react-native"
    | "react-native-metro"
    | "chrome-inspector"
    | "nativescript"
    | "swiftui"
    | "in-app-inspector"
    | string;
  title: string;
  type: string;
  url: string;
  webSocketDebuggerUrl: string;
}

export interface ChromeDevToolsTargetDiscovery {
  foregroundApp?: {
    appName?: string | null;
    bundleIdentifier?: string | null;
    processIdentifier: number;
  } | null;
  udid: string;
  targets: ChromeDevToolsTarget[];
  warnings: string[];
}

export interface HealthResponse {
  ok: boolean;
  videoCodec?: string;
  webRtc?: {
    iceServers?: RTCIceServer[];
    iceTransportPolicy?: RTCIceTransportPolicy;
  };
}

export interface SimulatorResponse {
  simulator: SimulatorMetadata;
}

export interface InstallUploadResponse {
  action: "install";
  fileName: string;
  ok: boolean;
  udid: string;
}

export interface SimulatorForegroundApp {
  appName?: string | null;
  bundleIdentifier?: string | null;
  processIdentifier: number;
}

export interface SimulatorStateResponse {
  booted: boolean;
  displayReady: boolean;
  displayStatus: string;
  foregroundApp?: SimulatorForegroundApp | null;
  frameSequence: number;
  lastFrameAgeMs?: number | null;
  lastFrameAt: number;
  simulator: SimulatorMetadata;
  udid: string;
}

export interface ChromeProfile {
  totalWidth: number;
  totalHeight: number;
  screenX: number;
  screenY: number;
  screenWidth: number;
  screenHeight: number;
  contentX?: number;
  contentY?: number;
  contentWidth?: number;
  contentHeight?: number;
  cornerRadius: number;
  cornerRadii?: {
    topLeft?: number;
    topRight?: number;
    bottomRight?: number;
    bottomLeft?: number;
  };
  chromeStyle?: "asset" | string;
  hasScreenMask?: boolean;
  buttons?: ChromeButtonProfile[];
}

export interface ChromeButtonProfile {
  name: string;
  label?: string;
  type?: string;
  imageName?: string;
  imageDownName?: string;
  imageDownDrawMode?: string;
  x: number;
  y: number;
  width: number;
  height: number;
  anchor?: "left" | "right" | "top" | "bottom" | string;
  align?: string;
  onTop?: boolean;
  usagePage?: number;
  usage?: number;
  normalOffset?: { x: number; y: number };
  rolloverOffset?: { x: number; y: number };
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
  androidClass?: string | null;
  androidPackage?: string | null;
  androidResourceId?: string | null;
  backgroundColor?: Record<string, unknown> | null;
  bounds?: AccessibilityFrame | null;
  checkable?: boolean | null;
  checked?: boolean | null;
  className?: string | null;
  clickable?: boolean | null;
  children?: AccessibilityNode[];
  control?: Record<string, unknown> | null;
  content_required?: boolean | null;
  custom_actions?: string[] | null;
  debugDescription?: string | null;
  enabled?: boolean | null;
  focusable?: boolean | null;
  focused?: boolean | null;
  frame?: AccessibilityFrame | null;
  frameInScreen?: AccessibilityFrame | null;
  flutter?: Record<string, unknown> | null;
  help?: string | null;
  imageName?: string | null;
  inspectorId?: string | null;
  isHidden?: boolean | null;
  isOpaque?: boolean | null;
  isUserInteractionEnabled?: boolean | null;
  longClickable?: boolean | null;
  moduleName?: string | null;
  nativeScript?: Record<string, unknown> | null;
  password?: boolean | null;
  pid?: number | null;
  placeholder?: string | null;
  reactNative?: Record<string, unknown> | null;
  role?: string | null;
  role_description?: string | null;
  scroll?: Record<string, unknown> | null;
  semantics?: Record<string, unknown> | null;
  scrollable?: boolean | null;
  selected?: boolean | null;
  source?:
    | "native-ax"
    | "in-app-inspector"
    | "nativescript"
    | "react-native"
    | "flutter"
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
  | "android-uiautomator"
  | "native-ax"
  | "in-app-inspector"
  | "nativescript"
  | "react-native"
  | "flutter"
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

export interface PerformanceProcess {
  pid: number;
  parentPid: number;
  process: string;
  role: string;
  state: string;
  appName?: string | null;
  bundleIdentifier?: string | null;
  command: string;
  isForeground: boolean;
}

export interface PerformanceHangStatus {
  state: string;
  staleMs?: number | null;
  reason: string;
}

export interface PerformanceSample {
  pid: number;
  timestampMs: number;
  cpuPercent: number;
  memoryResidentBytes?: number | null;
  memoryFootprintBytes?: number | null;
  memoryPeakFootprintBytes?: number | null;
  diskReadBytes?: number | null;
  diskWriteBytes?: number | null;
  diskReadBytesPerSecond?: number | null;
  diskWriteBytesPerSecond?: number | null;
  networkReceivedBytes?: number | null;
  networkSentBytes?: number | null;
  networkReceivedBytesPerSecond?: number | null;
  networkSentBytesPerSecond?: number | null;
  networkConnectionCount?: number | null;
  networkEstablishedConnectionCount?: number | null;
  networkEndpoints: string[];
  hang: PerformanceHangStatus;
}

export interface PerformanceEvent {
  category: string;
  level: string;
  message: string;
  pid: number | null;
  process: string;
  subsystem: string;
  timestamp: string;
}

export interface SimulatorPerformanceResponse {
  udid: string;
  sampledAt: number;
  selectedPid?: number | null;
  foregroundProcess?: SimulatorForegroundApp | null;
  processes: PerformanceProcess[];
  current?: PerformanceSample | null;
  history: PerformanceSample[];
  events: PerformanceEvent[];
  warnings: string[];
}

export interface SimulatorProcessListResponse {
  udid: string;
  foregroundProcess?: SimulatorForegroundApp | null;
  processes: PerformanceProcess[];
}

export interface StackSampleReport {
  pid: number;
  seconds: number;
  sampledAt: number;
  report: string;
  stderr: string;
  truncated: boolean;
}

export interface StackSampleResponse {
  udid: string;
  sample: StackSampleReport;
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

export interface EdgeTouchPayload extends TouchPayload {
  edge: "left" | "top" | "bottom" | "right" | "none";
}

export interface MultiTouchPayload {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  phase: TouchPhase;
}

export interface KeyPayload {
  keyCode: number;
  modifiers: number;
}

export interface ButtonPayload {
  button: string;
  durationMs?: number;
  phase?: "down" | "up" | "began" | "ended" | "cancelled";
  usagePage?: number;
  usage?: number;
}

export interface CrownPayload {
  delta: number;
}

export interface LaunchPayload {
  bundleId: string;
}

export interface OpenUrlPayload {
  url: string;
}
