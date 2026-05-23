import { spawn, spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
const DEFAULT_QUERY_SOURCE = "native-ax";
export async function connect(options = {}) {
    const cliPath = options.cliPath ?? "simdeck";
    const result = options.isolated
        ? await startIsolatedDaemon(cliPath, options)
        : runJson(cliPath, ["daemon", "start"], {
            cwd: options.projectRoot,
        });
    const endpoint = result.url;
    const createSession = (defaultUdid) => {
        const simulatorPath = (udid, suffix) => `/api/simulators/${encodeURIComponent(udid)}${suffix}`;
        const actionPath = (udid) => simulatorPath(udid, "/action");
        const requestAction = (udid, body) => requestJson(endpoint, "POST", actionPath(udid), body);
        const requestActionOk = (udid, body) => requestAction(udid, body).then(() => undefined);
        const requireUdid = (udid) => {
            const resolved = udid ?? defaultUdid;
            if (!resolved) {
                throw new Error("This SimDeck session method requires a UDID. Pass one as the first argument or call connect({ udid }).");
            }
            return resolved;
        };
        const resolveNoArgDeviceCall = (args) => ({
            udid: requireUdid(typeof args[0] === "string" ? args[0] : undefined),
        });
        const resolveStringArgDeviceCall = (args) => {
            if (args.length >= 2 &&
                typeof args[0] === "string" &&
                typeof args[1] === "string") {
                return { udid: args[0], value: args[1], rest: args.slice(2) };
            }
            return {
                udid: requireUdid(),
                value: args[0],
                rest: args.slice(1),
            };
        };
        const resolveObjectArgDeviceCall = (args) => {
            if (typeof args[0] === "string") {
                return { udid: args[0], value: args[1], rest: args.slice(2) };
            }
            return { udid: requireUdid(), value: args[0], rest: args.slice(1) };
        };
        const resolveOptionalObjectDeviceCall = (args) => {
            if (typeof args[0] === "string") {
                return { udid: args[0], options: args[1] };
            }
            return { udid: requireUdid(), options: args[0] };
        };
        const session = {
            endpoint,
            pid: result.pid,
            projectRoot: result.projectRoot,
            udid: defaultUdid,
            device: (udid) => createSession(udid),
            list: () => requestJson(endpoint, "GET", "/api/simulators"),
            boot: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestJson(endpoint, "POST", simulatorPath(udid, "/boot"), null);
            },
            shutdown: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestJson(endpoint, "POST", simulatorPath(udid, "/shutdown"), null);
            },
            erase: (...args) => {
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
                return requestActionOk(udid, {
                    action: "launch",
                    bundleId,
                });
            },
            openUrl: (...args) => {
                const { udid, value: url } = resolveStringArgDeviceCall(args);
                return requestActionOk(udid, {
                    action: "openUrl",
                    url,
                });
            },
            tap: (...args) => {
                const [udid, x, y] = typeof args[0] === "string"
                    ? [args[0], args[1], args[2]]
                    : [requireUdid(), args[0], args[1]];
                return requestActionOk(udid, {
                    action: "tap",
                    x,
                    y,
                    normalized: true,
                });
            },
            tapElement: (...args) => {
                const { udid, value: selector, rest, } = resolveObjectArgDeviceCall(args);
                const [tapOptions] = rest;
                const { expect, expectTimeoutMs, expectMaxDepth, expectIncludeHidden, ...restOptions } = tapOptions ?? {};
                return requestActionOk(udid, {
                    action: "tap",
                    selector: selectorPayload(selector),
                    ...withQueryDefaults(restOptions),
                    expect: expect
                        ? {
                            selector: selectorPayload(expect),
                            source: tapOptions?.source ?? DEFAULT_QUERY_SOURCE,
                            maxDepth: expectMaxDepth ?? 8,
                            includeHidden: expectIncludeHidden,
                            timeoutMs: expectTimeoutMs,
                            pollMs: tapOptions?.pollMs,
                        }
                        : undefined,
                });
            },
            touch: (...args) => {
                const [udid, x, y, phase] = typeof args[0] === "string"
                    ? [args[0], args[1], args[2], args[3]]
                    : [
                        requireUdid(),
                        args[0],
                        args[1],
                        args[2],
                    ];
                return requestActionOk(udid, {
                    action: "touch",
                    x,
                    y,
                    phase,
                });
            },
            swipe: (...args) => {
                const [udid, startX, startY, endX, endY, swipeOptions = {}] = typeof args[0] === "string"
                    ? [
                        args[0],
                        args[1],
                        args[2],
                        args[3],
                        args[4],
                        args[5],
                    ]
                    : [
                        requireUdid(),
                        args[0],
                        args[1],
                        args[2],
                        args[3],
                        args[4],
                    ];
                return requestAction(udid, {
                    action: "swipe",
                    startX,
                    startY,
                    endX,
                    endY,
                    ...swipeOptions,
                });
            },
            gesture: (...args) => {
                const { udid, value: preset, rest } = resolveStringArgDeviceCall(args);
                const [gestureOptions = {}] = rest;
                return requestAction(udid, {
                    action: "gesture",
                    preset,
                    ...gestureOptions,
                });
            },
            typeText: (...args) => {
                const { udid, value: text, rest } = resolveStringArgDeviceCall(args);
                const [typeOptions = {}] = rest;
                return requestAction(udid, {
                    action: "type",
                    text,
                    ...typeOptions,
                });
            },
            key: (...args) => {
                const [udid, keyCode, modifiers = 0] = typeof args[0] === "string"
                    ? [args[0], args[1], args[2]]
                    : [requireUdid(), args[0], args[1]];
                return requestActionOk(udid, {
                    action: "key",
                    keyCode,
                    modifiers,
                });
            },
            keySequence: (...args) => {
                const { udid, value: keyCodes, rest, } = resolveObjectArgDeviceCall(args);
                const [keySequenceOptions = {}] = rest;
                return requestActionOk(udid, {
                    action: "keySequence",
                    keyCodes,
                    ...keySequenceOptions,
                });
            },
            button: (...args) => {
                const { udid, value: button, rest } = resolveStringArgDeviceCall(args);
                const [durationMs = 0] = rest;
                return requestActionOk(udid, {
                    action: "button",
                    button,
                    durationMs,
                });
            },
            home: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestActionOk(udid, { action: "home" });
            },
            back: (...args) => {
                const { udid, options } = resolveOptionalObjectDeviceCall(args);
                return requestActionOk(udid, { action: "back", ...options });
            },
            dismissKeyboard: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestActionOk(udid, { action: "dismissKeyboard" });
            },
            appSwitcher: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestActionOk(udid, { action: "appSwitcher" });
            },
            rotateLeft: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestActionOk(udid, { action: "rotateLeft" });
            },
            rotateRight: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestActionOk(udid, { action: "rotateRight" });
            },
            toggleAppearance: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestActionOk(udid, { action: "toggleAppearance" });
            },
            pasteboardSet: (...args) => {
                const { udid, value: text } = resolveStringArgDeviceCall(args);
                return requestOk(endpoint, simulatorPath(udid, "/pasteboard"), {
                    text,
                });
            },
            pasteboardGet: async (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                const result = await requestJson(endpoint, "GET", simulatorPath(udid, "/pasteboard"));
                return result.text ?? "";
            },
            chromeProfile: (...args) => {
                const { udid } = resolveNoArgDeviceCall(args);
                return requestJson(endpoint, "GET", simulatorPath(udid, "/chrome-profile"));
            },
            logs: async (...args) => {
                const { udid, value: logsOptions } = typeof args[0] === "string"
                    ? { udid: args[0], value: args[1] }
                    : {
                        udid: requireUdid(),
                        value: args[0],
                    };
                const result = await requestJson(endpoint, "GET", simulatorPath(udid, `/logs?${logsQuery(logsOptions)}`));
                return result.entries ?? [];
            },
            tree: (...args) => {
                const { udid, value: treeOptions } = typeof args[0] === "string"
                    ? { udid: args[0], value: args[1] }
                    : {
                        udid: requireUdid(),
                        value: args[0],
                    };
                return requestJson(endpoint, "GET", simulatorPath(udid, `/accessibility-tree?${treeQuery(treeOptions)}`));
            },
            query: async (...args) => {
                const { udid, value: selector, rest, } = resolveObjectArgDeviceCall(args);
                const [treeOptions] = rest;
                const result = await requestAction(udid, {
                    action: "query",
                    selector: selectorPayload(selector),
                    ...withQueryDefaults(treeOptions),
                });
                return result.result?.matches ?? result.matches ?? [];
            },
            assert: (...args) => {
                const { udid, value: selector, rest, } = resolveObjectArgDeviceCall(args);
                const [assertOptions] = rest;
                return requestAction(udid, {
                    action: "assert",
                    selector: selectorPayload(selector),
                    ...withQueryDefaults(assertOptions),
                });
            },
            assertNot: (...args) => {
                const { udid, value: selector, rest, } = resolveObjectArgDeviceCall(args);
                const [assertOptions] = rest;
                return requestAction(udid, {
                    action: "assertNot",
                    selector: selectorPayload(selector),
                    ...withQueryDefaults(assertOptions),
                });
            },
            waitFor: (...args) => {
                const { udid, value: selector, rest, } = resolveObjectArgDeviceCall(args);
                const [waitOptions] = rest;
                return requestAction(udid, {
                    action: "waitFor",
                    selector: selectorPayload(selector),
                    ...withQueryDefaults(waitOptions),
                });
            },
            waitForNot: (...args) => {
                const { udid, value: selector, rest, } = resolveObjectArgDeviceCall(args);
                const [waitOptions] = rest;
                return requestAction(udid, {
                    action: "assertNot",
                    selector: selectorPayload(selector),
                    ...withQueryDefaults(waitOptions),
                });
            },
            scrollUntilVisible: (...args) => {
                const { udid, value: selector, rest, } = resolveObjectArgDeviceCall(args);
                const [scrollOptions] = rest;
                return requestAction(udid, {
                    action: "scrollUntilVisible",
                    selector: selectorPayload(selector),
                    ...withQueryDefaults(scrollOptions),
                });
            },
            batch: (...args) => {
                const { udid, value: steps, rest, } = resolveObjectArgDeviceCall(args);
                const [continueOnError = false] = rest;
                return requestAction(udid, {
                    action: "batch",
                    steps,
                    continueOnError,
                });
            },
            screenshot: (...args) => {
                const { udid, options } = resolveOptionalObjectDeviceCall(args);
                const params = new URLSearchParams();
                if (options?.withBezel ?? options?.bezel) {
                    params.set("bezel", "true");
                }
                const query = params.toString();
                return requestBuffer(endpoint, simulatorPath(udid, `/screenshot.png${query ? `?${query}` : ""}`));
            },
            record: (...args) => {
                const { udid, options } = resolveOptionalObjectDeviceCall(args);
                return requestBuffer(endpoint, simulatorPath(udid, "/screen-recording"), "POST", {
                    seconds: options?.seconds ?? 5,
                });
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
    };
    return createSession(options.udid);
}
async function startIsolatedDaemon(cliPath, options) {
    const port = options.port ?? (await freePortPair());
    const projectRoot = fs.mkdtempSync(path.join(os.tmpdir(), "simdeck-test-project-"));
    const metadataPath = path.join(os.tmpdir(), `simdeck-test-${process.pid}-${Date.now()}-${crypto.randomUUID()}.json`);
    const accessToken = crypto.randomBytes(32).toString("hex");
    const child = spawn(cliPath, [
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
    ], {
        cwd: options.projectRoot,
        stdio: ["ignore", "pipe", "pipe"],
    });
    const output = captureChildOutput(child);
    const url = `http://127.0.0.1:${port}`;
    try {
        await waitForHealth(url, child, output);
    }
    catch (error) {
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
async function waitForHealth(endpoint, child, output) {
    const deadline = Date.now() + 60_000;
    let lastError;
    while (Date.now() < deadline) {
        if (child.exitCode !== null) {
            throw new Error(`SimDeck isolated daemon exited with ${child.exitCode}.\n${output()}`);
        }
        try {
            await requestJson(endpoint, "GET", "/api/health");
            return;
        }
        catch (error) {
            lastError = error;
            await new Promise((resolve) => setTimeout(resolve, 50));
        }
    }
    throw new Error(`Timed out waiting for isolated SimDeck daemon: ${lastError instanceof Error ? lastError.message : String(lastError)}\n${output()}`);
}
function captureChildOutput(child) {
    const chunks = [];
    const append = (source, chunk) => {
        chunks.push(`[${source}] ${chunk.toString("utf8")}`);
        while (chunks.join("").length > 16_384) {
            chunks.shift();
        }
    };
    child.stdout?.on("data", (chunk) => append("stdout", chunk));
    child.stderr?.on("data", (chunk) => append("stderr", chunk));
    return () => chunks.join("").trim();
}
async function freePortPair() {
    for (let attempt = 0; attempt < 100; attempt += 1) {
        const port = await freePort();
        if (port < 65535 && (await portAvailable(port + 1))) {
            return port;
        }
    }
    throw new Error("Unable to allocate adjacent free TCP ports.");
}
function freePort() {
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
function portAvailable(port) {
    return new Promise((resolve) => {
        const server = net.createServer();
        server.once("error", () => resolve(false));
        server.listen(port, "127.0.0.1", () => {
            server.close(() => resolve(true));
        });
    });
}
function runJson(command, args, options = {}) {
    const result = spawnSync(command, args, {
        cwd: options.cwd,
        encoding: "utf8",
        maxBuffer: 1024 * 1024,
    });
    if (result.status !== 0) {
        throw new Error(result.stderr.trim() || `${command} ${args.join(" ")} failed`);
    }
    return JSON.parse(result.stdout);
}
function requestOk(endpoint, pathName, body) {
    return requestJson(endpoint, "POST", pathName, body).then(() => undefined);
}
function requestJson(endpoint, method, pathName, body) {
    return requestBuffer(endpoint, pathName, method, body).then((buffer) => JSON.parse(buffer.toString("utf8")));
}
function requestBuffer(endpoint, pathName, method = "GET", body) {
    const url = new URL(pathName, endpoint);
    const payload = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
    return new Promise((resolve, reject) => {
        const request = http.request(url, {
            method,
            headers: payload
                ? {
                    "content-type": "application/json",
                    "content-length": String(payload.length),
                    origin: endpoint,
                }
                : { origin: endpoint },
        }, (response) => {
            const chunks = [];
            response.on("data", (chunk) => chunks.push(chunk));
            response.on("end", () => {
                const buffer = Buffer.concat(chunks);
                if ((response.statusCode ?? 500) < 200 ||
                    (response.statusCode ?? 500) >= 300) {
                    reject(new Error(`${method} ${pathName} returned ${response.statusCode}: ${buffer.toString("utf8") || response.statusMessage || ""}`));
                }
                else {
                    resolve(buffer);
                }
            });
        });
        request.on("error", reject);
        if (payload) {
            request.write(payload);
        }
        request.end();
    });
}
function treeQuery(options = {}) {
    const params = new URLSearchParams();
    params.set("source", options.source ?? DEFAULT_QUERY_SOURCE);
    if (options.maxDepth !== undefined)
        params.set("maxDepth", String(options.maxDepth));
    if (options.includeHidden)
        params.set("includeHidden", "true");
    if (options.interactiveOnly)
        params.set("interactiveOnly", "true");
    return params.toString();
}
function withQueryDefaults(options) {
    return { source: DEFAULT_QUERY_SOURCE, ...(options ?? {}) };
}
function logsQuery(options = {}) {
    const params = new URLSearchParams();
    if (options.backfill !== undefined)
        params.set("backfill", String(options.backfill));
    if (options.seconds !== undefined)
        params.set("seconds", String(options.seconds));
    if (options.limit !== undefined)
        params.set("limit", String(options.limit));
    if (options.levels?.length)
        params.set("levels", options.levels.join(","));
    if (options.processes?.length)
        params.set("processes", options.processes.join(","));
    if (options.q)
        params.set("q", options.q);
    return params.toString();
}
function selectorPayload(selector) {
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
