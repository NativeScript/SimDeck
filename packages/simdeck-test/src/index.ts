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
    options?: QueryOptions & { timeoutMs?: number; pollMs?: number },
  ): Promise<unknown>;
  batch(
    udid: string,
    steps: unknown[],
    continueOnError?: boolean,
  ): Promise<unknown>;
  screenshot(udid: string): Promise<Buffer>;
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
  const session: SimDeckSession = {
    endpoint,
    pid: result.pid,
    projectRoot: result.projectRoot,
    list: () => requestJson(endpoint, "GET", "/api/simulators"),
    boot: (udid) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/boot`,
        null,
      ),
    shutdown: (udid) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/shutdown`,
        null,
      ),
    erase: (udid) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/erase`,
        null,
      ),
    install: (udid, appPath) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/install`,
        {
          appPath,
        },
      ),
    uninstall: (udid, bundleId) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/uninstall`,
        {
          bundleId,
        },
      ),
    launch: (udid, bundleId) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/launch`,
        {
          bundleId,
        },
      ),
    openUrl: (udid, url) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/open-url`,
        {
          url,
        },
      ),
    tap: (udid, x, y) =>
      requestOk(endpoint, `/api/simulators/${encodeURIComponent(udid)}/tap`, {
        x,
        y,
        normalized: true,
      }),
    tapElement: (udid, selector, tapOptions) =>
      requestOk(endpoint, `/api/simulators/${encodeURIComponent(udid)}/tap`, {
        selector: selectorPayload(selector),
        ...tapOptions,
      }),
    touch: (udid, x, y, phase) =>
      requestOk(endpoint, `/api/simulators/${encodeURIComponent(udid)}/touch`, {
        x,
        y,
        phase,
      }),
    swipe: (udid, startX, startY, endX, endY, swipeOptions = {}) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/batch`,
        {
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
        },
      ),
    gesture: (udid, preset, gestureOptions = {}) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/batch`,
        {
          steps: [
            {
              action: "gesture",
              preset,
              ...gestureOptions,
            },
          ],
        },
      ),
    typeText: (udid, text, typeOptions = {}) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/batch`,
        {
          steps: [
            {
              action: "type",
              text,
              ...typeOptions,
            },
          ],
        },
      ),
    key: (udid, keyCode, modifiers = 0) =>
      requestOk(endpoint, `/api/simulators/${encodeURIComponent(udid)}/key`, {
        keyCode,
        modifiers,
      }),
    keySequence: (udid, keyCodes, keySequenceOptions = {}) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/key-sequence`,
        {
          keyCodes,
          ...keySequenceOptions,
        },
      ),
    button: (udid, button, durationMs = 0) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/button`,
        {
          button,
          durationMs,
        },
      ),
    home: (udid) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/home`,
        null,
      ),
    dismissKeyboard: (udid) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/dismiss-keyboard`,
        null,
      ),
    appSwitcher: (udid) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/app-switcher`,
        null,
      ),
    rotateLeft: (udid) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/rotate-left`,
        null,
      ),
    rotateRight: (udid) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/rotate-right`,
        null,
      ),
    toggleAppearance: (udid) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/toggle-appearance`,
        null,
      ),
    pasteboardSet: (udid, text) =>
      requestOk(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/pasteboard`,
        {
          text,
        },
      ),
    pasteboardGet: async (udid) => {
      const result = await requestJson<{ text?: string }>(
        endpoint,
        "GET",
        `/api/simulators/${encodeURIComponent(udid)}/pasteboard`,
      );
      return result.text ?? "";
    },
    chromeProfile: (udid) =>
      requestJson(
        endpoint,
        "GET",
        `/api/simulators/${encodeURIComponent(udid)}/chrome-profile`,
      ),
    logs: async (udid, logsOptions) => {
      const result = await requestJson<{ entries?: unknown[] }>(
        endpoint,
        "GET",
        `/api/simulators/${encodeURIComponent(udid)}/logs?${logsQuery(logsOptions)}`,
      );
      return result.entries ?? [];
    },
    tree: (udid, treeOptions) =>
      requestJson(
        endpoint,
        "GET",
        `/api/simulators/${encodeURIComponent(udid)}/accessibility-tree?${treeQuery(treeOptions)}`,
      ),
    query: async (udid, selector, treeOptions) => {
      const result = await requestJson<{ matches: unknown[] }>(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/query`,
        {
          selector: selectorPayload(selector),
          ...treeOptions,
        },
      );
      return result.matches;
    },
    assert: (udid, selector, assertOptions) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/assert`,
        {
          selector: selectorPayload(selector),
          ...assertOptions,
        },
      ),
    waitFor: (udid, selector, waitOptions) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/wait-for`,
        {
          selector: selectorPayload(selector),
          ...waitOptions,
        },
      ),
    batch: (udid, steps, continueOnError = false) =>
      requestJson(
        endpoint,
        "POST",
        `/api/simulators/${encodeURIComponent(udid)}/batch`,
        {
          steps,
          continueOnError,
        },
      ),
    screenshot: (udid) =>
      requestBuffer(
        endpoint,
        `/api/simulators/${encodeURIComponent(udid)}/screenshot.png`,
      ),
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
