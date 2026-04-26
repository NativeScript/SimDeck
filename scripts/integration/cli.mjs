#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";

const root = path.resolve(new URL("../..", import.meta.url).pathname);
const simdeck = path.join(root, "build", "simdeck");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "simdeck-cli-it-"));
const serverPort = Number(process.env.SIMDECK_INTEGRATION_PORT ?? "4510");
const serverUrl = `http://127.0.0.1:${serverPort}`;
const origin = serverUrl;
const fixtureBundleId = "dev.nativescript.simdeck.integration.fixture";
const fixtureUrl = "simdeck-fixture://integration";
const verbose = process.env.SIMDECK_INTEGRATION_VERBOSE === "1";
const traceHttp = process.env.SIMDECK_INTEGRATION_TRACE_HTTP === "1";
const showSimulator = process.env.SIMDECK_INTEGRATION_SHOW_SIMULATOR === "1";
const keepSimulator = process.env.SIMDECK_INTEGRATION_KEEP_SIMULATOR === "1";
const cliCommandBudgetMs = Number(
  process.env.SIMDECK_INTEGRATION_CLI_BUDGET_MS ?? "10000",
);
const describeUiBudgetMs = Number(
  process.env.SIMDECK_INTEGRATION_DESCRIBE_UI_BUDGET_MS ?? "10000",
);
const httpActionBudgetMs = Number(
  process.env.SIMDECK_INTEGRATION_HTTP_BUDGET_MS ?? "10000",
);

let simulatorUDID = "";
let serverProcess = null;

process.on("SIGINT", () => {
  cleanup();
  process.exit(130);
});
process.on("SIGTERM", () => {
  cleanup();
  process.exit(143);
});
process.on("exit", cleanup);

main()
  .then(() => {
    cleanup();
    process.exit(0);
  })
  .catch((error) => {
    console.error(error?.stack ?? error);
    cleanup();
    process.exit(1);
  });

async function main() {
  if (process.platform !== "darwin") {
    throw new Error("SimDeck CLI integration tests require macOS.");
  }
  if (!fs.existsSync(simdeck)) {
    throw new Error(`Missing ${simdeck}. Run npm run build:cli first.`);
  }

  const runtime = latestAvailableIosRuntime();
  const deviceType = preferredIphoneDeviceType(runtime);
  const simulatorName = `SimDeck CLI Integration ${Date.now()}`;
  simulatorUDID = runText("xcrun", [
    "simctl",
    "create",
    simulatorName,
    deviceType.identifier,
    runtime.identifier,
  ]).trim();

  console.log(
    `created ${simulatorUDID} (${deviceType.name}, ${runtime.version})`,
  );
  simdeckJson(["boot", simulatorUDID]);
  runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
    timeoutMs: 600_000,
  });
  if (showSimulator) {
    openSimulatorApp(simulatorUDID);
  }

  const fixture = buildFixtureApp();

  assertSimulatorListed(simulatorUDID);
  assertJson(simdeckJson(["chrome-profile", simulatorUDID]), "chrome-profile");
  assertJson(
    simdeckJson(["logs", simulatorUDID, "--seconds", "5", "--limit", "5"]),
    "logs",
  );

  simdeckJson(["install", simulatorUDID, fixture.appPath]);
  await verifyUi("after install");

  startServer();
  await waitForHealth();
  logStep(`server ready at ${serverUrl}`);

  const fullTree = await retrySimdeckJson(
    ["describe-ui", simulatorUDID, "--direct", "--max-depth", "2"],
    "direct describe-ui json",
  );
  assertRoots(fullTree, "direct describe-ui json");
  const queryPoint = pointFromSnapshot(fullTree);
  assertJson(
    await retrySimdeckJson(
      [
        "describe-ui",
        simulatorUDID,
        "--direct",
        "--format",
        "compact-json",
        "--max-depth",
        "2",
      ],
      "direct describe-ui compact-json",
    ),
    "direct describe-ui compact-json",
  );
  const agentTree = simdeckText([
    "describe-ui",
    simulatorUDID,
    "--server-url",
    serverUrl,
    "--format",
    "agent",
    "--max-depth",
    "2",
  ]);
  if (!agentTree.includes("source:") || !agentTree.includes("- ")) {
    throw new Error("agent describe-ui output did not look like a hierarchy");
  }
  assertRoots(
    await retrySimdeckJson(
      [
        "describe-ui",
        simulatorUDID,
        "--point",
        `${queryPoint.x},${queryPoint.y}`,
        "--format",
        "compact-json",
        "--direct",
      ],
      "point describe-ui compact-json",
    ),
    "point describe-ui compact-json",
  );

  await runRestControls(queryPoint);
  await runCliControls();

  const screenshotPath = path.join(tempRoot, "screen.png");
  simdeckJson(["screenshot", simulatorUDID, "--output", screenshotPath]);
  assertPng(screenshotPath);
  await verifyUi("after screenshot file");
  const stdoutPng = path.join(tempRoot, "screen-stdout.png");
  fs.writeFileSync(
    stdoutPng,
    runBuffer(simdeck, ["screenshot", simulatorUDID, "--stdout"], {
      timeoutMs: 300_000,
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
  assertPng(stdoutPng);
  await verifyUi("after screenshot stdout");

  simdeckJson(["pasteboard", "set", simulatorUDID, "simdeck integration"]);
  await verifyUi("after pasteboard set");
  const pasteboard = simdeckJson(["pasteboard", "get", simulatorUDID]);
  if (pasteboard.text !== "simdeck integration") {
    throw new Error(
      `pasteboard round-trip failed: ${JSON.stringify(pasteboard)}`,
    );
  }
  await verifyUi("after pasteboard get");

  const fileInput = path.join(tempRoot, "type.txt");
  fs.writeFileSync(fileInput, "file input");
  simdeckJson(["type", simulatorUDID, "--file", fileInput]);
  await verifyUi("after type file");
  simdeckJson(["type", simulatorUDID, "--stdin"], {
    input: "stdin input",
  });
  await verifyUi("after type stdin");

  const batch = simdeckJson([
    "batch",
    simulatorUDID,
    "--step",
    "button home",
    "--step",
    "tap --x 200 --y 700 --duration-ms 20",
    "--step",
    "type batch",
    "--continue-on-error",
  ]);
  if (batch.ok !== true || batch.failureCount !== 0) {
    throw new Error(`batch command failed: ${JSON.stringify(batch)}`);
  }
  await verifyUi("after batch");

  await runHardwareButtonControls();

  simdeckJson(["uninstall", simulatorUDID, fixtureBundleId]);
  simdeckJson(["shutdown", simulatorUDID]);
  simdeckJson(["erase", simulatorUDID]);
  simdeckJson(["boot", simulatorUDID]);
  runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
    timeoutMs: 600_000,
  });
  assertRoots(
    await retrySimdeckJson(
      [
        "describe-ui",
        simulatorUDID,
        "--direct",
        "--format",
        "compact-json",
        "--max-depth",
        "1",
      ],
      "post-erase describe-ui",
    ),
    "post-erase describe-ui",
  );

  console.log("SimDeck CLI integration suite passed");
}

async function runCliControls() {
  await cliStep("CLI home", ["home", simulatorUDID]);
  await cliStep("CLI tap", [
    "tap",
    simulatorUDID,
    "200",
    "700",
    "--duration-ms",
    "20",
  ]);
  await cliStep("CLI touch began", [
    "touch",
    simulatorUDID,
    "0.5",
    "0.5",
    "--phase",
    "began",
    "--normalized",
  ]);
  await cliStep("CLI touch ended", [
    "touch",
    simulatorUDID,
    "0.5",
    "0.5",
    "--phase",
    "ended",
    "--normalized",
  ]);
  await cliStep("CLI touch down/up", [
    "touch",
    simulatorUDID,
    "120",
    "240",
    "--down",
    "--up",
    "--delay-ms",
    "20",
  ]);
  await cliStep("CLI swipe", [
    "swipe",
    simulatorUDID,
    "200",
    "700",
    "200",
    "300",
    "--duration-ms",
    "100",
    "--steps",
    "4",
  ]);
  await cliStep("CLI gesture scroll-down", [
    "gesture",
    simulatorUDID,
    "scroll-down",
    "--duration-ms",
    "100",
    "--delta",
    "100",
  ]);
  await cliStep("CLI gesture edge swipe", [
    "gesture",
    simulatorUDID,
    "swipe-from-left-edge",
    "--duration-ms",
    "100",
  ]);
  await cliStep("CLI pinch", [
    "pinch",
    simulatorUDID,
    "--start-distance",
    "0.15",
    "--end-distance",
    "0.25",
    "--normalized",
    "--duration-ms",
    "100",
    "--steps",
    "4",
  ]);
  await cliStep("CLI rotate gesture", [
    "rotate-gesture",
    simulatorUDID,
    "--radius",
    "0.10",
    "--degrees",
    "30",
    "--normalized",
    "--duration-ms",
    "100",
    "--steps",
    "4",
  ]);
  await cliStep("CLI key enter", ["key", simulatorUDID, "enter"]);
  await cliStep("CLI key sequence", [
    "key-sequence",
    simulatorUDID,
    "--keycodes",
    "h,e,l,l,o",
    "--delay-ms",
    "5",
  ]);
  await cliStep("CLI key combo", [
    "key-combo",
    simulatorUDID,
    "--modifiers",
    "cmd",
    "--key",
    "a",
  ]);
  await cliStep("CLI type", ["type", simulatorUDID, "qa"]);
  await cliStep("CLI dismiss keyboard", ["dismiss-keyboard", simulatorUDID]);
  await cliStep("CLI app switcher", ["app-switcher", simulatorUDID]);
  await cliStep("CLI home after switcher", ["home", simulatorUDID]);
  await cliStep("CLI rotate left", ["rotate-left", simulatorUDID]);
  await cliStep("CLI rotate right", ["rotate-right", simulatorUDID]);
  await cliStep("CLI toggle appearance", ["toggle-appearance", simulatorUDID]);
  await cliStep(
    "CLI launch SwiftUI fixture",
    ["launch", simulatorUDID, fixtureBundleId],
    { attempts: 3, delayMs: 5_000, timeoutMs: 180_000 },
    { expectFixture: true },
  );
  await cliStep(
    "CLI open fixture URL",
    ["open-url", simulatorUDID, fixtureUrl],
    {
      attempts: 3,
      delayMs: 5_000,
      timeoutMs: 180_000,
      maxElapsedMs: 20_000,
    },
    { expectFixture: true, expectText: "URL Opened" },
  );
  await cliStep(
    "CLI focus fixture text field",
    ["tap", simulatorUDID, "--id", "fixture.message", "--wait-timeout-ms", "5000"],
    {},
    { expectFixture: true },
  );
  await cliStep(
    "CLI focus fixture text field normalized",
    [
      "tap",
      simulatorUDID,
      "0.708",
      "0.5",
      "--normalized",
      "--duration-ms",
      "120",
      "--post-delay-ms",
      "500",
    ],
    {},
    { expectFixture: true },
  );
  await cliStep(
    "CLI type fixture text",
    ["type", simulatorUDID, "agent-ready"],
    {},
    { expectFixture: true, expectText: "agent-ready" },
  );
}

async function runHardwareButtonControls() {
  await cliStep("CLI button home", ["button", simulatorUDID, "home"]);
  await cliStep("CLI button lock", [
    "button",
    simulatorUDID,
    "lock",
    "--duration-ms",
    "50",
  ]);
  await cliStep("CLI button lock wake", [
    "button",
    simulatorUDID,
    "lock",
    "--duration-ms",
    "50",
  ]);
  await cliStep("CLI button side", [
    "button",
    simulatorUDID,
    "side-button",
    "--duration-ms",
    "50",
  ]);
  await cliStep("CLI button side repeat", [
    "button",
    simulatorUDID,
    "side-button",
    "--duration-ms",
    "50",
  ]);
  await cliStep("CLI button siri", [
    "button",
    simulatorUDID,
    "siri",
    "--duration-ms",
    "50",
  ]);
  await cliStep("CLI home after siri", ["home", simulatorUDID]);
  await cliStep("CLI button apple-pay", [
    "button",
    simulatorUDID,
    "apple-pay",
    "--duration-ms",
    "50",
  ]);
  await cliStep("CLI home after apple-pay", ["home", simulatorUDID]);
}

async function runRestControls(queryPoint) {
  const simulators = await httpJson("GET", "/api/simulators");
  if (
    !simulators.simulators?.some(
      (simulator) => simulator.udid === simulatorUDID,
    )
  ) {
    throw new Error("REST simulator list did not include temp simulator");
  }
  assertRoots(
    await httpJson(
      "GET",
      `/api/simulators/${simulatorUDID}/accessibility-tree?maxDepth=1`,
    ),
    "REST accessibility-tree",
  );
  assertRoots(
    await httpJson(
      "GET",
      `/api/simulators/${simulatorUDID}/accessibility-point?x=${queryPoint.x}&y=${queryPoint.y}`,
    ),
    "REST accessibility-point",
  );
  assertJson(
    await httpJson("GET", `/api/simulators/${simulatorUDID}/chrome-profile`),
    "REST chrome-profile",
  );
  assertPngBuffer(
    await httpBuffer("GET", `/api/simulators/${simulatorUDID}/chrome.png`),
  );

  await httpStep(
    "REST rotate-left",
    "POST",
    `/api/simulators/${simulatorUDID}/rotate-left`,
    {},
  );
  await httpStep(
    "REST rotate-right",
    "POST",
    `/api/simulators/${simulatorUDID}/rotate-right`,
    {},
  );
  await httpStep(
    "REST toggle appearance",
    "POST",
    `/api/simulators/${simulatorUDID}/toggle-appearance`,
    {},
  );
  await httpStep(
    "REST launch SwiftUI fixture",
    "POST",
    `/api/simulators/${simulatorUDID}/launch`,
    { bundleId: fixtureBundleId },
    { attempts: 3, delayMs: 5_000 },
    { expectFixture: true },
  );
  await httpStep(
    "REST open fixture URL",
    "POST",
    `/api/simulators/${simulatorUDID}/open-url`,
    { url: fixtureUrl },
    { attempts: 3, delayMs: 5_000 },
    { expectFixture: true, expectText: "URL Opened" },
  );

  await httpStep(
    "REST touch began",
    "POST",
    `/api/simulators/${simulatorUDID}/touch`,
    {
      x: 0.5,
      y: 0.525,
      phase: "began",
    },
  );
  await httpStep(
    "REST touch ended",
    "POST",
    `/api/simulators/${simulatorUDID}/touch`,
    {
      x: 0.5,
      y: 0.525,
      phase: "ended",
    },
    {},
    { expectText: "Continue Tapped 1" },
  );
  await httpStep(
    "REST key enter",
    "POST",
    `/api/simulators/${simulatorUDID}/key`,
    {
      keyCode: 40,
      modifiers: 0,
    },
  );
  await httpStep(
    "REST home",
    "POST",
    `/api/simulators/${simulatorUDID}/home`,
    {},
  );
  await httpStep(
    "REST app-switcher",
    "POST",
    `/api/simulators/${simulatorUDID}/app-switcher`,
    {},
  );
}

function latestAvailableIosRuntime() {
  const payload = runJson("xcrun", ["simctl", "list", "runtimes", "-j"]);
  const runtimes = payload.runtimes
    .filter(
      (runtime) => runtime.isAvailable && runtime.identifier?.includes("iOS"),
    )
    .sort(compareRuntimeVersions);
  const runtime = runtimes.at(-1);
  if (!runtime) {
    throw new Error("No available iOS simulator runtime found.");
  }
  return runtime;
}

function preferredIphoneDeviceType(runtime) {
  const runtimeSupported = Array.isArray(runtime.supportedDeviceTypes)
    ? runtime.supportedDeviceTypes
    : [];
  const allDeviceTypes = runJson("xcrun", [
    "simctl",
    "list",
    "devicetypes",
    "-j",
  ]).devicetypes;
  const supported =
    runtimeSupported.length > 0
      ? runtimeSupported
      : allDeviceTypes.filter(
          (device) =>
            device.productFamily === "iPhone" ||
            device.identifier?.includes("iPhone"),
        );
  const iphones = supported.filter(
    (device) =>
      device.productFamily === "iPhone" ||
      device.identifier?.includes("iPhone"),
  );
  const preferred = [
    "iPhone 17",
    "iPhone 16",
    "iPhone 15",
    "iPhone 14",
    "iPhone 13",
  ];
  for (const name of preferred) {
    const match = iphones.find((device) => device.name === name);
    if (match) {
      return match;
    }
  }
  const fallback = iphones[0];
  if (!fallback) {
    throw new Error(
      `Runtime ${runtime.identifier} does not support an iPhone device.`,
    );
  }
  return fallback;
}

function compareRuntimeVersions(left, right) {
  const leftParts = String(left.version ?? "0")
    .split(".")
    .map(Number);
  const rightParts = String(right.version ?? "0")
    .split(".")
    .map(Number);
  for (
    let index = 0;
    index < Math.max(leftParts.length, rightParts.length);
    index += 1
  ) {
    const delta = (leftParts[index] ?? 0) - (rightParts[index] ?? 0);
    if (delta !== 0) {
      return delta;
    }
  }
  return String(left.identifier).localeCompare(String(right.identifier));
}

function buildFixtureApp() {
  const appPath = path.join(tempRoot, "SimDeckFixture.app");
  fs.mkdirSync(appPath, { recursive: true });
  const executable = "SimDeckFixture";
  fs.writeFileSync(
    path.join(appPath, "Info.plist"),
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>${executable}</string>
  <key>CFBundleIdentifier</key><string>${fixtureBundleId}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>SimDeckFixture</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>MinimumOSVersion</key><string>15.0</string>
  <key>UIDeviceFamily</key><array><integer>1</integer></array>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>SimDeckFixture</string>
      <key>CFBundleURLSchemes</key>
      <array><string>simdeck-fixture</string></array>
    </dict>
  </array>
</dict>
</plist>
`,
  );
  const main = path.join(tempRoot, "SimDeckFixture.swift");
  fs.writeFileSync(
    main,
    `import SwiftUI

struct FixtureView: View {
  @State private var status = "Integration Ready"
  @State private var tapCount = 0
  @State private var message = ""
  @FocusState private var messageFocused: Bool

  var body: some View {
    VStack(spacing: 24) {
      Text("SimDeck Fixture")
        .font(.title2)
        .accessibilityIdentifier("fixture.title")

      Text(status)
        .accessibilityIdentifier("fixture.status")

      Button("Continue") {
        tapCount += 1
        status = "Continue Tapped \\(tapCount)"
      }
        .buttonStyle(.borderedProminent)
        .accessibilityIdentifier("fixture.continue")

      TextField("Message", text: $message)
        .textFieldStyle(.roundedBorder)
        .accessibilityIdentifier("fixture.message")
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled(true)
        .focused($messageFocused)
        .frame(width: 240)
    }
    .padding()
    .onOpenURL { _ in
      status = "URL Opened"
    }
  }
}

@main
struct SimDeckFixtureApp: App {
  var body: some Scene {
    WindowGroup {
      FixtureView()
    }
  }
}
`,
  );
  const targetArch = process.arch === "arm64" ? "arm64" : "x86_64";
  runText("xcrun", [
    "--sdk",
    "iphonesimulator",
    "swiftc",
    "-target",
    `${targetArch}-apple-ios15.0-simulator`,
    "-parse-as-library",
    "-O",
    "-framework",
    "SwiftUI",
    "-framework",
    "UIKit",
    main,
    "-o",
    path.join(appPath, executable),
  ]);
  return { appPath };
}

function startServer() {
  killPortListeners(serverPort);
  logStep(`starting server on ${serverUrl}`);
  serverProcess = spawn(
    simdeck,
    [
      "serve",
      "--port",
      String(serverPort),
      "--client-root",
      path.join(root, "client", "dist"),
      "--video-codec",
      "h264-software",
    ],
    {
      cwd: root,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  serverProcess.stdout.on("data", (data) =>
    process.stdout.write(`[serve] ${data}`),
  );
  serverProcess.stderr.on("data", (data) =>
    process.stderr.write(`[serve] ${data}`),
  );
}

async function waitForHealth() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const health = await httpJson("GET", "/api/health");
      if (health.httpPort === serverPort) {
        return;
      }
    } catch {
      await sleep(250);
    }
  }
  throw new Error("Timed out waiting for SimDeck integration server.");
}

function simdeckJson(args, options = {}) {
  return JSON.parse(simdeckText(args, options));
}

async function retrySimdeckJson(args, label, options = {}) {
  const attempts = options.attempts ?? 6;
  const delayMs = options.delayMs ?? 2_000;
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return simdeckJson(args, options);
    } catch (error) {
      lastError = error;
      if (attempt === attempts) {
        break;
      }
      await sleep(delayMs);
    }
  }
  throw new Error(
    `${label} failed after ${attempts} attempts: ${lastError?.message ?? lastError}`,
  );
}

async function retrySimdeckText(args, label, options = {}) {
  const attempts = options.attempts ?? 6;
  const delayMs = options.delayMs ?? 2_000;
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return simdeckText(args, options);
    } catch (error) {
      lastError = error;
      if (attempt === attempts) {
        break;
      }
      await sleep(delayMs);
    }
  }
  throw new Error(
    `${label} failed after ${attempts} attempts: ${lastError?.message ?? lastError}`,
  );
}

async function cliStep(label, args, commandOptions = {}, verifyOptions = {}) {
  const result = await retrySimdeckJson(args, label, {
    maxElapsedMs: cliCommandBudgetMs,
    ...commandOptions,
  });
  await verifyUi(label, verifyOptions);
  return result;
}

async function httpStep(
  label,
  method,
  requestPath,
  body,
  requestOptions = {},
  verifyOptions = {},
) {
  logStep(`${label}`);
  const result = await retryHttpJson(
    method,
    requestPath,
    body,
    label,
    requestOptions,
  );
  await verifyUi(label, verifyOptions);
  return result;
}

async function verifyUi(label, options = {}) {
  const attempts = options.attempts ?? (options.expectFixture ? 8 : 3);
  const delayMs = options.delayMs ?? (options.expectFixture ? 3_000 : 1_000);
  let lastSnapshot = "";
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    let snapshot;
    try {
      snapshot = await retrySimdeckText(
        [
          "describe-ui",
          simulatorUDID,
          "--direct",
          "--format",
          "agent",
          "--max-depth",
          "5",
        ],
        `${label} describe-ui`,
        {
          attempts: 1,
          timeoutMs: 90_000,
          maxElapsedMs: options.describeMaxElapsedMs ?? describeUiBudgetMs,
        },
      );
    } catch (error) {
      lastSnapshot = error?.message ?? String(error);
      logStep(`ui after ${label}: describe-ui failed, retrying`);
      if (attempt < attempts) {
        await sleep(delayMs);
        continue;
      }
      break;
    }
    snapshot = await resolveKnownSystemPrompts(snapshot, label);
    lastSnapshot = snapshot;
    logStep(`ui after ${label}: ${summarizeUi(snapshot)}`);
    const fixtureOk = !options.expectFixture || fixtureReady(snapshot);
    const textOk = !options.expectText || snapshot.includes(options.expectText);
    if (fixtureOk && textOk) {
      return snapshot;
    }
    if (attempt < attempts) {
      await sleep(delayMs);
    }
  }
  throw new Error(
    `${label} did not reach expected UI after ${attempts} UI checks:\n${lastSnapshot}`,
  );
}

async function resolveKnownSystemPrompts(snapshot, label) {
  if (!looksLikeOpenUrlPrompt(snapshot) && !looksLikeKeyboardTipPrompt(snapshot)) {
    return snapshot;
  }
  const promptKind = looksLikeOpenUrlPrompt(snapshot)
    ? "open-url"
    : "keyboard-tip";
  logStep(`handling system ${promptKind} prompt after ${label}`);
  let current = snapshot;
  const actions =
    promptKind === "open-url"
      ? openUrlPromptActions(snapshot)
      : keyboardTipPromptActions(snapshot);
  for (const action of actions) {
    logStep(`trying prompt action: ${action.label}`);
    action.run();
    await sleep(1_500);
    current = await retrySimdeckText(
      [
        "describe-ui",
        simulatorUDID,
        "--direct",
        "--format",
        "agent",
        "--max-depth",
        "5",
      ],
      `${label} describe-ui after ${action.label}`,
      {
        attempts: 3,
        delayMs: 1_000,
        timeoutMs: 90_000,
        maxElapsedMs: describeUiBudgetMs,
      },
    );
    if (!looksLikeOpenUrlPrompt(current) && !looksLikeKeyboardTipPrompt(current)) {
      logStep(`system ${promptKind} prompt cleared by ${action.label}`);
      return current;
    }
  }
  return current;
}

function openUrlPromptActions(snapshot) {
  const actions = [
    {
      label: "key enter",
      run: () => simdeckJson(["key", simulatorUDID, "enter"], { timeoutMs: 60_000 }),
    },
    {
      label: "tap Open by label",
      run: () =>
        simdeckJson(
          ["tap", simulatorUDID, "--label", "Open", "--wait-timeout-ms", "5000"],
          { timeoutMs: 60_000 },
        ),
    },
  ];
  for (const point of openButtonCandidatePoints(snapshot)) {
    actions.push({
      label: `tap Open at ${point.x},${point.y}`,
      run: () =>
        simdeckJson(
          [
            "tap",
            simulatorUDID,
            String(point.x),
            String(point.y),
            "--duration-ms",
            "80",
          ],
          { timeoutMs: 60_000 },
        ),
    });
  }
  for (const point of openButtonCandidateNormalizedPoints(snapshot)) {
    actions.push({
      label: `tap Open normalized ${point.x.toFixed(3)},${point.y.toFixed(3)}`,
      run: () =>
        simdeckJson(
          [
            "tap",
            simulatorUDID,
            String(point.x),
            String(point.y),
            "--normalized",
            "--duration-ms",
            "80",
          ],
          { timeoutMs: 60_000 },
        ),
    });
  }
  return actions;
}

function keyboardTipPromptActions(snapshot) {
  const actions = [
    {
      label: "key enter",
      run: () =>
        simdeckJson(["key", simulatorUDID, "enter"], { timeoutMs: 60_000 }),
    },
    {
      label: "tap keyboard tip Continue by label",
      run: () =>
        simdeckJson(
          [
            "tap",
            simulatorUDID,
            "--label",
            "Continue",
            "--wait-timeout-ms",
            "5000",
          ],
          { timeoutMs: 60_000 },
        ),
    },
  ];
  for (const point of buttonCandidatePoints(snapshot, "Continue")) {
    actions.push({
      label: `tap keyboard tip Continue at ${point.x},${point.y}`,
      run: () =>
        simdeckJson(
          [
            "tap",
            simulatorUDID,
            String(point.x),
            String(point.y),
            "--duration-ms",
            "80",
          ],
          { timeoutMs: 60_000 },
        ),
    });
  }
  return actions;
}

function openButtonPoint(snapshot) {
  return buttonPoint(snapshot, "Open");
}

function buttonPoint(snapshot, label) {
  const escapedLabel = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = snapshot.match(
    new RegExp(
      `Button(?:\\s+#[^:\\n]+)?:\\s+${escapedLabel}\\s+@([0-9.]+),([0-9.]+)\\s+([0-9.]+)x([0-9.]+)`,
    ),
  );
  if (!match) {
    return null;
  }
  return {
    x: Math.round(Number(match[1]) + Number(match[3]) / 2),
    y: Math.round(Number(match[2]) + Number(match[4]) / 2),
  };
}

function buttonCandidatePoints(snapshot, label) {
  const point = buttonPoint(snapshot, label);
  if (!point) {
    return [];
  }
  const bounds = rootBounds(snapshot);
  const candidates = [
    point,
    { x: point.y, y: point.x },
  ];
  if (bounds) {
    candidates.push(
      { x: bounds.width - point.x, y: point.y },
      { x: point.x, y: bounds.height - point.y },
      { x: point.y, y: bounds.width - point.x },
      { x: bounds.height - point.y, y: point.x },
    );
  }
  return uniquePoints(candidates).filter(
    (candidate) => candidate.x >= 0 && candidate.y >= 0,
  );
}

function openButtonCandidatePoints(snapshot) {
  const point = openButtonPoint(snapshot);
  if (!point) {
    return [];
  }
  const bounds = rootBounds(snapshot);
  const candidates = [
    point,
    { x: point.y, y: point.x },
  ];
  if (bounds) {
    candidates.push(
      { x: bounds.width - point.x, y: point.y },
      { x: point.x, y: bounds.height - point.y },
      { x: point.y, y: bounds.width - point.x },
      { x: bounds.height - point.y, y: point.x },
    );
  }
  return uniquePoints(candidates).filter(
    (candidate) => candidate.x >= 0 && candidate.y >= 0,
  );
}

function openButtonCandidateNormalizedPoints(snapshot) {
  const point = openButtonPoint(snapshot);
  const bounds = rootBounds(snapshot);
  if (!point || !bounds || bounds.width <= 0 || bounds.height <= 0) {
    return [];
  }
  const x = point.x / bounds.width;
  const y = point.y / bounds.height;
  return uniqueUnitPoints([
    { x, y },
    { x: y, y: x },
    { x: y, y: 1 - x },
    { x: 1 - y, y: x },
    { x: 1 - x, y },
    { x, y: 1 - y },
  ]).filter((candidate) => candidate.x >= 0 && candidate.y >= 0);
}

function rootBounds(snapshot) {
  const match = snapshot.match(
    /Application(?:\s+#[^:\n]+)?:?.*?@([0-9.]+),([0-9.]+)\s+([0-9.]+)x([0-9.]+)/,
  );
  if (!match) {
    return null;
  }
  return {
    x: Number(match[1]),
    y: Number(match[2]),
    width: Number(match[3]),
    height: Number(match[4]),
  };
}

function uniquePoints(points) {
  const seen = new Set();
  const unique = [];
  for (const point of points) {
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) {
      continue;
    }
    const rounded = {
      x: Math.round(point.x),
      y: Math.round(point.y),
    };
    const key = `${rounded.x},${rounded.y}`;
    if (!seen.has(key)) {
      seen.add(key);
      unique.push(rounded);
    }
  }
  return unique;
}

function uniqueUnitPoints(points) {
  const seen = new Set();
  const unique = [];
  for (const point of points) {
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) {
      continue;
    }
    const rounded = {
      x: Math.max(0, Math.min(1, Number(point.x.toFixed(4)))),
      y: Math.max(0, Math.min(1, Number(point.y.toFixed(4)))),
    };
    const key = `${rounded.x},${rounded.y}`;
    if (!seen.has(key)) {
      seen.add(key);
      unique.push(rounded);
    }
  }
  return unique;
}

function looksLikeOpenUrlPrompt(snapshot) {
  return (
    /\bOpen\b/.test(snapshot) &&
    /(SimDeckFixture|simdeck-fixture|fixture|integration)/i.test(snapshot)
  );
}

function looksLikeKeyboardTipPrompt(snapshot) {
  return (
    /Speed up your typing/i.test(snapshot) &&
    /Button(?:\s+#[^:\n]+)?:\s+Continue\b/.test(snapshot)
  );
}

function fixtureReady(snapshot) {
  return (
    snapshot.includes("SimDeck Fixture") &&
    snapshot.includes("fixture.status") &&
    snapshot.includes("fixture.continue") &&
    snapshot.includes("fixture.message")
  );
}

function summarizeUi(snapshot) {
  const lines = snapshot
    .split("\n")
    .filter(
      (line) => line.startsWith("source:") || line.trim().startsWith("- "),
    )
    .slice(0, 6);
  return lines.join(" | ").slice(0, 500);
}

function simdeckText(args, options = {}) {
  return runText(simdeck, args, {
    timeoutMs: options.timeoutMs ?? 120_000,
    maxElapsedMs: options.maxElapsedMs,
    input: options.input,
  });
}

function runJson(command, args, options = {}) {
  return JSON.parse(runText(command, args, options));
}

function runText(command, args, options = {}) {
  const startedAt = Date.now();
  logCommand(command, args);
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    input: options.input,
    timeout: options.timeoutMs ?? 120_000,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with ${result.status ?? result.signal}\n${result.stdout}\n${result.stderr}`,
    );
  }
  const elapsedMs = Date.now() - startedAt;
  if (options.maxElapsedMs && elapsedMs > options.maxElapsedMs) {
    throw new Error(
      `${command} ${args.join(" ")} took ${elapsedMs}ms, above ${options.maxElapsedMs}ms budget`,
    );
  }
  logCommandResult(command, args, elapsedMs, result.stdout);
  return result.stdout;
}

function runBuffer(command, args, options = {}) {
  const startedAt = Date.now();
  logCommand(command, args);
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "buffer",
    maxBuffer: options.maxBuffer ?? 16 * 1024 * 1024,
    timeout: options.timeoutMs ?? 120_000,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with ${result.status ?? result.signal}\n${result.stderr?.toString("utf8") ?? ""}\n${result.error?.message ?? ""}`,
    );
  }
  const elapsedMs = Date.now() - startedAt;
  if (options.maxElapsedMs && elapsedMs > options.maxElapsedMs) {
    throw new Error(
      `${command} ${args.join(" ")} took ${elapsedMs}ms, above ${options.maxElapsedMs}ms budget`,
    );
  }
  logCommandResult(
    command,
    args,
    elapsedMs,
    `<${result.stdout.length} bytes>`,
  );
  return result.stdout;
}

async function httpJson(method, requestPath, body, options = {}) {
  const buffer = await httpBuffer(method, requestPath, body, options);
  return JSON.parse(buffer.toString("utf8"));
}

async function retryHttpJson(method, requestPath, body, label, options = {}) {
  const attempts = options.attempts ?? 3;
  const delayMs = options.delayMs ?? 2_000;
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await httpJson(method, requestPath, body, {
        maxElapsedMs: options.maxElapsedMs ?? httpActionBudgetMs,
      });
    } catch (error) {
      lastError = error;
      if (attempt === attempts) {
        break;
      }
      await sleep(delayMs);
    }
  }
  throw new Error(
    `${label} failed after ${attempts} attempts: ${lastError?.message ?? lastError}`,
  );
}

function httpBuffer(method, requestPath, body, options = {}) {
  const payload = body === undefined ? null : Buffer.from(JSON.stringify(body));
  const startedAt = Date.now();
  logHttp(method, requestPath, body);
  return new Promise((resolve, reject) => {
    const request = http.request(
      {
        hostname: "127.0.0.1",
        port: serverPort,
        path: requestPath,
        method,
        headers: {
          Origin: origin,
          Accept: "application/json",
          ...(payload
            ? {
                "Content-Type": "application/json",
                "Content-Length": payload.length,
              }
            : {}),
        },
      },
      (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => {
          const buffer = Buffer.concat(chunks);
          if (response.statusCode < 200 || response.statusCode >= 300) {
            reject(
              new Error(
                `${method} ${requestPath} returned ${response.statusCode}: ${buffer.toString("utf8")}`,
              ),
            );
            return;
          }
          const elapsedMs = Date.now() - startedAt;
          if (options.maxElapsedMs && elapsedMs > options.maxElapsedMs) {
            reject(
              new Error(
                `${method} ${requestPath} took ${elapsedMs}ms, above ${options.maxElapsedMs}ms budget`,
              ),
            );
            return;
          }
          logHttpResult(method, requestPath, elapsedMs, buffer);
          resolve(buffer);
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

function openSimulatorApp(udid) {
  logStep(`opening Simulator.app for ${udid}`);
  spawnSync("open", ["-a", "Simulator", "--args", "-CurrentDeviceUDID", udid], {
    cwd: root,
    stdio: verbose ? "inherit" : "ignore",
  });
}

function assertSimulatorListed(udid) {
  const payload = simdeckJson(["list"]);
  if (!payload.simulators?.some((simulator) => simulator.udid === udid)) {
    throw new Error(`simdeck list did not include ${udid}`);
  }
}

function assertRoots(payload, label) {
  assertJson(payload, label);
  if (!Array.isArray(payload.roots) || payload.roots.length === 0) {
    throw new Error(`${label} returned no roots: ${JSON.stringify(payload)}`);
  }
}

function assertJson(payload, label) {
  if (!payload || typeof payload !== "object") {
    throw new Error(`${label} did not return a JSON object`);
  }
}

function assertPng(filePath) {
  assertPngBuffer(fs.readFileSync(filePath));
}

function assertPngBuffer(buffer) {
  const pngSignature = "89504e470d0a1a0a";
  if (buffer.subarray(0, 8).toString("hex") !== pngSignature) {
    throw new Error("Expected PNG data.");
  }
}

function pointFromSnapshot(snapshot) {
  const root = snapshot.roots?.[0];
  const node = findPreferredPointNode(root) ?? findLeafPointNode(root) ?? root;
  const frame = node?.frame ?? node?.frameInScreen;
  if (!frame || typeof frame !== "object") {
    throw new Error(
      `Unable to derive point from snapshot: ${JSON.stringify(snapshot)}`,
    );
  }
  const x = Number(frame.x) + Number(frame.width) / 2;
  const y = Number(frame.y) + Number(frame.height) / 2;
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    throw new Error(`Snapshot root frame is invalid: ${JSON.stringify(frame)}`);
  }
  return {
    x: Math.round(x),
    y: Math.round(y),
  };
}

function findPreferredPointNode(node) {
  if (!node || typeof node !== "object") {
    return null;
  }
  const text = [
    node.label,
    node.title,
    node.value,
    node.text,
    node.name,
    node.identifier,
    node.accessibilityIdentifier,
  ]
    .filter((value) => typeof value === "string")
    .join(" ");
  if (/SimDeck Fixture|Continue|fixture\./.test(text) && hasUsableFrame(node)) {
    return node;
  }
  const children = Array.isArray(node.children) ? node.children : [];
  for (const child of children) {
    const match = findPreferredPointNode(child);
    if (match) {
      return match;
    }
  }
  return null;
}

function findLeafPointNode(node) {
  if (!node || typeof node !== "object") {
    return null;
  }
  const children = Array.isArray(node.children) ? node.children : [];
  for (const child of children) {
    const match = findLeafPointNode(child);
    if (match) {
      return match;
    }
  }
  if (children.length === 0 && hasUsableFrame(node)) {
    return node;
  }
  return null;
}

function hasUsableFrame(node) {
  const frame = node.frame ?? node.frameInScreen;
  return (
    frame &&
    Number(frame.width) > 4 &&
    Number(frame.height) > 4 &&
    Number.isFinite(Number(frame.x)) &&
    Number.isFinite(Number(frame.y))
  );
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function logStep(message) {
  if (verbose) {
    console.log(`[integration] ${message}`);
  }
}

function logCommand(command, args) {
  if (verbose) {
    console.log(`[cmd] ${shellQuote([command, ...args])}`);
  }
}

function logCommandResult(command, args, elapsedMs, stdout) {
  if (!verbose) {
    return;
  }
  const output = typeof stdout === "string" ? stdout.trim() : String(stdout);
  const preview =
    output.length > 0 && output.length <= 1_000 ? `\n${output}` : "";
  console.log(
    `[ok ${elapsedMs}ms] ${path.basename(command)} ${args[0] ?? ""}${preview}`,
  );
}

function logHttp(method, requestPath, body) {
  if (traceHttp) {
    const suffix = body === undefined ? "" : ` ${JSON.stringify(body)}`;
    console.log(`[http] ${method} ${requestPath}${suffix}`);
  }
}

function logHttpResult(method, requestPath, elapsedMs, buffer) {
  if (!traceHttp) {
    return;
  }
  const text = buffer.toString("utf8");
  const preview = text.length > 0 && text.length <= 1_000 ? `\n${text}` : "";
  console.log(`[ok ${elapsedMs}ms] ${method} ${requestPath}${preview}`);
}

function shellQuote(parts) {
  return parts
    .map((part) => {
      const value = String(part);
      return /^[A-Za-z0-9_./:=@+-]+$/.test(value)
        ? value
        : `'${value.replaceAll("'", "'\\''")}'`;
    })
    .join(" ");
}

function cleanup() {
  if (serverProcess && !serverProcess.killed) {
    serverProcess.kill("SIGTERM");
    serverProcess = null;
  }
  killPortListeners(serverPort);
  if (simulatorUDID && !keepSimulator) {
    spawnSync("xcrun", ["simctl", "shutdown", simulatorUDID], {
      stdio: "ignore",
    });
    spawnSync("xcrun", ["simctl", "delete", simulatorUDID], {
      stdio: "ignore",
    });
    simulatorUDID = "";
  } else if (simulatorUDID && keepSimulator) {
    console.log(`Keeping integration simulator ${simulatorUDID}`);
    simulatorUDID = "";
  }
  if (fs.existsSync(tempRoot)) {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

function killPortListeners(port) {
  const result = spawnSync(
    "lsof",
    ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN", "-t"],
    {
      encoding: "utf8",
    },
  );
  if (result.status !== 0 || !result.stdout.trim()) {
    return;
  }
  for (const pid of result.stdout.trim().split(/\s+/)) {
    if (pid && pid !== String(process.pid)) {
      spawnSync("kill", ["-TERM", pid], { stdio: "ignore" });
    }
  }
}
