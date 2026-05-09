export type SimDeckLaunchOptions = {
  cliPath?: string;
  projectRoot?: string;
  udid?: string;
  keepDaemon?: boolean;
  isolated?: boolean;
  port?: number;
  videoCodec?: "auto" | "hardware" | "software" | "h264-software";
};
export type QueryOptions = {
  source?:
    | "auto"
    | "nativescript"
    | "react-native"
    | "swiftui"
    | "uikit"
    | "native-ax";
  maxDepth?: number;
  includeHidden?: boolean;
};
export type ElementSelector = {
  text?: string;
  id?: string;
  label?: string;
  value?: string;
  type?: string;
  index?: number;
  enabled?: boolean;
  checked?: boolean;
  focused?: boolean;
  selected?: boolean;
  regex?: boolean;
};
export type TapOptions = QueryOptions & {
  durationMs?: number;
  waitTimeoutMs?: number;
  pollMs?: number;
};
type DeviceMethod<TArgs extends unknown[], TResult> = {
  (...args: TArgs): Promise<TResult>;
  (udid: string, ...args: TArgs): Promise<TResult>;
};
type DeviceNoArgMethod<TResult> = {
  (): Promise<TResult>;
  (udid: string): Promise<TResult>;
};
export type SimDeckSession = {
  endpoint: string;
  pid: number;
  projectRoot: string;
  udid?: string;
  device(udid: string): SimDeckSession;
  list(): Promise<unknown>;
  install: DeviceMethod<[appPath: string], void>;
  uninstall: DeviceMethod<[bundleId: string], void>;
  launch: DeviceMethod<[bundleId: string], void>;
  openUrl: DeviceMethod<[url: string], void>;
  tap: DeviceMethod<[x: number, y: number], void>;
  tapElement: DeviceMethod<
    [selector: ElementSelector, options?: TapOptions],
    void
  >;
  touch: DeviceMethod<[x: number, y: number, phase: string], void>;
  key: DeviceMethod<[keyCode: number, modifiers?: number], void>;
  button: DeviceMethod<[button: string, durationMs?: number], void>;
  pasteboardSet: DeviceMethod<[text: string], void>;
  pasteboardGet: DeviceNoArgMethod<string>;
  chromeProfile: DeviceNoArgMethod<unknown>;
  tree: DeviceMethod<[options?: QueryOptions], unknown>;
  query: DeviceMethod<
    [selector: ElementSelector, options?: QueryOptions],
    unknown[]
  >;
  assert: DeviceMethod<
    [selector: ElementSelector, options?: QueryOptions],
    unknown
  >;
  assertNot: DeviceMethod<
    [selector: ElementSelector, options?: QueryOptions],
    unknown
  >;
  waitFor: DeviceMethod<
    [
      selector: ElementSelector,
      options?: QueryOptions & {
        timeoutMs?: number;
        pollMs?: number;
      },
    ],
    unknown
  >;
  waitForNot: DeviceMethod<
    [
      selector: ElementSelector,
      options?: QueryOptions & {
        timeoutMs?: number;
        pollMs?: number;
      },
    ],
    unknown
  >;
  scrollUntilVisible: DeviceMethod<
    [
      selector: ElementSelector,
      options?: QueryOptions & {
        timeoutMs?: number;
        pollMs?: number;
        direction?: "up" | "down" | "left" | "right";
        durationMs?: number;
        steps?: number;
      },
    ],
    unknown
  >;
  batch: DeviceMethod<[steps: unknown[], continueOnError?: boolean], unknown>;
  screenshot: DeviceNoArgMethod<Buffer>;
  close(): void;
};
export declare function connect(
  options?: SimDeckLaunchOptions,
): Promise<SimDeckSession>;
export {};
