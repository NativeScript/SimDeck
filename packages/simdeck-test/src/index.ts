import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";

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
      options?: QueryOptions & { timeoutMs?: number; pollMs?: number },
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

type DaemonStartResult = {
  ok: boolean;
  projectRoot: string;
  pid: number;
  url: string;
  started: boolean;
};

type DaemonConnection = DaemonStartResult & {
  child?: ChildProcess;
  isolatedRoot?: string;
};

export async function connect(
  options: SimDeckLaunchOptions = {},
): Promise<SimDeckSession> {
  const cliPath = options.cliPath ?? "simdeck";
  const result: DaemonConnection = options.isolated
    ? await startIsolatedDaemon(cliPath, options)
    : runJson<DaemonStartResult>(cliPath, ["daemon", "start"], {
        cwd: options.projectRoot,
      });
  const endpoint = result.url;
  const defaultUdid = options.udid;
  const simulatorPath = (udid: string, suffix: string) =>
    `/api/simulators/${encodeURIComponent(udid)}${suffix}`;
  const requireUdid = (udid?: string) => {
    const resolved = udid ?? defaultUdid;
    if (!resolved) {
      throw new Error(
        "This SimDeck session method requires a UDID. Pass one as the first argument or call connect({ udid }).",
      );
    }
    return resolved;
  };
  const resolveNoArgDeviceCall = (args: unknown[]) => ({
    udid: requireUdid(typeof args[0] === "string" ? args[0] : undefined),
  });
  const resolveStringArgDeviceCall = (args: unknown[]) => {
    if (
      args.length >= 2 &&
      typeof args[0] === "string" &&
      typeof args[1] === "string"
    ) {
      return { udid: args[0], value: args[1] as string, rest: args.slice(2) };
    }
    return {
      udid: requireUdid(),
      value: args[0] as string,
      rest: args.slice(1),
    };
  };
  const resolveObjectArgDeviceCall = <T>(args: unknown[]) => {
    if (typeof args[0] === "string") {
      return { udid: args[0], value: args[1] as T, rest: args.slice(2) };
    }
    return { udid: requireUdid(), value: args[0] as T, rest: args.slice(1) };
  };
  const resolveOptionalObjectDeviceCall = <T>(args: unknown[]) => {
    if (typeof args[0] === "string") {
      return { udid: args[0], options: args[1] as T | undefined };
    }
    return { udid: requireUdid(), options: args[0] as T | undefined };
  };
  const session: SimDeckSession = {
    endpoint,
    pid: result.pid,
    projectRoot: result.projectRoot,
    list: () => requestJson(endpoint, "GET", "/api/simulators"),
    boot: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestJson(endpoint, "POST", simulatorPath(udid, "/boot"), null);
    },
    shutdown: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestJson(
        endpoint,
        "POST",
        simulatorPath(udid, "/shutdown"),
        null,
      );
    },
    erase: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestJson(endpoint, "POST", simulatorPath(udid, "/erase"), null);
    },
    install: (...args) => {
      const { udid, value: appPath } = resolveStringArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/install"), {
        appPath,
      });
    },
    uninstall: (...args) => {
      const { udid, value: bundleId } = resolveStringArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/uninstall"), {
        bundleId,
      });
    },
    launch: (...args) => {
      const { udid, value: bundleId } = resolveStringArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/launch"), {
        bundleId,
      });
    },
    openUrl: (...args) => {
      const { udid, value: url } = resolveStringArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/open-url"), {
        url,
      });
    },
    tap: (...args) => {
      const [udid, x, y] =
        typeof args[0] === "string"
          ? [args[0], args[1] as number, args[2] as number]
          : [requireUdid(), args[0] as number, args[1] as number];
      return requestOk(endpoint, simulatorPath(udid, "/tap"), {
        x,
        y,
        normalized: true,
      });
    },
    tapElement: (...args) => {
      const {
        udid,
        value: selector,
        rest,
      } = resolveObjectArgDeviceCall<ElementSelector>(args);
      const [tapOptions] = rest as [TapOptions?];
      return requestOk(endpoint, simulatorPath(udid, "/tap"), {
        selector: selectorPayload(selector),
        ...tapOptions,
      });
    },
    touch: (...args) => {
      const [udid, x, y, phase] =
        typeof args[0] === "string"
          ? [args[0], args[1] as number, args[2] as number, args[3] as string]
          : [
              requireUdid(),
              args[0] as number,
              args[1] as number,
              args[2] as string,
            ];
      return requestOk(endpoint, simulatorPath(udid, "/touch"), {
        x,
        y,
        phase,
      });
    },
    swipe: (...args) => {
      const [udid, startX, startY, endX, endY, swipeOptions = {}] =
        typeof args[0] === "string"
          ? [
              args[0],
              args[1] as number,
              args[2] as number,
              args[3] as number,
              args[4] as number,
              args[5] as SwipeOptions | undefined,
            ]
          : [
              requireUdid(),
              args[0] as number,
              args[1] as number,
              args[2] as number,
              args[3] as number,
              args[4] as SwipeOptions | undefined,
            ];
      return requestJson(endpoint, "POST", simulatorPath(udid, "/batch"), {
        steps: [
          {
            action: "swipe",
            startX,
            startY,
            endX,
            endY,
            ...swipeOptions,
          },
        ],
      });
    },
    gesture: (...args) => {
      const { udid, value: preset, rest } = resolveStringArgDeviceCall(args);
      const [gestureOptions = {}] = rest as [GestureOptions?];
      return requestJson(endpoint, "POST", simulatorPath(udid, "/batch"), {
        steps: [
          {
            action: "gesture",
            preset,
            ...gestureOptions,
          },
        ],
      });
    },
    typeText: (...args) => {
      const { udid, value: text, rest } = resolveStringArgDeviceCall(args);
      const [typeOptions = {}] = rest as [TypeTextOptions?];
      return requestJson(endpoint, "POST", simulatorPath(udid, "/batch"), {
        steps: [
          {
            action: "type",
            text,
            ...typeOptions,
          },
        ],
      });
    },
    key: (...args) => {
      const [udid, keyCode, modifiers = 0] =
        typeof args[0] === "string"
          ? [args[0], args[1] as number, args[2] as number | undefined]
          : [requireUdid(), args[0] as number, args[1] as number | undefined];
      return requestOk(endpoint, simulatorPath(udid, "/key"), {
        keyCode,
        modifiers,
      });
    },
    keySequence: (...args) => {
      const {
        udid,
        value: keyCodes,
        rest,
      } = resolveObjectArgDeviceCall<number[]>(args);
      const [keySequenceOptions = {}] = rest as [KeySequenceOptions?];
      return requestOk(endpoint, simulatorPath(udid, "/key-sequence"), {
        keyCodes,
        ...keySequenceOptions,
      });
    },
    button: (...args) => {
      const { udid, value: button, rest } = resolveStringArgDeviceCall(args);
      const [durationMs = 0] = rest as [number?];
      return requestOk(endpoint, simulatorPath(udid, "/button"), {
        button,
        durationMs,
      });
    },
    home: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/home"), null);
    },
    dismissKeyboard: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestOk(
        endpoint,
        simulatorPath(udid, "/dismiss-keyboard"),
        null,
      );
    },
    appSwitcher: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/app-switcher"), null);
    },
    rotateLeft: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/rotate-left"), null);
    },
    rotateRight: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/rotate-right"), null);
    },
    toggleAppearance: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestOk(
        endpoint,
        simulatorPath(udid, "/toggle-appearance"),
        null,
      );
    },
    pasteboardSet: (...args) => {
      const { udid, value: text } = resolveStringArgDeviceCall(args);
      return requestOk(endpoint, simulatorPath(udid, "/pasteboard"), {
        text,
      });
    },
    pasteboardGet: async (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      const result = await requestJson<{ text?: string }>(
        endpoint,
        "GET",
        simulatorPath(udid, "/pasteboard"),
      );
      return result.text ?? "";
    },
    chromeProfile: (...args: [] | [string]) => {
      const { udid } = resolveNoArgDeviceCall(args);
      return requestJson(
        endpoint,
        "GET",
        simulatorPath(udid, "/chrome-profile"),
      );
    },
    logs: async (...args) => {
      const { udid, value: logsOptions } =
        typeof args[0] === "string"
          ? { udid: args[0], value: args[1] as LogsOptions | undefined }
          : { udid: requireUdid(), value: args[0] as LogsOptions | undefined };
      const result = await requestJson<{ entries?: unknown[] }>(
        endpoint,
        "GET",
        simulatorPath(udid, `/logs?${logsQuery(logsOptions)}`),
      );
      return result.entries ?? [];
    },
    tree: (...args) => {
      const { udid, value: treeOptions } =
        typeof args[0] === "string"
          ? { udid: args[0], value: args[1] as QueryOptions | undefined }
          : { udid: requireUdid(), value: args[0] as QueryOptions | undefined };
      return requestJson(
        endpoint,
        "GET",
        simulatorPath(udid, `/accessibility-tree?${treeQuery(treeOptions)}`),
      );
    },
    query: async (...args) => {
      const {
        udid,
        value: selector,
        rest,
      } = resolveObjectArgDeviceCall<ElementSelector>(args);
      const [treeOptions] = rest as [QueryOptions?];
      const result = await requestJson<{ matches: unknown[] }>(
        endpoint,
        "POST",
        simulatorPath(udid, "/query"),
        {
          selector: selectorPayload(selector),
          ...treeOptions,
        },
      );
      return result.matches;
    },
    assert: (...args) => {
      const {
        udid,
        value: selector,
        rest,
      } = resolveObjectArgDeviceCall<ElementSelector>(args);
      const [assertOptions] = rest as [QueryOptions?];
      return requestJson(endpoint, "POST", simulatorPath(udid, "/assert"), {
        selector: selectorPayload(selector),
        ...assertOptions,
      });
    },
    waitFor: (...args) => {
      const {
        udid,
        value: selector,
        rest,
      } = resolveObjectArgDeviceCall<ElementSelector>(args);
      const [waitOptions] = rest as [
        (QueryOptions & { timeoutMs?: number; pollMs?: number })?,
      ];
      return requestJson(endpoint, "POST", simulatorPath(udid, "/wait-for"), {
        selector: selectorPayload(selector),
        ...waitOptions,
      });
    },
    batch: (...args) => {
      const {
        udid,
        value: steps,
        rest,
      } = resolveObjectArgDeviceCall<unknown[]>(args);
      const [continueOnError = false] = rest as [boolean?];
      return requestJson(endpoint, "POST", simulatorPath(udid, "/batch"), {
        steps,
        continueOnError,
      });
    },
    screenshot: (
      ...args: [string, ScreenshotOptions?] | [ScreenshotOptions?]
    ) => {
      const { udid, options } =
        resolveOptionalObjectDeviceCall<ScreenshotOptions>(args);
      const params = new URLSearchParams();
      if (options?.withBezel ?? options?.bezel) {
        params.set("bezel", "true");
      }
      const query = params.toString();
      return requestBuffer(
        endpoint,
        simulatorPath(udid, `/screenshot.png${query ? `?${query}` : ""}`),
      );
    },
    record: (
      ...args: [string, ScreenRecordingOptions?] | [ScreenRecordingOptions?]
    ) => {
      const { udid, options } =
        resolveOptionalObjectDeviceCall<ScreenRecordingOptions>(args);
      return requestBuffer(
        endpoint,
        simulatorPath(udid, "/screen-recording"),
        "POST",
        {
          seconds: options?.seconds ?? 5,
        },
      );
    },
    close: () => {
      if (options.keepDaemon) {
        return;
      }
      if (result.child) {
        result.child.kill();
        if (result.isolatedRoot) {
          fs.rmSync(result.isolatedRoot, { recursive: true, force: true });
        }
        return;
      }
      if (result.started) {
        spawnSync(cliPath, ["daemon", "stop"], { cwd: options.projectRoot });
      }
    },
  };
  return session;
}

async function startIsolatedDaemon(
  cliPath: string,
  options: SimDeckLaunchOptions,
): Promise<DaemonConnection> {
  const port = options.port ?? (await freePortPair());
  const projectRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "simdeck-test-project-"),
  );
  const metadataPath = path.join(
    os.tmpdir(),
    `simdeck-test-${process.pid}-${Date.now()}-${crypto.randomUUID()}.json`,
  );
  const accessToken = crypto.randomBytes(32).toString("hex");
  const child = spawn(
    cliPath,
    [
      "daemon",
      "run",
      "--project-root",
      projectRoot,
      "--metadata-path",
      metadataPath,
      "--port",
      String(port),
      "--bind",
      "127.0.0.1",
      "--access-token",
      accessToken,
      "--video-codec",
      options.videoCodec ?? "software",
    ],
    {
      cwd: options.projectRoot,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  const output = captureChildOutput(child);
  const url = `http://127.0.0.1:${port}`;
  try {
    await waitForHealth(url, child, output);
  } catch (error) {
    child.kill();
    fs.rmSync(projectRoot, { recursive: true, force: true });
    throw error;
  }
  return {
    ok: true,
    projectRoot,
    pid: child.pid ?? 0,
    url,
    started: true,
    child,
    isolatedRoot: projectRoot,
  };
}

async function waitForHealth(
  endpoint: string,
  child: ChildProcess,
  output: () => string,
): Promise<void> {
  const deadline = Date.now() + 60_000;
  let lastError: unknown;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(
        `SimDeck isolated daemon exited with ${child.exitCode}.\n${output()}`,
      );
    }
    try {
      await requestJson(endpoint, "GET", "/api/health");
      return;
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  }
  throw new Error(
    `Timed out waiting for isolated SimDeck daemon: ${
      lastError instanceof Error ? lastError.message : String(lastError)
    }\n${output()}`,
  );
}

function captureChildOutput(child: ChildProcess): () => string {
  const chunks: string[] = [];
  const append = (source: string, chunk: Buffer) => {
    chunks.push(`[${source}] ${chunk.toString("utf8")}`);
    while (chunks.join("").length > 16_384) {
      chunks.shift();
    }
  };
  child.stdout?.on("data", (chunk: Buffer) => append("stdout", chunk));
  child.stderr?.on("data", (chunk: Buffer) => append("stderr", chunk));
  return () => chunks.join("").trim();
}

async function freePortPair(): Promise<number> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const port = await freePort();
    if (port < 65535 && (await portAvailable(port + 1))) {
      return port;
    }
  }
  throw new Error("Unable to allocate adjacent free TCP ports.");
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close();
        reject(new Error("Unable to allocate a free TCP port."));
        return;
      }
      const port = address.port;
      server.close(() => resolve(port));
    });
    server.on("error", reject);
  });
}

function portAvailable(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const server = net.createServer();
    server.once("error", () => resolve(false));
    server.listen(port, "127.0.0.1", () => {
      server.close(() => resolve(true));
    });
  });
}

function runJson<T>(
  command: string,
  args: string[],
  options: { cwd?: string } = {},
): T {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(
      result.stderr.trim() || `${command} ${args.join(" ")} failed`,
    );
  }
  return JSON.parse(result.stdout) as T;
}

function requestOk(
  endpoint: string,
  pathName: string,
  body: unknown,
): Promise<void> {
  return requestJson(endpoint, "POST", pathName, body).then(() => undefined);
}

function requestJson<T = unknown>(
  endpoint: string,
  method: string,
  pathName: string,
  body?: unknown,
): Promise<T> {
  return requestBuffer(endpoint, pathName, method, body).then((buffer) =>
    JSON.parse(buffer.toString("utf8")),
  );
}

function requestBuffer(
  endpoint: string,
  pathName: string,
  method = "GET",
  body?: unknown,
): Promise<Buffer> {
  const url = new URL(pathName, endpoint);
  const payload =
    body === undefined ? undefined : Buffer.from(JSON.stringify(body));
  return new Promise((resolve, reject) => {
    const request = http.request(
      url,
      {
        method,
        headers: payload
          ? {
              "content-type": "application/json",
              "content-length": String(payload.length),
              origin: endpoint,
            }
          : { origin: endpoint },
      },
      (response) => {
        const chunks: Buffer[] = [];
        response.on("data", (chunk: Buffer) => chunks.push(chunk));
        response.on("end", () => {
          const buffer = Buffer.concat(chunks);
          if (
            (response.statusCode ?? 500) < 200 ||
            (response.statusCode ?? 500) >= 300
          ) {
            reject(
              new Error(
                `${method} ${pathName} returned ${response.statusCode}: ${
                  buffer.toString("utf8") || response.statusMessage || ""
                }`,
              ),
            );
          } else {
            resolve(buffer);
          }
        });
      },
    );
    request.on("error", reject);
    if (payload) {
      request.write(payload);
    }
    request.end();
  });
}

function treeQuery(options: QueryOptions = {}): string {
  const params = new URLSearchParams();
  if (options.source) params.set("source", options.source);
  if (options.maxDepth !== undefined)
    params.set("maxDepth", String(options.maxDepth));
  if (options.includeHidden) params.set("includeHidden", "true");
  return params.toString();
}

function logsQuery(options: LogsOptions = {}): string {
  const params = new URLSearchParams();
  if (options.backfill !== undefined)
    params.set("backfill", String(options.backfill));
  if (options.seconds !== undefined)
    params.set("seconds", String(options.seconds));
  if (options.limit !== undefined) params.set("limit", String(options.limit));
  if (options.levels?.length) params.set("levels", options.levels.join(","));
  if (options.processes?.length)
    params.set("processes", options.processes.join(","));
  if (options.q) params.set("q", options.q);
  return params.toString();
}

function selectorPayload(
  selector: ElementSelector,
): Record<string, string | undefined> {
  return {
    id: selector.id,
    label: selector.label,
    value: selector.value,
    elementType: selector.type,
  };
}
