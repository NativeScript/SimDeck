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
const fixtureUrlScheme = "simdeck-fixture";
const fixtureUrl = "simdeck-fixture://integration";
const fixtureFocusUrl = "simdeck-fixture://focus-message";
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
const phaseSetup = "setup";
const phaseCommandSmoke = "command-smoke";
const phaseTest = "test";
const phaseSimulatorLifecycle = "simulator-lifecycle";

let simulatorUDID = "";
let serverProcess = null;
const stepTimings = [];
let activeTiming = null;

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
    printTimingSummary();
    cleanup();
    process.exit(0);
  })
  .catch((error) => {
    printTimingSummary();
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
  simulatorUDID = await measuredStep(
    "simctl create simulator",
    () =>
      runText("xcrun", [
        "simctl",
        "create",
        simulatorName,
        deviceType.identifier,
        runtime.identifier,
      ]).trim(),
    { phase: phaseSetup },
  );

  console.log(
    `created ${simulatorUDID} (${deviceType.name}, ${runtime.version})`,
  );
  startServer();
  await measuredStep("server health", () => waitForHealth(), {
    phase: phaseSetup,
  });
  logStep(`server ready at ${serverUrl}`);

  await measuredStep(
    "CLI boot simulator",
    () =>
      retrySimdeckJson(["boot", simulatorUDID], "CLI boot simulator", {
        attempts: 3,
        delayMs: 3_000,
        timeoutMs: 180_000,
      }),
    { phase: phaseSetup },
  );
  await measuredStep(
    "simctl bootstatus initial",
    () =>
      runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
        timeoutMs: 600_000,
      }),
    { phase: phaseSetup },
  );
  if (showSimulator) {
    openSimulatorApp(simulatorUDID);
  }

  const fixture = await measuredStep(
    "build SwiftUI fixture",
    () => buildFixtureApp(),
    { phase: phaseSetup },
  );

  await measuredStep("CLI list", () => assertSimulatorListed(simulatorUDID), {
    phase: phaseSetup,
  });
  await measuredStep(
    "CLI chrome-profile",
    () =>
      assertJson(
        simdeckJson(["chrome-profile", simulatorUDID]),
        "chrome-profile",
      ),
    { phase: phaseSetup },
  );
  await measuredStep(
    "CLI logs",
    () =>
      assertJson(
        simdeckJson(["logs", simulatorUDID, "--seconds", "1", "--limit", "1"]),
        "logs",
      ),
    { phase: phaseSetup },
  );

  await measuredStep(
    "CLI install fixture",
    async () => {
      simdeckJson(["install", simulatorUDID, fixture.appPath]);
      preapproveFixtureUrlScheme();
    },
    { phase: phaseSetup },
  );

  await measuredStep(
    "setup launch SwiftUI fixture",
    async () => {
      try {
        await retrySimdeckJson(
          cliArgs(["launch", simulatorUDID, fixtureBundleId]),
          "setup launch SwiftUI fixture",
          {
            attempts: 3,
            delayMs: 5_000,
            timeoutMs: 180_000,
          },
        );
        await verifyUi("setup launch SwiftUI fixture", {
          expectFixture: true,
          phase: phaseSetup,
          waitTimeoutMs: 3_000,
        });
      } catch (error) {
        logStep(
          `setup warm launch skipped: ${String(error?.message ?? error).split("\n")[0]}`,
        );
      }
    },
    { phase: phaseSetup },
  );

  const agentTree = await measuredStep("server describe agent", () =>
    simdeckText([
      "describe",
      simulatorUDID,
      "--source",
      "native-ax",
      "--format",
      "agent",
      "--max-depth",
      "1",
    ]),
  );
  if (!agentTree.includes("source:") || !agentTree.includes("- ")) {
    throw new Error("agent describe output did not look like a hierarchy");
  }
  await runRestControls();
  await runCliControls();

  const screenshotPath = path.join(tempRoot, "screen.png");
  await measuredStep(
    "CLI screenshot file",
    async () => {
      simdeckJson(["screenshot", simulatorUDID, "--output", screenshotPath]);
      assertPng(screenshotPath);
    },
    { phase: phaseCommandSmoke },
  );
  const stdoutPng = path.join(tempRoot, "screen-stdout.png");
  await measuredStep(
    "CLI screenshot stdout",
    async () => {
      fs.writeFileSync(
        stdoutPng,
        runBuffer(simdeck, ["screenshot", simulatorUDID, "--stdout"], {
          timeoutMs: 300_000,
          maxBuffer: 64 * 1024 * 1024,
        }),
      );
      assertPng(stdoutPng);
    },
    { phase: phaseCommandSmoke },
  );

  await measuredStep(
    "CLI pasteboard set",
    async () => {
      simdeckJson(["pasteboard", "set", simulatorUDID, "simdeck integration"]);
    },
    { phase: phaseCommandSmoke },
  );
  await measuredStep(
    "CLI pasteboard get",
    async () => {
      const pasteboard = simdeckJson(["pasteboard", "get", simulatorUDID]);
      if (pasteboard.text !== "simdeck integration") {
        throw new Error(
          `pasteboard round-trip failed: ${JSON.stringify(pasteboard)}`,
        );
      }
    },
    { phase: phaseCommandSmoke },
  );

  const fileInput = path.join(tempRoot, "type.txt");
  fs.writeFileSync(fileInput, "file input");
  await measuredStep(
    "CLI type file",
    async () => {
      simdeckJson(["type", simulatorUDID, "--file", fileInput]);
    },
    { phase: phaseCommandSmoke },
  );
  await measuredStep(
    "CLI type stdin",
    async () => {
      simdeckJson(["type", simulatorUDID, "--stdin"], {
        input: "stdin input",
      });
    },
    { phase: phaseCommandSmoke },
  );

  await measuredStep(
    "CLI batch",
    async () => {
      const batch = simdeckJson([
        "batch",
        simulatorUDID,
        "--step",
        "button home",
        "--step",
        "tap --x 0.5 --y 0.7 --normalized --duration-ms 20",
        "--step",
        "type batch",
        "--continue-on-error",
      ]);
      if (batch.ok !== true || batch.failureCount !== 0) {
        throw new Error(`batch command failed: ${JSON.stringify(batch)}`);
      }
    },
    { phase: phaseCommandSmoke },
  );

  await runHardwareButtonControls();

  await measuredStep(
    "CLI uninstall fixture",
    () => simdeckJson(["uninstall", simulatorUDID, fixtureBundleId]),
    { phase: phaseSimulatorLifecycle },
  );
  await measuredStep(
    "CLI shutdown",
    () => shutdownSimulatorIfNeeded(simulatorUDID),
    { phase: phaseSimulatorLifecycle },
  );
  await measuredStep("CLI erase", () => simdeckJson(["erase", simulatorUDID]), {
    phase: phaseSimulatorLifecycle,
  });
  await measuredStep(
    "CLI boot after erase",
    () =>
      retrySimdeckJson(["boot", simulatorUDID], "CLI boot after erase", {
        attempts: 3,
        delayMs: 3_000,
        timeoutMs: 180_000,
      }),
    { phase: phaseSimulatorLifecycle },
  );
  await measuredStep(
    "simctl bootstatus after erase",
    () =>
      runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
        timeoutMs: 600_000,
      }),
    { phase: phaseSimulatorLifecycle },
  );
  await measuredStep(
    "post-erase describe",
    async () => {
      assertRoots(
        await retrySimdeckJson(
          [
            "describe",
            simulatorUDID,
            "--direct",
            "--format",
            "compact-json",
            "--max-depth",
            "1",
          ],
          "post-erase describe",
        ),
        "post-erase describe",
      );
    },
    { phase: phaseSimulatorLifecycle },
  );

  console.log("SimDeck CLI integration suite passed");
}

async function runCliControls() {
  await cliStep("CLI home", ["home", simulatorUDID], {}, { skip: true });
  await cliStep(
    "CLI tap",
    [
      "tap",
      simulatorUDID,
      "0.5",
      "0.525",
      "--normalized",
      "--duration-ms",
      "20",
    ],
    {},
    { skip: true },
  );
  await cliStep(
    "CLI touch began",
    [
      "touch",
      simulatorUDID,
      "0.5",
      "0.525",
      "--phase",
      "began",
      "--normalized",
    ],
    {},
    { skip: true },
  );
  await cliStep(
    "CLI touch ended",
    [
      "touch",
      simulatorUDID,
      "0.5",
      "0.525",
      "--phase",
      "ended",
      "--normalized",
    ],
    {},
    { skip: true },
  );
  await cliStep(
    "CLI touch down/up",
    [
      "touch",
      simulatorUDID,
      "0.5",
      "0.525",
      "--down",
      "--up",
      "--normalized",
      "--delay-ms",
      "20",
    ],
    {},
    { skip: true },
  );
  await cliStep(
    "CLI pinch",
    [
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
    ],
    {},
    { skip: true, phase: phaseCommandSmoke },
  );
  await cliStep(
    "CLI rotate gesture",
    [
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
    ],
    {},
    { skip: true, phase: phaseCommandSmoke },
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
    ["open-url", simulatorUDID, fixtureFocusUrl],
    { attempts: 3, delayMs: 5_000, timeoutMs: 180_000 },
    { expectFixture: true, expectText: "Message Focused" },
  );
  await cliStep(
    "CLI tap fixture text field",
    [
      "tap",
      simulatorUDID,
      "--id",
      "fixture.message",
      "--wait-timeout-ms",
      "15000",
      "--duration-ms",
      "30",
    ],
    { timeoutMs: 180_000, maxElapsedMs: 60_000 },
    {
      expectFixture: true,
      expectText: "Message Focused",
      attempts: 6,
      delayMs: 1_500,
    },
  );
  await cliStep(
    "CLI type fixture text",
    ["type", simulatorUDID, "agent-ready"],
    { attempts: 1, timeoutMs: 180_000, maxElapsedMs: 60_000 },
    { expectFixture: true, expectText: "agent-ready", attempts: 12 },
  );
  await measuredStep(
    "CLI smoke control batch",
    async () => {
      const batch = simdeckJson(
        cliArgs([
          "batch",
          simulatorUDID,
          "--step",
          "swipe 0.5 0.75 0.5 0.25 --duration-ms 100 --steps 4",
          "--step",
          "gesture scroll-down --duration-ms 100 --delta 0.2 --steps 4",
          "--step",
          "gesture swipe-from-left-edge --duration-ms 100 --steps 4",
          "--step",
          "key enter",
          "--step",
          "key-sequence --keycodes h,e,l,l,o --delay-ms 5",
          "--step",
          "key-combo --modifiers cmd --key a",
          "--step",
          "type qa",
          "--step",
          "dismiss-keyboard",
          "--step",
          "app-switcher",
          "--step",
          "home",
          "--step",
          "rotate-left",
          "--step",
          "rotate-right",
          "--step",
          "toggle-appearance",
        ]),
      );
      if (batch.ok !== true || batch.failureCount !== 0) {
        throw new Error(`smoke control batch failed: ${JSON.stringify(batch)}`);
      }
    },
    { phase: phaseCommandSmoke },
  );
}

async function runHardwareButtonControls() {
  await measuredStep(
    "CLI hardware button batch",
    async () => {
      const batch = simdeckJson(
        cliArgs([
          "batch",
          simulatorUDID,
          "--step",
          "button home",
          "--step",
          "button lock --duration-ms 50",
          "--step",
          "button lock --duration-ms 50",
          "--step",
          "button side-button --duration-ms 50",
          "--step",
          "button side-button --duration-ms 50",
          "--step",
          "button siri --duration-ms 50",
          "--step",
          "home",
          "--step",
          "button apple-pay --duration-ms 50",
          "--step",
          "home",
        ]),
      );
      if (batch.ok !== true || batch.failureCount !== 0) {
        throw new Error(
          `hardware button batch failed: ${JSON.stringify(batch)}`,
        );
      }
    },
    { phase: phaseCommandSmoke },
  );
}

async function runRestControls() {
  await measuredStep(
    "REST simulator list",
    async () => {
      const simulators = await httpJson("GET", "/api/simulators");
      if (
        !simulators.simulators?.some(
          (simulator) => simulator.udid === simulatorUDID,
        )
      ) {
        throw new Error("REST simulator list did not include temp simulator");
      }
    },
    { phase: phaseSetup },
  );
  await measuredStep(
    "REST accessibility-tree",
    async () => {
      assertRoots(
        await httpJson(
          "GET",
          `/api/simulators/${simulatorUDID}/accessibility-tree?source=native-ax&maxDepth=1`,
        ),
        "REST accessibility-tree",
      );
    },
    { phase: phaseSetup },
  );
  await measuredStep(
    "REST chrome-profile",
    async () => {
      assertJson(
        await httpJson(
          "GET",
          `/api/simulators/${simulatorUDID}/chrome-profile`,
        ),
        "REST chrome-profile",
      );
    },
    { phase: phaseSetup },
  );
  await measuredStep(
    "REST chrome.png",
    async () => {
      assertPngBuffer(
        await httpBuffer("GET", `/api/simulators/${simulatorUDID}/chrome.png`),
      );
    },
    { phase: phaseSetup },
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
    .onOpenURL { url in
      if url.host == "focus-message" {
        status = "Message Focused"
        messageFocused = true
      } else {
        status = "URL Opened"
      }
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
  runText(
    "xcrun",
    [
      "--sdk",
      "iphonesimulator",
      "swiftc",
      "-target",
      `${targetArch}-apple-ios15.0-simulator`,
      "-parse-as-library",
      "-Onone",
      "-framework",
      "SwiftUI",
      "-framework",
      "UIKit",
      main,
      "-o",
      path.join(appPath, executable),
    ],
    { timeoutMs: 300_000 },
  );
  return { appPath };
}

function startServer() {
  killPortListeners(serverPort);
  logStep(`starting server on ${serverUrl}`);
  serverProcess = spawn(
    simdeck,
    [
      "daemon",
      "run",
      "--project-root",
      root,
      "--metadata-path",
      path.join(tempRoot, "daemon.json"),
      "--port",
      String(serverPort),
      "--client-root",
      path.join(root, "client", "dist"),
      "--access-token",
      "integration",
      "--video-codec",
      "h264-software",
    ],
    {
      cwd: root,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  serverProcess.stdout.on("data", (data) =>
    process.stdout.write(`[daemon] ${data}`),
  );
  serverProcess.stderr.on("data", (data) =>
    process.stderr.write(`[daemon] ${data}`),
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
  return measuredStep(
    label,
    async () => {
      const result = await retrySimdeckJson(cliArgs(args), label, {
        maxElapsedMs: cliCommandBudgetMs,
        ...commandOptions,
      });
      if (!verifyOptions.skip) {
        await verifyUi(label, verifyOptions);
      }
      return result;
    },
    { phase: verifyOptions.phase ?? phaseTest },
  );
}

function cliArgs(args) {
  return args;
}

async function httpStep(
  label,
  method,
  requestPath,
  body,
  requestOptions = {},
  verifyOptions = {},
) {
  return measuredStep(
    label,
    async () => {
      logStep(`${label}`);
      const result = await retryHttpJson(
        method,
        requestPath,
        body,
        label,
        requestOptions,
      );
      if (!verifyOptions.skip) {
        await verifyUi(label, verifyOptions);
      }
      return result;
    },
    { phase: verifyOptions.phase ?? phaseTest },
  );
}

async function verifyUi(label, options = {}) {
  if (!serverProcess) {
    return verifyUiWithDescribe(label, options);
  }

  const attempts = options.attempts ?? (options.expectFixture ? 8 : 3);
  const delayMs = options.delayMs ?? (options.expectFixture ? 1_000 : 500);
  let lastError = "";
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      await resolveKnownSystemPromptsWithQueries(label);
      if (options.expectFixture) {
        await waitForUiElement(label, { id: "fixture.continue" }, options);
        await queryUi(label, {
          selector: { id: "fixture.message" },
          limit: 1,
          maxDepth: options.maxDepth ?? 3,
        });
      }
      if (options.expectText) {
        await waitForUiText(label, options.expectText, options);
      }
      if (!options.expectFixture && !options.expectText) {
        await queryUi(label, { limit: 1, maxDepth: 2 });
      }
      logStep(`ui after ${label}: ${summarizeUiCheck(options)}`);
      return "";
    } catch (error) {
      lastError = error?.message ?? String(error);
      if (attempt < attempts) {
        await sleep(delayMs);
      }
    }
  }
  throw new Error(
    `${label} did not reach expected UI after ${attempts} UI checks:\n${lastError}`,
  );
}

async function verifyUiWithDescribe(label, options = {}) {
  const attempts = options.attempts ?? (options.expectFixture ? 8 : 3);
  const delayMs = options.delayMs ?? (options.expectFixture ? 3_000 : 1_000);
  let lastSnapshot = "";
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      let snapshot = await retrySimdeckText(
        describeUiVerificationArgs(),
        `${label} describe`,
        {
          attempts: 1,
          timeoutMs: 90_000,
          maxElapsedMs: options.describeMaxElapsedMs ?? describeUiBudgetMs,
        },
      );
      snapshot = await resolveKnownSystemPrompts(snapshot, label);
      lastSnapshot = snapshot;
      logStep(`ui after ${label}: ${summarizeUi(snapshot)}`);
      const fixtureOk = !options.expectFixture || fixtureReady(snapshot);
      const textOk =
        !options.expectText || snapshot.includes(options.expectText);
      if (fixtureOk && textOk) {
        return snapshot;
      }
    } catch (error) {
      lastSnapshot = error?.message ?? String(error);
      logStep(`ui after ${label}: describe failed, retrying`);
    }
    if (attempt < attempts) {
      await sleep(delayMs);
    }
  }
  throw new Error(
    `${label} did not reach expected UI after ${attempts} UI checks:\n${lastSnapshot}`,
  );
}

async function queryUi(label, body) {
  return retryHttpJson(
    "POST",
    `/api/simulators/${simulatorUDID}/query`,
    {
      source: "native-ax",
      maxDepth: body.maxDepth ?? 8,
      ...body,
    },
    `${label} query`,
    {
      attempts: 1,
      maxElapsedMs: body.maxElapsedMs ?? httpActionBudgetMs,
    },
  );
}

async function waitForUiElement(label, selector, options = {}) {
  return retryHttpJson(
    "POST",
    `/api/simulators/${simulatorUDID}/wait-for`,
    {
      source: "native-ax",
      maxDepth: options.maxDepth ?? 3,
      timeoutMs: options.waitTimeoutMs ?? 5_000,
      pollMs: options.pollMs ?? 250,
      selector,
    },
    `${label} wait-for ${JSON.stringify(selector)}`,
    {
      attempts: 1,
      maxElapsedMs: options.waitMaxElapsedMs ?? httpActionBudgetMs,
    },
  );
}

async function waitForUiText(label, text, options = {}) {
  const timeoutMs = options.waitTimeoutMs ?? 5_000;
  const pollMs = options.pollMs ?? 250;
  const startedAt = Date.now();
  let lastError = "";
  while (Date.now() - startedAt <= timeoutMs) {
    if (options.expectFixture) {
      try {
        const status = await queryUi(label, {
          selector: { id: "fixture.status" },
          limit: 1,
          maxDepth: options.maxDepth ?? 3,
          maxElapsedMs: options.queryMaxElapsedMs ?? httpActionBudgetMs,
        });
        if (JSON.stringify(status.matches ?? []).includes(text)) {
          return status.matches[0];
        }
      } catch (error) {
        lastError = error?.message ?? String(error);
      }
    }
    for (const selector of [{ label: text }, { value: text }, { id: text }]) {
      try {
        const result = await queryUi(label, {
          selector,
          limit: 1,
          maxDepth: options.maxDepth ?? 3,
          maxElapsedMs: options.queryMaxElapsedMs ?? httpActionBudgetMs,
        });
        if (result.matches?.length > 0) {
          return result.matches[0];
        }
      } catch (error) {
        lastError = error?.message ?? String(error);
      }
    }
    await sleep(pollMs);
  }
  throw new Error(`Timed out waiting for UI text "${text}": ${lastError}`);
}

async function resolveKnownSystemPromptsWithQueries(label) {
  const prompts = [{ kind: "open-url", selector: { label: "Open" } }];
  for (const prompt of prompts) {
    const result = await queryUi(label, {
      selector: prompt.selector,
      limit: 1,
      maxDepth: 6,
      maxElapsedMs: httpActionBudgetMs,
    });
    if (!result.matches?.length) {
      continue;
    }
    logStep(`handling system ${prompt.kind} prompt after ${label}`);
    await retryHttpJson(
      "POST",
      `/api/simulators/${simulatorUDID}/tap`,
      {
        source: "native-ax",
        maxDepth: 6,
        waitTimeoutMs: 2_000,
        durationMs: 80,
        selector: prompt.selector,
      },
      `${label} tap ${prompt.kind} prompt`,
      { attempts: 1, maxElapsedMs: httpActionBudgetMs },
    );
    await sleep(500);
  }
}

function summarizeUiCheck(options = {}) {
  const parts = ["fast query"];
  if (options.expectFixture) {
    parts.push("fixture ids present");
  }
  if (options.expectText) {
    parts.push(`text "${options.expectText}" present`);
  }
  return parts.join(", ");
}

async function resolveKnownSystemPrompts(snapshot, label) {
  if (
    !looksLikeOpenUrlPrompt(snapshot) &&
    !looksLikeKeyboardTipPrompt(snapshot)
  ) {
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
      describeUiVerificationArgs(),
      `${label} describe after ${action.label}`,
      {
        attempts: 3,
        delayMs: 1_000,
        timeoutMs: 90_000,
        maxElapsedMs: describeUiBudgetMs,
      },
    );
    if (
      !looksLikeOpenUrlPrompt(current) &&
      !looksLikeKeyboardTipPrompt(current)
    ) {
      logStep(`system ${promptKind} prompt cleared by ${action.label}`);
      return current;
    }
  }
  return current;
}

function describeUiVerificationArgs() {
  const args = [
    "describe",
    simulatorUDID,
    "--source",
    "native-ax",
    "--format",
    "agent",
    "--max-depth",
    "2",
  ];
  return args;
}

function openUrlPromptActions(snapshot) {
  const actions = [
    {
      label: "key enter",
      run: () =>
        simdeckJson(["key", simulatorUDID, "enter"], { timeoutMs: 60_000 }),
    },
    {
      label: "tap Open by label",
      run: () =>
        simdeckJson(
          [
            "tap",
            simulatorUDID,
            "--label",
            "Open",
            "--wait-timeout-ms",
            "5000",
          ],
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
  const candidates = [point, { x: point.y, y: point.x }];
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
  const candidates = [point, { x: point.y, y: point.x }];
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
  return candidateNormalizedTransforms(x, y);
}

function candidateNormalizedTransforms(x, y) {
  return uniqueUnitPoints([
    { x, y },
    { x: y, y: x },
    { x: 1 - x, y },
    { x, y: 1 - y },
    { x: 1 - x, y: 1 - y },
    { x: y, y: 1 - x },
    { x: 1 - y, y: x },
    { x: 1 - y, y: 1 - x },
  ]).filter(
    (candidate) =>
      candidate.x >= 0 &&
      candidate.x <= 1 &&
      candidate.y >= 0 &&
      candidate.y <= 1,
  );
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
  return runText(simdeck, ["--server-url", serverUrl, ...args], {
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
  recordCommandTiming(command, args, elapsedMs);
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
  recordCommandTiming(command, args, elapsedMs);
  if (options.maxElapsedMs && elapsedMs > options.maxElapsedMs) {
    throw new Error(
      `${command} ${args.join(" ")} took ${elapsedMs}ms, above ${options.maxElapsedMs}ms budget`,
    );
  }
  logCommandResult(command, args, elapsedMs, `<${result.stdout.length} bytes>`);
  return result.stdout;
}

function recordCommandTiming(command, args, elapsedMs) {
  if (!activeTiming) {
    return;
  }
  activeTiming.commandMs += elapsedMs;
  if (command === simdeck && args.includes("describe")) {
    activeTiming.describeUiMs += elapsedMs;
  }
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

function preapproveFixtureUrlScheme() {
  const plist = path.join(
    os.homedir(),
    "Library",
    "Developer",
    "CoreSimulator",
    "Devices",
    simulatorUDID,
    "data",
    "Library",
    "Preferences",
    "com.apple.launchservices.schemeapproval.plist",
  );
  const key = `com.apple.CoreSimulator.CoreSimulatorBridge-->${fixtureUrlScheme}`;
  fs.mkdirSync(path.dirname(plist), { recursive: true });

  const setResult = spawnSync(
    "/usr/libexec/PlistBuddy",
    ["-c", `Set :${key} ${fixtureBundleId}`, plist],
    {
      cwd: root,
      encoding: "utf8",
      timeout: 60_000,
    },
  );
  if (setResult.status === 0) {
    logStep(`preapproved fixture URL scheme in ${path.basename(plist)}`);
    return;
  }

  runText(
    "/usr/libexec/PlistBuddy",
    ["-c", `Add :${key} string ${fixtureBundleId}`, plist],
    { timeoutMs: 60_000 },
  );
  logStep(`preapproved fixture URL scheme in ${path.basename(plist)}`);
}

function assertSimulatorListed(udid) {
  const payload = simdeckJson(["list"]);
  if (!payload.simulators?.some((simulator) => simulator.udid === udid)) {
    throw new Error(`simdeck list did not include ${udid}`);
  }
}

function shutdownSimulatorIfNeeded(udid) {
  try {
    return simdeckJson(["shutdown", udid]);
  } catch (error) {
    if (String(error?.message ?? error).includes("current state: Shutdown")) {
      return { ok: true, udid, alreadyShutdown: true };
    }
    throw error;
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

async function measuredStep(label, fn, options = {}) {
  const parentTiming = activeTiming;
  const timing = {
    label,
    phase: options.phase ?? phaseTest,
    startedAt: Date.now(),
    elapsedMs: 0,
    sleepMs: 0,
    describeUiMs: 0,
    commandMs: 0,
    ok: false,
  };
  activeTiming = timing;
  try {
    const result = await fn();
    timing.ok = true;
    return result;
  } finally {
    timing.elapsedMs = Date.now() - timing.startedAt;
    stepTimings.push(timing);
    activeTiming = parentTiming;
    logStep(
      `timing ${label}: active ${formatDuration(timing.elapsedMs - timing.sleepMs)} (${formatDuration(timing.elapsedMs)} wall, ${formatDuration(timing.sleepMs)} artificial delay)`,
    );
  }
}

function sleep(ms) {
  if (activeTiming) {
    activeTiming.sleepMs += ms;
  }
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function printTimingSummary() {
  if (stepTimings.length === 0) {
    return;
  }
  const rows = stepTimings.map((timing) => ({
    label: timing.label,
    phase: timing.phase,
    activeMs: Math.max(0, timing.elapsedMs - timing.sleepMs),
    elapsedMs: timing.elapsedMs,
    sleepMs: timing.sleepMs,
    describeUiMs: timing.describeUiMs,
    commandMs: timing.commandMs,
    ok: timing.ok,
  }));
  const totalActiveMs = rows.reduce((sum, row) => sum + row.activeMs, 0);
  const totalElapsedMs = rows.reduce((sum, row) => sum + row.elapsedMs, 0);
  const totalSleepMs = rows.reduce((sum, row) => sum + row.sleepMs, 0);
  const totalDescribeUiMs = rows.reduce(
    (sum, row) => sum + row.describeUiMs,
    0,
  );
  const phaseTotals = new Map();
  for (const row of rows) {
    const totals = phaseTotals.get(row.phase) ?? {
      activeMs: 0,
      elapsedMs: 0,
      sleepMs: 0,
      describeUiMs: 0,
    };
    totals.activeMs += row.activeMs;
    totals.elapsedMs += row.elapsedMs;
    totals.sleepMs += row.sleepMs;
    totals.describeUiMs += row.describeUiMs;
    phaseTotals.set(row.phase, totals);
  }

  console.log(
    "\nIntegration timing summary (artificial delays excluded from active):",
  );
  console.log("active\twall\tdelay\tdescribe\tphase\tstatus\tstep");
  for (const row of rows.toSorted(
    (left, right) => right.activeMs - left.activeMs,
  )) {
    console.log(
      `${formatDuration(row.activeMs)}\t${formatDuration(row.elapsedMs)}\t${formatDuration(row.sleepMs)}\t${formatDuration(row.describeUiMs)}\t${row.phase}\t${row.ok ? "ok" : "fail"}\t${row.label}`,
    );
  }
  console.log("\nPhase totals:");
  for (const [phase, totals] of [...phaseTotals.entries()].sort()) {
    console.log(
      `${phase}: active ${formatDuration(totals.activeMs)} / wall ${formatDuration(totals.elapsedMs)} / artificial delay ${formatDuration(totals.sleepMs)} / describe ${formatDuration(totals.describeUiMs)}`,
    );
  }
  const testTotals = phaseTotals.get(phaseTest) ?? {
    activeMs: 0,
    elapsedMs: 0,
    sleepMs: 0,
    describeUiMs: 0,
  };
  console.log(
    `test body active ${formatDuration(testTotals.activeMs)} / wall ${formatDuration(testTotals.elapsedMs)} / artificial delay ${formatDuration(testTotals.sleepMs)} / describe ${formatDuration(testTotals.describeUiMs)}`,
  );
  console.log(
    `total active ${formatDuration(totalActiveMs)} / wall ${formatDuration(totalElapsedMs)} / artificial delay ${formatDuration(totalSleepMs)} / describe ${formatDuration(totalDescribeUiMs)}`,
  );
}

function formatDuration(ms) {
  if (!Number.isFinite(ms)) {
    return "n/a";
  }
  if (ms >= 60_000) {
    return `${(ms / 60_000).toFixed(2)}m`;
  }
  if (ms >= 1_000) {
    return `${(ms / 1_000).toFixed(2)}s`;
  }
  return `${Math.max(0, Math.round(ms))}ms`;
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
