export type SimDeckLaunchOptions = {
  cliPath?: string;
  projectRoot?: string;
  keepDaemon?: boolean;
  isolated?: boolean;
  port?: number;
  videoCodec?: "hevc" | "h264" | "h264-software";
};
export type QueryOptions = {
  source?: "auto" | "nativescript" | "uikit" | "native-ax";
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
export type SimDeckSession = {
  endpoint: string;
  pid: number;
  projectRoot: string;
  list(): Promise<unknown>;
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
  key(udid: string, keyCode: number, modifiers?: number): Promise<void>;
  button(udid: string, button: string, durationMs?: number): Promise<void>;
  pasteboardSet(udid: string, text: string): Promise<void>;
  pasteboardGet(udid: string): Promise<string>;
  chromeProfile(udid: string): Promise<unknown>;
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
