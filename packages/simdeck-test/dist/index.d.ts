export type SimDeckLaunchOptions = {
  cliPath?: string;
  projectRoot?: string;
  keepDaemon?: boolean;
  isolated?: boolean;
  port?: number;
  videoCodec?: "auto" | "hardware" | "software" | "h264-software";
  udid?: string;
};
export type QueryOptions = {
  source?:
    | "auto"
    | "nativescript"
    | "react-native"
    | "flutter"
    | "uikit"
    | "native-ax"
    | "android-uiautomator";
  maxDepth?: number;
  includeHidden?: boolean;
};
export type ElementSelector = {
  id?: string;
  label?: string;
  value?: string;
  type?: string;
};
export type TapOptions = QueryOptions & {
  durationMs?: number;
  waitTimeoutMs?: number;
  pollMs?: number;
};
export type SwipeOptions = {
  durationMs?: number;
  steps?: number;
};
export type GestureOptions = SwipeOptions & {
  delta?: number;
};
export type TypeTextOptions = {
  delayMs?: number;
};
export type KeySequenceOptions = {
  delayMs?: number;
};
export type LogsOptions = {
  backfill?: boolean;
  seconds?: number;
  limit?: number;
  levels?: string[];
  processes?: string[];
  q?: string;
};
export type ScreenshotOptions = {
  bezel?: boolean;
  withBezel?: boolean;
};
export type ScreenRecordingOptions = {
  seconds?: number;
};
type DeviceMethod<TArgs extends unknown[], TResult> = {
  (udid: string, ...args: TArgs): TResult;
  (...args: TArgs): TResult;
};
export type SimDeckSession = {
  endpoint: string;
  pid: number;
  projectRoot: string;
  list(): Promise<unknown>;
  boot: DeviceMethod<[], Promise<unknown>>;
  shutdown: DeviceMethod<[], Promise<unknown>>;
  erase: DeviceMethod<[], Promise<unknown>>;
  install: DeviceMethod<[appPath: string], Promise<void>>;
  uninstall: DeviceMethod<[bundleId: string], Promise<void>>;
  launch: DeviceMethod<[bundleId: string], Promise<void>>;
  openUrl: DeviceMethod<[url: string], Promise<void>>;
  tap: DeviceMethod<[x: number, y: number], Promise<void>>;
  tapElement: DeviceMethod<
    [selector: ElementSelector, options?: TapOptions],
    Promise<void>
  >;
  touch: DeviceMethod<[x: number, y: number, phase: string], Promise<void>>;
  swipe: DeviceMethod<
    [
      startX: number,
      startY: number,
      endX: number,
      endY: number,
      options?: SwipeOptions,
    ],
    Promise<unknown>
  >;
  gesture: DeviceMethod<
    [preset: string, options?: GestureOptions],
    Promise<unknown>
  >;
  typeText: DeviceMethod<
    [text: string, options?: TypeTextOptions],
    Promise<unknown>
  >;
  key: DeviceMethod<[keyCode: number, modifiers?: number], Promise<void>>;
  keySequence: DeviceMethod<
    [keyCodes: number[], options?: KeySequenceOptions],
    Promise<void>
  >;
  button: DeviceMethod<[button: string, durationMs?: number], Promise<void>>;
  home: DeviceMethod<[], Promise<void>>;
  dismissKeyboard: DeviceMethod<[], Promise<void>>;
  appSwitcher: DeviceMethod<[], Promise<void>>;
  rotateLeft: DeviceMethod<[], Promise<void>>;
  rotateRight: DeviceMethod<[], Promise<void>>;
  toggleAppearance: DeviceMethod<[], Promise<void>>;
  pasteboardSet: DeviceMethod<[text: string], Promise<void>>;
  pasteboardGet: DeviceMethod<[], Promise<string>>;
  chromeProfile: DeviceMethod<[], Promise<unknown>>;
  logs: DeviceMethod<[options?: LogsOptions], Promise<unknown[]>>;
  tree: DeviceMethod<[options?: QueryOptions], Promise<unknown>>;
  query: DeviceMethod<
    [selector: ElementSelector, options?: QueryOptions],
    Promise<unknown[]>
  >;
  assert: DeviceMethod<
    [selector: ElementSelector, options?: QueryOptions],
    Promise<unknown>
  >;
  waitFor: DeviceMethod<
    [
      selector: ElementSelector,
      options?: QueryOptions & {
        timeoutMs?: number;
        pollMs?: number;
      },
    ],
    Promise<unknown>
  >;
  batch: DeviceMethod<
    [steps: unknown[], continueOnError?: boolean],
    Promise<unknown>
  >;
  screenshot: DeviceMethod<[options?: ScreenshotOptions], Promise<Buffer>>;
  record: DeviceMethod<[options?: ScreenRecordingOptions], Promise<Buffer>>;
  close(): void;
};
export declare function connect(
  options?: SimDeckLaunchOptions,
): Promise<SimDeckSession>;
export {};
