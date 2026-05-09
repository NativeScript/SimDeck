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
      options?: QueryOptions & { timeoutMs?: number; pollMs?: number },
    ],
    unknown
  >;
  waitForNot: DeviceMethod<
    [
      selector: ElementSelector,
      options?: QueryOptions & { timeoutMs?: number; pollMs?: number },
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
  const createSession = (boundUdid?: string): SimDeckSession => {
    const requireUdid = (method: string): string => {
      if (boundUdid) return boundUdid;
      throw new Error(
        `${method} requires a UDID. Pass connect({ udid }) or call ${method}(udid, ...).`,
      );
    };
    const splitDeviceArgs = (
      method: string,
      args: unknown[],
      boundArity: number,
    ): [string, unknown[]] => {
      if (typeof args[0] === "string" && args.length > boundArity) {
        return [args[0], args.slice(1)];
      }
      return [requireUdid(method), args];
    };
    const splitDeviceNoArg = (
      method: string,
      args: unknown[],
    ): [string, unknown[]] => {
      if (typeof args[0] === "string") {
        return [args[0], args.slice(1)];
      }
      return [requireUdid(method), args];
    };
    return {
      endpoint,
      pid: result.pid,
      projectRoot: result.projectRoot,
      udid: boundUdid,
      device: (udid: string) => createSession(udid),
      list: () => requestJson(endpoint, "GET", "/api/simulators"),
      install: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("install", args, 1);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/install`,
          { appPath: rest[0] },
        );
      },
      uninstall: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("uninstall", args, 1);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/uninstall`,
          { bundleId: rest[0] },
        );
      },
      launch: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("launch", args, 1);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/launch`,
          { bundleId: rest[0] },
        );
      },
      openUrl: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("openUrl", args, 1);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/open-url`,
          { url: rest[0] },
        );
      },
      tap: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("tap", args, 2);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/tap`,
          {
            x: rest[0],
            y: rest[1],
            normalized: true,
          },
        );
      },
      tapElement: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("tapElement", args, 1);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/tap`,
          {
            selector: selectorPayload(rest[0] as ElementSelector),
            ...(rest[1] as TapOptions | undefined),
          },
        );
      },
      touch: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("touch", args, 3);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/touch`,
          {
            x: rest[0],
            y: rest[1],
            phase: rest[2],
          },
        );
      },
      key: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("key", args, 1);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/key`,
          {
            keyCode: rest[0],
            modifiers: rest[1] ?? 0,
          },
        );
      },
      button: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("button", args, 1);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/button`,
          { button: rest[0], durationMs: rest[1] ?? 0 },
        );
      },
      pasteboardSet: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("pasteboardSet", args, 1);
        return requestOk(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/pasteboard`,
          { text: rest[0] },
        );
      },
      pasteboardGet: async (...args: unknown[]) => {
        const [udid] = splitDeviceNoArg("pasteboardGet", args);
        const result = await requestJson<{ text?: string }>(
          endpoint,
          "GET",
          `/api/simulators/${encodeURIComponent(udid)}/pasteboard`,
        );
        return result.text ?? "";
      },
      chromeProfile: (...args: unknown[]) => {
        const [udid] = splitDeviceNoArg("chromeProfile", args);
        return requestJson(
          endpoint,
          "GET",
          `/api/simulators/${encodeURIComponent(udid)}/chrome-profile`,
        );
      },
      tree: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("tree", args, 0);
        return requestJson(
          endpoint,
          "GET",
          `/api/simulators/${encodeURIComponent(udid)}/accessibility-tree?${treeQuery(rest[0] as QueryOptions | undefined)}`,
        );
      },
      query: async (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("query", args, 1);
        const result = await requestJson<{ matches: unknown[] }>(
          endpoint,
          "POST",
          `/api/simulators/${encodeURIComponent(udid)}/query`,
          {
            selector: selectorPayload(rest[0] as ElementSelector),
            ...(rest[1] as QueryOptions | undefined),
          },
        );
        return result.matches;
      },
      assert: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("assert", args, 1);
        return requestJson(
          endpoint,
          "POST",
          `/api/simulators/${encodeURIComponent(udid)}/assert`,
          {
            selector: selectorPayload(rest[0] as ElementSelector),
            ...(rest[1] as QueryOptions | undefined),
          },
        );
      },
      assertNot: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("assertNot", args, 1);
        return requestJson(
          endpoint,
          "POST",
          `/api/simulators/${encodeURIComponent(udid)}/assert-not`,
          {
            selector: selectorPayload(rest[0] as ElementSelector),
            ...(rest[1] as QueryOptions | undefined),
          },
        );
      },
      waitFor: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("waitFor", args, 1);
        return requestJson(
          endpoint,
          "POST",
          `/api/simulators/${encodeURIComponent(udid)}/wait-for`,
          {
            selector: selectorPayload(rest[0] as ElementSelector),
            ...(rest[1] as QueryOptions | undefined),
          },
        );
      },
      waitForNot: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("waitForNot", args, 1);
        return requestJson(
          endpoint,
          "POST",
          `/api/simulators/${encodeURIComponent(udid)}/wait-for-not`,
          {
            selector: selectorPayload(rest[0] as ElementSelector),
            ...(rest[1] as QueryOptions | undefined),
          },
        );
      },
      scrollUntilVisible: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("scrollUntilVisible", args, 1);
        return requestJson(
          endpoint,
          "POST",
          `/api/simulators/${encodeURIComponent(udid)}/scroll-until-visible`,
          {
            selector: selectorPayload(rest[0] as ElementSelector),
            ...(rest[1] as QueryOptions | undefined),
          },
        );
      },
      batch: (...args: unknown[]) => {
        const [udid, rest] = splitDeviceArgs("batch", args, 1);
        return requestJson(
          endpoint,
          "POST",
          `/api/simulators/${encodeURIComponent(udid)}/batch`,
          {
            steps: rest[0],
            continueOnError: rest[1] ?? false,
          },
        );
      },
      screenshot: (...args: unknown[]) => {
        const [udid] = splitDeviceNoArg("screenshot", args);
        return requestBuffer(
          endpoint,
          `/api/simulators/${encodeURIComponent(udid)}/screenshot.png`,
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
    } as SimDeckSession;
  };
  return createSession(options.udid);
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

function selectorPayload(
  selector: ElementSelector,
): Record<string, string | number | boolean | undefined> {
  return {
    text: selector.text,
    id: selector.id,
    label: selector.label,
    value: selector.value,
    elementType: selector.type,
    index: selector.index,
    enabled: selector.enabled,
    checked: selector.checked,
    focused: selector.focused,
    selected: selector.selected,
    regex: selector.regex,
  };
}
