#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { connect } from "simdeck/test";

const root = path.resolve(new URL("../..", import.meta.url).pathname);
const simdeck = path.join(root, "build", "simdeck");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "simdeck-js-api-it-"));
const fixtureBundleId = "dev.nativescript.simdeck.integration.fixture";
const fixtureUrlScheme = "simdeck-fixture";
const fixtureUrl = "simdeck-fixture://integration";
const fixtureFocusUrl = "simdeck-fixture://focus-message";
const showSimulator = process.env.SIMDECK_INTEGRATION_SHOW_SIMULATOR === "1";
const keepSimulator = process.env.SIMDECK_INTEGRATION_KEEP_SIMULATOR === "1";
const phaseSetup = "setup";
const phaseTest = "test";
const phaseCommandSmoke = "command-smoke";
const phaseSimulatorLifecycle = "simulator-lifecycle";

let simulatorUDID = "";
let session = null;
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
    throw new Error("SimDeck JS API integration tests require macOS.");
  }
  if (!fs.existsSync(simdeck)) {
    throw new Error(`Missing ${simdeck}. Run npm run build:cli first.`);
  }
  if (
    !fs.existsSync(
      path.join(root, "packages", "simdeck-test", "dist", "index.js"),
    )
  ) {
    throw new Error(
      "Missing simdeck/test dist. Run npm run build:simdeck-test first.",
    );
  }

  const runtime = latestAvailableIosRuntime();
  const deviceType = preferredIphoneDeviceType(runtime);
  const simulatorName = `SimDeck JS API Integration ${Date.now()}`;
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

  await measuredStep(
    "boot simulator",
    () =>
      retrySync(
        () =>
          runText("xcrun", ["simctl", "boot", simulatorUDID], {
            timeoutMs: 180_000,
          }),
        "boot simulator",
        3,
        3_000,
      ),
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

  session = await measuredStep(
    "simdeck/test isolated connect",
    () =>
      connect({
        cliPath: simdeck,
        projectRoot: root,
        isolated: true,
        videoCodec: "h264-software",
      }),
    { phase: phaseSetup },
  );
  console.log(`daemon ${session.endpoint}`);

  await measuredStep(
    "JS install fixture",
    async () => {
      await session.install(simulatorUDID, fixture.appPath);
      preapproveFixtureUrlScheme();
    },
    { phase: phaseSetup },
  );

  await measuredStep("JS list", async () => {
    const payload = await session.list();
    if (
      !payload?.simulators?.some(
        (simulator) => simulator.udid === simulatorUDID,
      )
    ) {
      throw new Error("JS list did not include temp simulator.");
    }
  });
  await measuredStep("JS chromeProfile", async () => {
    assertJson(await session.chromeProfile(simulatorUDID), "chromeProfile");
  });
  await measuredStep("JS launch fixture", async () => {
    await retryAsync(
      () => session.launch(simulatorUDID, fixtureBundleId),
      "JS launch fixture",
      3,
      5_000,
    );
    await session.waitFor(
      simulatorUDID,
      { id: "fixture.continue" },
      { source: "native-ax", maxDepth: 3, timeoutMs: 5_000, pollMs: 250 },
    );
  });
  await measuredStep("JS tree describe", async () => {
    assertRoots(
      await session.tree(simulatorUDID, { source: "native-ax", maxDepth: 1 }),
      "tree",
    );
  });
  await measuredStep("JS selector tap", async () => {
    await session.tapElement(
      simulatorUDID,
      { id: "fixture.continue" },
      {
        source: "native-ax",
        maxDepth: 3,
        waitTimeoutMs: 5_000,
        durationMs: 20,
      },
    );
    await expectFixtureText("Continue Tapped");
  });
  await measuredStep("JS coordinate touch", async () => {
    await session.touch(simulatorUDID, 0.5, 0.525, "began");
    await session.touch(simulatorUDID, 0.5, 0.525, "ended");
  });
  await measuredStep("JS open URL", async () => {
    await session.openUrl(simulatorUDID, fixtureUrl);
    await expectFixtureText("URL Opened");
  });
  await measuredStep("JS focus URL and type", async () => {
    await retryAsync(
      async () => {
        await session.openUrl(simulatorUDID, fixtureFocusUrl);
        await expectFixtureText("Message Focused");
      },
      "JS focus URL",
      3,
      2_000,
    );
    await session.batch(simulatorUDID, [
      { action: "type", text: "agent-ready", delayMs: 0 },
      {
        action: "assert",
        selector: { id: "fixture.message" },
        source: "native-ax",
        maxDepth: 3,
        timeoutMs: 5_000,
      },
    ]);
    await expectElementContains({ id: "fixture.message" }, "agent-ready");
  });

  await measuredStep(
    "JS command smoke batch",
    async () => {
      const result = await session.batch(simulatorUDID, [
        {
          action: "swipe",
          startX: 0.5,
          startY: 0.75,
          endX: 0.5,
          endY: 0.25,
          durationMs: 100,
          steps: 4,
        },
        {
          action: "gesture",
          preset: "scroll-down",
          durationMs: 100,
          delta: 0.2,
          steps: 4,
        },
        { action: "key", keyCode: 40 },
        { action: "keySequence", keyCodes: [11, 8, 15, 15, 18], delayMs: 5 },
        { action: "button", button: "home" },
        { action: "home" },
      ]);
      if (result?.ok !== true || result?.failureCount !== 0) {
        throw new Error(`JS smoke batch failed: ${JSON.stringify(result)}`);
      }
    },
    { phase: phaseCommandSmoke },
  );
  await measuredStep(
    "JS screenshot",
    async () => {
      assertPngBuffer(await session.screenshot(simulatorUDID));
    },
    { phase: phaseCommandSmoke },
  );
  await measuredStep(
    "JS pasteboard",
    async () => {
      await session.pasteboardSet(simulatorUDID, "simdeck js api");
      const text = await session.pasteboardGet(simulatorUDID);
      if (text !== "simdeck js api") {
        throw new Error(
          `pasteboard round-trip failed: ${JSON.stringify(text)}`,
        );
      }
    },
    { phase: phaseCommandSmoke },
  );

  await measuredStep(
    "JS uninstall fixture",
    () =>
      retryAsync(
        () => session.uninstall(simulatorUDID, fixtureBundleId),
        "JS uninstall fixture",
        3,
        1_000,
      ),
    { phase: phaseSimulatorLifecycle },
  );
  await measuredStep("close JS API session", () => closeSession(), {
    phase: phaseSimulatorLifecycle,
  });
  await measuredStep(
    "shutdown simulator",
    () => shutdownSimulatorIfNeeded(simulatorUDID),
    { phase: phaseSimulatorLifecycle },
  );
  await measuredStep(
    "erase simulator",
    () => eraseSimulatorReliably(simulatorUDID),
    {
      phase: phaseSimulatorLifecycle,
    },
  );

  console.log("SimDeck JS API integration suite passed");
}

async function expectFixtureText(text) {
  return expectElementContains({ id: "fixture.status" }, text);
}

async function expectElementContains(selector, text) {
  const deadline = Date.now() + 5_000;
  let last = "";
  while (Date.now() < deadline) {
    const matches = await session.query(simulatorUDID, selector, {
      source: "native-ax",
      maxDepth: 3,
    });
    last = JSON.stringify(matches);
    if (last.includes(text)) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `Timed out waiting for fixture text ${JSON.stringify(text)}: ${last}`,
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
  for (const name of [
    "iPhone 17",
    "iPhone 16",
    "iPhone 15",
    "iPhone 14",
    "iPhone 13",
  ]) {
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
    if (delta !== 0) return delta;
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
      <array><string>${fixtureUrlScheme}</string></array>
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
  if (setResult.status !== 0) {
    runText(
      "/usr/libexec/PlistBuddy",
      ["-c", `Add :${key} string ${fixtureBundleId}`, plist],
      {
        timeoutMs: 60_000,
      },
    );
  }
}

function shutdownSimulatorIfNeeded(udid) {
  try {
    runText("xcrun", ["simctl", "shutdown", udid], {
      timeoutMs: 180_000,
    });
    return { ok: true, udid, action: "shutdown" };
  } catch (error) {
    if (String(error?.message ?? error).includes("current state: Shutdown")) {
      return { ok: true, udid, alreadyShutdown: true };
    }
    throw error;
  }
}

function eraseSimulatorReliably(udid) {
  return retrySync(
    () => {
      shutdownSimulatorIfNeeded(udid);
      runText("xcrun", ["simctl", "erase", udid], {
        timeoutMs: 180_000,
      });
      return { ok: true, udid, action: "erase" };
    },
    "erase simulator",
    3,
    3_000,
  );
}

function closeSession() {
  session?.close();
  session = null;
  return { ok: true };
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

function assertPngBuffer(buffer) {
  if (buffer.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a") {
    throw new Error("Expected PNG data.");
  }
}

async function retryAsync(fn, label, attempts, delayMs) {
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await fn();
    } catch (error) {
      lastError = error;
      if (attempt < attempts) {
        await sleep(delayMs);
      }
    }
  }
  throw new Error(
    `${label} failed after ${attempts} attempts: ${lastError?.message ?? lastError}`,
  );
}

function retrySync(fn, label, attempts, delayMs) {
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return fn();
    } catch (error) {
      lastError = error;
      if (attempt < attempts) {
        sleepSync(delayMs);
      }
    }
  }
  throw new Error(
    `${label} failed after ${attempts} attempts: ${lastError?.message ?? lastError}`,
  );
}

async function measuredStep(label, fn, options = {}) {
  const parentTiming = activeTiming;
  const timing = {
    label,
    phase: options.phase ?? phaseTest,
    startedAt: Date.now(),
    elapsedMs: 0,
    sleepMs: 0,
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
  }
}

function sleep(ms) {
  if (activeTiming) activeTiming.sleepMs += ms;
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function sleepSync(ms) {
  if (activeTiming) activeTiming.sleepMs += ms;
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function printTimingSummary() {
  if (stepTimings.length === 0) return;
  const rows = stepTimings.map((timing) => ({
    label: timing.label,
    phase: timing.phase,
    activeMs: Math.max(0, timing.elapsedMs - timing.sleepMs),
    elapsedMs: timing.elapsedMs,
    sleepMs: timing.sleepMs,
    ok: timing.ok,
  }));
  const phaseTotals = new Map();
  for (const row of rows) {
    const totals = phaseTotals.get(row.phase) ?? {
      activeMs: 0,
      elapsedMs: 0,
      sleepMs: 0,
    };
    totals.activeMs += row.activeMs;
    totals.elapsedMs += row.elapsedMs;
    totals.sleepMs += row.sleepMs;
    phaseTotals.set(row.phase, totals);
  }
  console.log(
    "\nJS API integration timing summary (artificial delays excluded from active):",
  );
  console.log("active\twall\tdelay\tphase\tstatus\tstep");
  for (const row of rows.toSorted(
    (left, right) => right.activeMs - left.activeMs,
  )) {
    console.log(
      `${formatDuration(row.activeMs)}\t${formatDuration(row.elapsedMs)}\t${formatDuration(row.sleepMs)}\t${row.phase}\t${row.ok ? "ok" : "fail"}\t${row.label}`,
    );
  }
  console.log("\nPhase totals:");
  for (const [phase, totals] of [...phaseTotals.entries()].sort()) {
    console.log(
      `${phase}: active ${formatDuration(totals.activeMs)} / wall ${formatDuration(totals.elapsedMs)} / artificial delay ${formatDuration(totals.sleepMs)}`,
    );
  }
  const testTotals = phaseTotals.get(phaseTest) ?? {
    activeMs: 0,
    elapsedMs: 0,
    sleepMs: 0,
  };
  console.log(
    `test body active ${formatDuration(testTotals.activeMs)} / wall ${formatDuration(testTotals.elapsedMs)} / artificial delay ${formatDuration(testTotals.sleepMs)}`,
  );
}

function formatDuration(ms) {
  if (ms >= 60_000) return `${(ms / 60_000).toFixed(2)}m`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)}s`;
  return `${Math.round(ms)}ms`;
}

function runJson(command, args, options = {}) {
  return JSON.parse(runText(command, args, options));
}

function runText(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    input: options.input,
    maxBuffer: options.maxBuffer ?? 32 * 1024 * 1024,
    timeout: options.timeoutMs ?? 120_000,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with ${result.status ?? result.signal}\n${result.stderr ?? ""}\n${result.error?.message ?? ""}`,
    );
  }
  return result.stdout;
}

function openSimulatorApp(udid) {
  spawnSync("open", ["-a", "Simulator", "--args", "-CurrentDeviceUDID", udid], {
    cwd: root,
    stdio: "ignore",
  });
}

function cleanup() {
  try {
    closeSession();
  } catch {}
  if (simulatorUDID && !keepSimulator) {
    spawnSync("xcrun", ["simctl", "shutdown", simulatorUDID], {
      stdio: "ignore",
    });
    spawnSync("xcrun", ["simctl", "delete", simulatorUDID], {
      stdio: "ignore",
    });
  }
  fs.rmSync(tempRoot, { recursive: true, force: true });
}
