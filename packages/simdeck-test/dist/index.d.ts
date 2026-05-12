export type SimDeckLaunchOptions = {
  cliPath?: string;
  projectRoot?: string;
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
export type SimDeckSession = {
  endpoint: string;
  pid: number;
  projectRoot: string;
  list(): Promise<unknown>;
  boot(udid: string): Promise<unknown>;
  shutdown(udid: string): Promise<unknown>;
  erase(udid: string): Promise<unknown>;
  install(udid: string, appPath: string): Promise<void>;
  uninstall(udid: string, bundleId: string): Promise<void>;
  launch(udid: string, bundleId: string): Promise<void>;
  openUrl(udid: string, url: string): Promise<void>;
  tap(udid: string, x: number, y: number): Promise<void>;
  tapElement(
    udid: string,
    selector: ElementSelector,
    options?: TapOptions,
  ): Promise<void>;
  touch(udid: string, x: number, y: number, phase: string): Promise<void>;
  swipe(
    udid: string,
    startX: number,
    startY: number,
    endX: number,
    endY: number,
    options?: SwipeOptions,
  ): Promise<unknown>;
  gesture(
    udid: string,
    preset: string,
    options?: GestureOptions,
  ): Promise<unknown>;
  typeText(
    udid: string,
    text: string,
    options?: TypeTextOptions,
  ): Promise<unknown>;
  key(udid: string, keyCode: number, modifiers?: number): Promise<void>;
  keySequence(
    udid: string,
    keyCodes: number[],
    options?: KeySequenceOptions,
  ): Promise<void>;
  button(udid: string, button: string, durationMs?: number): Promise<void>;
  home(udid: string): Promise<void>;
  dismissKeyboard(udid: string): Promise<void>;
  appSwitcher(udid: string): Promise<void>;
  rotateLeft(udid: string): Promise<void>;
  rotateRight(udid: string): Promise<void>;
  toggleAppearance(udid: string): Promise<void>;
  pasteboardSet(udid: string, text: string): Promise<void>;
  pasteboardGet(udid: string): Promise<string>;
  chromeProfile(udid: string): Promise<unknown>;
  logs(udid: string, options?: LogsOptions): Promise<unknown[]>;
  tree(udid: string, options?: QueryOptions): Promise<unknown>;
  query(
    udid: string,
    selector: ElementSelector,
    options?: QueryOptions,
  ): Promise<unknown[]>;
  assert(
    udid: string,
    selector: ElementSelector,
    options?: QueryOptions,
  ): Promise<unknown>;
  waitFor(
    udid: string,
    selector: ElementSelector,
    options?: QueryOptions & {
      timeoutMs?: number;
      pollMs?: number;
    },
  ): Promise<unknown>;
  batch(
    udid: string,
    steps: unknown[],
    continueOnError?: boolean,
  ): Promise<unknown>;
  screenshot(udid: string): Promise<Buffer>;
  close(): void;
};
export declare function connect(
  options?: SimDeckLaunchOptions,
): Promise<SimDeckSession>;
