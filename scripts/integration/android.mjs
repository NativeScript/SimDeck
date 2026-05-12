#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { connect } from "simdeck/test";

const root = path.resolve(new URL("../..", import.meta.url).pathname);
const simdeck = path.join(root, "build", "simdeck");
const verbose = process.env.SIMDECK_INTEGRATION_VERBOSE === "1";
const keepAndroid = process.env.SIMDECK_INTEGRATION_KEEP_ANDROID === "1";
const bootAndroid = process.env.SIMDECK_INTEGRATION_BOOT_ANDROID === "1";
const requireRunningAndroid =
  process.env.SIMDECK_INTEGRATION_REQUIRE_RUNNING_ANDROID === "1" ||
  process.env.CI === "true";
const requestedAvd = process.env.SIMDECK_INTEGRATION_ANDROID_AVD;
const defaultStepTimeoutMs = Number(
  process.env.SIMDECK_INTEGRATION_STEP_TIMEOUT_MS ?? "180000",
);

let session = null;
let androidUDID = "";
let shutdownAndroidAfterRun = false;
const stepTimings = [];

process.on("SIGINT", () => {
  cleanupSync();
  process.exit(130);
});
process.on("SIGTERM", () => {
  cleanupSync();
  process.exit(143);
});

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error?.stack ?? error);
    process.exit(1);
  });

async function main() {
  try {
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

    session = await measuredStep("simdeck/test isolated connect", () =>
      connect({
        cliPath: simdeck,
        projectRoot: root,
        isolated: true,
        videoCodec: "software",
      }),
    );
    console.log(`daemon ${session.endpoint}`);

    const target = await measuredStep("resolve Android AVD", () =>
      resolveAndroidDevice(),
    );
    if (!target) {
      console.log(
        "No Android AVDs discovered; skipping Android integration suite.",
      );
      return;
    }

    androidUDID = target.udid;
    console.log(
      `android target ${androidUDID} (${target.name})${
        target.isBooted ? " already booted" : ""
      }`,
    );
    if (!target.isBooted && !bootAndroid) {
      const message = `${androidUDID} is not running. Start the emulator first or set SIMDECK_INTEGRATION_BOOT_ANDROID=1 to let SimDeck boot it.`;
      if (requireRunningAndroid) {
        throw new Error(message);
      }
      console.log(`${message} Skipping Android integration suite.`);
      return;
    }
    shutdownAndroidAfterRun = !target.isBooted && bootAndroid && !keepAndroid;

    await measuredStep(
      target.isBooted
        ? "JS boot Android idempotent"
        : "JS boot Android emulator",
      async () => {
        await session.boot(androidUDID);
        await waitForBooted(androidUDID);
      },
      { timeoutMs: target.isBooted ? 120_000 : 300_000 },
    );

    await runCliSurface();
    await runJsSurface();
    console.log("SimDeck Android integration suite passed");
  } finally {
    await cleanupAndroid();
    cleanupSync();
    printTimingSummary();
  }
}

async function runCliSurface() {
  await measuredStep("CLI list includes Android", () => {
    assertAndroidListed(simdeckJson(["list"]), androidUDID);
  });
  await measuredStep(
    "CLI boot Android idempotent",
    () => assertJson(simdeckJson(["boot", androidUDID]), "boot"),
    { timeoutMs: 180_000 },
  );
  await measuredStep("CLI chrome-profile", () => {
    const profile = simdeckJson(["chrome-profile", androidUDID]);
    assertJson(profile, "chrome-profile");
    assert.ok(Number(profile.screenWidth) > 0, "missing Android screenWidth");
    assert.ok(Number(profile.screenHeight) > 0, "missing Android screenHeight");
  });
  const tree = await measuredStep("CLI describe Android tree", () => {
    const payload = simdeckJson([
      "describe",
      androidUDID,
      "--source",
      "android-uiautomator",
      "--format",
      "compact-json",
      "--max-depth",
      "2",
    ]);
    assertRoots(payload, "CLI describe");
    return payload;
  });
  await measuredStep("CLI describe Android point", () => {
    const point = centerOfFirstRoot(tree);
    if (!point) {
      throw new Error("Unable to derive point from Android tree root.");
    }
    assertRoots(
      simdeckJson([
        "describe",
        androidUDID,
        "--source",
        "android-uiautomator",
        "--format",
        "compact-json",
        "--point",
        `${point.x},${point.y}`,
        "--max-depth",
        "1",
      ]),
      "CLI point describe",
    );
  });
  await measuredStep("CLI screenshot stdout", () => {
    assertPngBuffer(
      runBuffer(
        simdeck,
        [
          "--server-url",
          session.endpoint,
          "screenshot",
          androidUDID,
          "--stdout",
        ],
        { timeoutMs: 120_000, maxBuffer: 64 * 1024 * 1024 },
      ),
    );
  });
  await measuredStep("CLI pasteboard behavior", () => {
    const setResult = runCommand(
      simdeck,
      [
        "--server-url",
        session.endpoint,
        "pasteboard",
        "set",
        androidUDID,
        "simdeck android cli",
      ],
      { timeoutMs: 60_000 },
    );
    if (setResult.status !== 0) {
      assertClipboardUnsupported(setResult);
      return;
    }
    try {
      const payload = simdeckJson(["pasteboard", "get", androidUDID]);
      assert.equal(payload.text, "simdeck android cli");
    } catch (error) {
      assertClipboardUnsupported(error);
    }
  });
  await measuredStep("CLI app launch and URL", () => {
    simdeckJson(["launch", androidUDID, "com.android.settings"], {
      timeoutMs: 60_000,
    });
    simdeckJson(["open-url", androidUDID, "https://example.com"], {
      timeoutMs: 60_000,
    });
    simdeckJson(["home", androidUDID]);
  });
  await measuredStep("CLI pointer gestures", () => {
    simdeckJson([
      "tap",
      androidUDID,
      "0.5",
      "0.5",
      "--normalized",
      "--duration-ms",
      "20",
    ]);
    simdeckJson([
      "touch",
      androidUDID,
      "0.5",
      "0.5",
      "--phase",
      "began",
      "--normalized",
    ]);
    simdeckJson([
      "touch",
      androidUDID,
      "0.5",
      "0.5",
      "--phase",
      "ended",
      "--normalized",
    ]);
    simdeckJson([
      "swipe",
      androidUDID,
      "0.5",
      "0.75",
      "0.5",
      "0.25",
      "--normalized",
      "--duration-ms",
      "120",
      "--steps",
      "4",
    ]);
    simdeckJson([
      "gesture",
      androidUDID,
      "scroll-down",
      "--duration-ms",
      "120",
      "--delta",
      "0.2",
    ]);
  });
  await measuredStep("CLI keyboard and system controls", () => {
    simdeckJson(["key", androidUDID, "enter"]);
    simdeckJson([
      "key-sequence",
      androidUDID,
      "--keycodes",
      "h,e,l,l,o",
      "--delay-ms",
      "5",
    ]);
    simdeckJson([
      "key-combo",
      androidUDID,
      "--modifiers",
      "shift",
      "--key",
      "h",
    ]);
    simdeckJson(["type", androidUDID, "simdeck"]);
    simdeckJson(["dismiss-keyboard", androidUDID]);
    simdeckJson(["button", androidUDID, "back"]);
    simdeckJson(["app-switcher", androidUDID]);
    simdeckJson(["home", androidUDID]);
  });
  await measuredStep("CLI rotation and appearance restore", () => {
    simdeckJson(["rotate-left", androidUDID], { timeoutMs: 60_000 });
    simdeckJson(["rotate-right", androidUDID], { timeoutMs: 60_000 });
    simdeckJson(["toggle-appearance", androidUDID], { timeoutMs: 60_000 });
    simdeckJson(["toggle-appearance", androidUDID], { timeoutMs: 60_000 });
  });
  await measuredStep("CLI logs", () => {
    const payload = simdeckJson([
      "logs",
      androidUDID,
      "--seconds",
      "1",
      "--limit",
      "5",
    ]);
    assert.ok(
      Array.isArray(payload.entries),
      "CLI logs did not return entries",
    );
  });
}

async function runJsSurface() {
  await measuredStep("JS list includes Android", async () => {
    assertAndroidListed(await session.list(), androidUDID);
  });
  await measuredStep("JS chromeProfile", async () => {
    const profile = await session.chromeProfile(androidUDID);
    assertJson(profile, "chromeProfile");
  });
  await measuredStep("JS Android tree", async () => {
    assertRoots(
      await session.tree(androidUDID, {
        source: "android-uiautomator",
        maxDepth: 2,
      }),
      "JS tree",
    );
  });
  await measuredStep("JS screenshot", async () => {
    assertPngBuffer(await session.screenshot(androidUDID));
  });
  await measuredStep("JS pasteboard behavior", async () => {
    try {
      await session.pasteboardSet(androidUDID, "simdeck android js");
      assert.equal(
        await session.pasteboardGet(androidUDID),
        "simdeck android js",
      );
    } catch (error) {
      assertClipboardUnsupported(error);
    }
  });
  await measuredStep("JS app launch and URL", async () => {
    await session.launch(androidUDID, "com.android.settings");
    await session.openUrl(androidUDID, "https://example.com");
    await session.home(androidUDID);
  });
  await measuredStep("JS pointer gestures", async () => {
    await session.tap(androidUDID, 0.5, 0.5);
    await session.touch(androidUDID, 0.5, 0.5, "began");
    await session.touch(androidUDID, 0.5, 0.5, "ended");
    await session.swipe(androidUDID, 0.5, 0.75, 0.5, 0.25, {
      durationMs: 120,
      steps: 4,
    });
    await session.gesture(androidUDID, "scroll-down", {
      durationMs: 120,
      delta: 0.2,
    });
  });
  await measuredStep("JS keyboard and system controls", async () => {
    await session.key(androidUDID, 40);
    await session.keySequence(androidUDID, [11, 8, 15, 15, 18], {
      delayMs: 5,
    });
    await session.typeText(androidUDID, "simdeck");
    await session.dismissKeyboard(androidUDID);
    await session.button(androidUDID, "back");
    await session.appSwitcher(androidUDID);
    await session.home(androidUDID);
  });
  await measuredStep("JS rotation and appearance restore", async () => {
    await session.rotateLeft(androidUDID);
    await session.rotateRight(androidUDID);
    await session.toggleAppearance(androidUDID);
    await session.toggleAppearance(androidUDID);
  });
  await measuredStep("JS logs", async () => {
    assert.ok(
      Array.isArray(await session.logs(androidUDID, { limit: 5, seconds: 1 })),
      "JS logs did not return entries",
    );
  });
  await measuredStep("JS batch Android controls", async () => {
    const payload = await session.batch(androidUDID, [
      {
        action: "touchSequence",
        events: [
          { x: 0.5, y: 0.5, phase: "began", delayMsAfter: 20 },
          { x: 0.5, y: 0.5, phase: "ended" },
        ],
      },
      {
        action: "swipe",
        startX: 0.5,
        startY: 0.75,
        endX: 0.5,
        endY: 0.25,
        durationMs: 100,
        steps: 4,
      },
      { action: "key", keyCode: 40 },
      { action: "home" },
      { action: "describe", source: "android-uiautomator", maxDepth: 1 },
    ]);
    assertJson(payload, "JS batch");
  });
}

function resolveAndroidDevice() {
  const emulator = androidSdkTool("emulator/emulator");
  if (!emulator) {
    if (requestedAvd) {
      throw new Error("Android SDK emulator binary was not found.");
    }
    return null;
  }
  const avds = runText(emulator, ["-list-avds"], { timeoutMs: 30_000 })
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  const running = runningAndroidAvds();
  if (requestedAvd) {
    const avdName = requestedAvd.replace(/^android:/, "");
    if (!avds.includes(avdName)) {
      throw new Error(
        `SIMDECK_INTEGRATION_ANDROID_AVD=${requestedAvd} was not found. Available Android AVDs: ${avds.join(", ")}`,
      );
    }
    return {
      udid: `android:${avdName}`,
      name: avdName,
      isBooted: running.has(avdName),
    };
  }
  const avdName = avds[0];
  return avdName
    ? {
        udid: `android:${avdName}`,
        name: avdName,
        isBooted: running.has(avdName),
      }
    : null;
}

function runningAndroidAvds() {
  const adb = androidSdkTool("platform-tools/adb");
  if (!adb) {
    return new Set();
  }
  const devices = runText(adb, ["devices"], { timeoutMs: 30_000 })
    .split(/\r?\n/)
    .map((line) => line.trim().split(/\s+/))
    .filter(
      ([serial, state]) =>
        serial?.startsWith("emulator-") && state === "device",
    )
    .map(([serial]) => serial);
  const avds = new Set();
  for (const serial of devices) {
    const name = androidAvdNameForSerial(adb, serial);
    if (name) {
      avds.add(name);
    }
  }
  return avds;
}

function androidAvdNameForSerial(adb, serial) {
  for (const property of ["ro.boot.qemu.avd_name", "ro.kernel.qemu.avd_name"]) {
    try {
      const name = runText(adb, ["-s", serial, "shell", "getprop", property], {
        timeoutMs: 10_000,
      }).trim();
      if (name) {
        return name;
      }
    } catch {
      // Fall through to the next lookup. ADB can briefly expose devices
      // before every shell service is ready.
    }
  }
  try {
    return runText(adb, ["-s", serial, "emu", "avd", "name"], {
      timeoutMs: 10_000,
    })
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line && line !== "OK");
  } catch {
    return null;
  }
}

function androidSdkTool(relativePath) {
  const roots = [
    process.env.ANDROID_HOME,
    process.env.ANDROID_SDK_ROOT,
    path.join(os.homedir(), "Library", "Android", "sdk"),
    path.join(os.homedir(), "Android", "Sdk"),
  ].filter(Boolean);
  for (const root of roots) {
    const candidate = path.join(root, relativePath);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  return null;
}

function simulatorList(payload) {
  return Array.isArray(payload?.simulators) ? payload.simulators : [];
}

async function waitForBooted(udid) {
  const avdName = udid.replace(/^android:/, "");
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    if (runningAndroidAvds().has(avdName)) {
      return;
    }
    await sleep(1_000);
  }
  throw new Error(`Timed out waiting for ${udid} to boot.`);
}

async function cleanupAndroid() {
  if (!session || !androidUDID || !shutdownAndroidAfterRun) {
    return;
  }
  try {
    await measuredStep("shutdown Android emulator", () =>
      session.shutdown(androidUDID),
    );
  } catch (error) {
    console.error(`Failed to shutdown ${androidUDID}: ${error.message}`);
  }
}

function cleanupSync() {
  if (session) {
    session.close();
    session = null;
  }
}

async function measuredStep(label, run, options = {}) {
  const started = process.hrtime.bigint();
  let timeoutId = null;
  if (verbose) {
    console.log(`> ${label}`);
  }
  try {
    const timeoutMs = options.timeoutMs ?? defaultStepTimeoutMs;
    const result = await Promise.race([
      Promise.resolve().then(run),
      new Promise((_, reject) => {
        timeoutId = setTimeout(
          () => reject(new Error(`${label} timed out after ${timeoutMs}ms`)),
          timeoutMs,
        );
      }),
    ]);
    const elapsedMs = Number(process.hrtime.bigint() - started) / 1_000_000;
    stepTimings.push({ label, elapsedMs });
    if (verbose) {
      console.log(`ok ${label} ${elapsedMs.toFixed(1)}ms`);
    }
    if (options.timeoutMs && elapsedMs > options.timeoutMs) {
      throw new Error(
        `${label} exceeded ${options.timeoutMs}ms budget (${elapsedMs.toFixed(
          1,
        )}ms)`,
      );
    }
    return result;
  } catch (error) {
    const elapsedMs = Number(process.hrtime.bigint() - started) / 1_000_000;
    console.error(`fail ${label} ${elapsedMs.toFixed(1)}ms`);
    throw error;
  } finally {
    if (timeoutId) {
      clearTimeout(timeoutId);
    }
  }
}

function printTimingSummary() {
  if (!verbose || stepTimings.length === 0) {
    return;
  }
  console.log("\nTiming summary:");
  for (const entry of stepTimings) {
    console.log(`  ${entry.label.padEnd(36)} ${entry.elapsedMs.toFixed(1)}ms`);
  }
}

function simdeckJson(args, options = {}) {
  return JSON.parse(
    runText(simdeck, ["--server-url", session.endpoint, ...args], options),
  );
}

function runText(command, args, options = {}) {
  return runBuffer(command, args, options).toString("utf8").trim();
}

function runBuffer(command, args, options = {}) {
  const result = runCommand(command, args, options);
  if (result.status !== 0) {
    throw new Error(
      `${[command, ...args].join(" ")} failed with ${result.status}\n${result.stderr}`,
    );
  }
  return result.stdoutBuffer ?? Buffer.alloc(0);
}

function runCommand(command, args, options = {}) {
  if (verbose) {
    console.log(`$ ${[command, ...args].join(" ")}`);
  }
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: null,
    timeout: options.timeoutMs ?? 120_000,
    maxBuffer: options.maxBuffer ?? 4 * 1024 * 1024,
  });
  return {
    status: result.status ?? 1,
    stdout: (result.stdout ?? Buffer.alloc(0)).toString("utf8"),
    stderr: (result.stderr ?? Buffer.alloc(0)).toString("utf8"),
    stdoutBuffer: result.stdout ?? Buffer.alloc(0),
  };
}

function assertClipboardUnsupported(errorOrResult) {
  const message =
    errorOrResult instanceof Error
      ? errorOrResult.message
      : `${errorOrResult.stdout}\n${errorOrResult.stderr}`;
  assert.match(
    message,
    /clipboard shell service is not implemented|No shell command implementation/i,
  );
}

function assertAndroidListed(payload, udid) {
  assert.ok(
    simulatorList(payload).some(
      (simulator) => simulator.udid === udid || simulator.id === udid,
    ),
    `${udid} was not present in simulator list`,
  );
}

function assertJson(payload, label) {
  assert.ok(payload && typeof payload === "object", `${label} was not JSON`);
}

function assertRoots(payload, label) {
  assertJson(payload, label);
  assert.ok(Array.isArray(payload.roots), `${label} missing roots array`);
  assert.ok(payload.roots.length > 0, `${label} returned no roots`);
}

function centerOfFirstRoot(payload) {
  const frame = payload?.roots?.[0]?.frame;
  if (!frame || typeof frame !== "object") {
    return null;
  }
  const x = Number(
    Array.isArray(frame) ? frame[0] : (frame.x ?? frame.minX ?? 0),
  );
  const y = Number(
    Array.isArray(frame) ? frame[1] : (frame.y ?? frame.minY ?? 0),
  );
  const width = Number(Array.isArray(frame) ? frame[2] : (frame.width ?? 0));
  const height = Number(Array.isArray(frame) ? frame[3] : (frame.height ?? 0));
  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return null;
  }
  return {
    x: Math.round(x + width / 2),
    y: Math.round(y + height / 2),
  };
}

function assertPngBuffer(buffer) {
  const signature = Buffer.from([
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
  ]);
  assert.ok(
    buffer.length > signature.length &&
      buffer.subarray(0, signature.length).equals(signature),
    "screenshot did not return a PNG buffer",
  );
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
