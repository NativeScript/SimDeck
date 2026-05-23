#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const simdeckBin = path.join(repoRoot, "build", "simdeck");
const defaultReps = 3;
const settingsBundleId = "com.apple.Preferences";
const benchmarkUrl = "https://example.com";
const screenTimeId = "com.apple.settings.screenTime";
const backButtonId = "BackButton";
const markdownWidth = 100;

const options = parseArgs(process.argv.slice(2));
const tempRoot = path.join(tmpdir(), "simdeck-agent-control-benchmark");
mkdirSync(tempRoot, { recursive: true });

const rows = [];
const cleanupTasks = [];
let argentEnv = null;

main()
  .catch((error) => {
    console.error(error?.stack || String(error));
    process.exitCode = 1;
  })
  .finally(async () => {
    for (const cleanup of cleanupTasks.reverse()) {
      try {
        await cleanup();
      } catch {
        // Best-effort cleanup only.
      }
    }
  });

async function main() {
  assertBinary(simdeckBin, "Run npm run build:cli before benchmarking.");

  const versions = {
    simdeck: readVersion(simdeckBin, ["--version"]),
    "agent-device": readVersion("agent-device", ["--version"]),
    argent: readVersion("argent", ["--version"]),
  };

  const udid = options.udid || selectBootedIphoneUdid();
  if (!udid) {
    throw new Error(
      "No booted iPhone simulator found. Boot one first or pass --udid <UDID>.",
    );
  }

  const metadata = {
    generatedAt: new Date().toISOString(),
    udid,
    reps: options.reps,
    versions,
    notes: [
      "Cold rows stop tool daemons where practical before measuring the next first command.",
      "Setup/reset work is excluded from action timings and uses xcrun/simdeck to normalize Settings state.",
      "Argent has no selector tap/wait primitive in this comparison, so comparable navigation uses AX-derived coordinates.",
    ],
  };

  console.log(`Benchmarking simulator ${udid}`);
  console.log(
    `Versions: simdeck ${versions.simdeck}, agent-device ${versions["agent-device"]}, argent ${versions.argent}`,
  );

  await benchmarkSimDeck(udid);
  await benchmarkAgentDevice(udid);
  await benchmarkArgent(udid);

  const report = buildReport(metadata, rows);
  const outDir =
    options.outDir ||
    path.join(tempRoot, new Date().toISOString().replace(/[:.]/g, "-"));
  mkdirSync(outDir, { recursive: true });
  const jsonPath = path.join(outDir, "agent-control-benchmark.json");
  const markdownPath = path.join(outDir, "agent-control-benchmark.md");
  writeFileSync(jsonPath, JSON.stringify({ metadata, rows }, null, 2));
  writeFileSync(markdownPath, report);

  console.log("");
  console.log(report);
  console.log("");
  console.log(`Wrote ${jsonPath}`);
  console.log(`Wrote ${markdownPath}`);
}

async function benchmarkSimDeck(udid) {
  section("simdeck");

  await sample("simdeck", "tool-start", "cold", 1, async () => {
    run(simdeckBin, ["daemon", "stop"], {
      allowFailure: true,
      timeoutMs: 5000,
    });
    return run(simdeckBin, ["daemon", "start", "--port", "4310"], {
      timeoutMs: 20000,
    });
  });

  await sample("simdeck", "tool-start", "hot", 1, async () =>
    run(simdeckBin, ["daemon", "start", "--port", "4310"], {
      timeoutMs: 10000,
    }),
  );

  run(simdeckBin, ["use", udid], { timeoutMs: 10000, allowFailure: true });

  await sample("simdeck", "list-devices", "hot", options.reps, async () =>
    run(simdeckBin, ["list", "--format", "compact-json"], { timeoutMs: 10000 }),
  );

  await sample("simdeck", "launch-settings", "hot", options.reps, async () =>
    run(simdeckBin, ["launch", settingsBundleId], { timeoutMs: 15000 }),
  );

  await sample("simdeck", "open-url", "hot", options.reps, async () =>
    run(simdeckBin, ["open-url", benchmarkUrl], { timeoutMs: 15000 }),
  );

  await withSettingsRoot(udid);
  await sample("simdeck", "describe-full", "hot", options.reps, async () =>
    run(simdeckBin, ["describe", "--format", "agent", "--max-depth", "8"], {
      timeoutMs: 10000,
    }),
  );

  await withSettingsRoot(udid);
  await sample(
    "simdeck",
    "describe-interactive",
    "hot",
    options.reps,
    async () =>
      run(
        simdeckBin,
        ["describe", "--format", "agent", "--max-depth", "8", "-i"],
        { timeoutMs: 10000 },
      ),
  );

  await withSettingsRoot(udid);
  await sample(
    "simdeck",
    "wait-visible-screen-time",
    "hot",
    options.reps,
    async () =>
      run(
        simdeckBin,
        ["wait-for", "--id", screenTimeId, "--timeout-ms", "5000"],
        { timeoutMs: 8000 },
      ),
  );

  await sample(
    "simdeck",
    "tap-screen-time-selector",
    "hot",
    options.reps,
    async () => {
      await withSettingsRoot(udid);
      const result = run(
        simdeckBin,
        [
          "tap",
          "--id",
          screenTimeId,
          "--expect-id",
          backButtonId,
          "--expect-timeout-ms",
          "5000",
        ],
        { timeoutMs: 10000 },
      );
      run(simdeckBin, ["back"], { timeoutMs: 5000, allowFailure: true });
      return result;
    },
  );

  await sample("simdeck", "back", "hot", options.reps, async () => {
    await withSettingsPane(udid);
    return run(simdeckBin, ["back"], { timeoutMs: 10000 });
  });

  await sample(
    "simdeck",
    "swipe-settings-list",
    "hot",
    options.reps,
    async () => {
      await withSettingsRoot(udid);
      return run(
        simdeckBin,
        [
          "swipe",
          "--normalized",
          "0.5",
          "0.75",
          "0.5",
          "0.25",
          "--duration-ms",
          "250",
        ],
        { timeoutMs: 10000 },
      );
    },
  );

  await sample("simdeck", "screenshot", "hot", options.reps, async (_, index) =>
    run(
      simdeckBin,
      [
        "screenshot",
        "--output",
        path.join(tempRoot, `simdeck-screen-${index}.png`),
      ],
      { timeoutMs: 15000 },
    ),
  );

  await sample("simdeck", "home", "hot", options.reps, async () => {
    await withSettingsRoot(udid);
    return run(simdeckBin, ["home"], { timeoutMs: 10000 });
  });

  await sample(
    "simdeck",
    "tap-and-back-batch",
    "hot",
    options.reps,
    async () => {
      await withSettingsRoot(udid);
      return run(
        simdeckBin,
        [
          "batch",
          "--step",
          `tap --id ${screenTimeId} --expect-id ${backButtonId}`,
          "--step",
          "back",
        ],
        { timeoutMs: 15000 },
      );
    },
  );
}

async function benchmarkAgentDevice(udid) {
  section("agent-device");

  await sample("agent-device", "tool-start", "cold", 1, async () => {
    stopAgentDevice();
    return run(
      "agent-device",
      ["devices", "--platform", "ios", "--udid", udid, "--json"],
      { timeoutMs: 45000 },
    );
  });

  await sample("agent-device", "tool-start", "hot", 1, async () =>
    run(
      "agent-device",
      ["devices", "--platform", "ios", "--udid", udid, "--json"],
      { timeoutMs: 20000 },
    ),
  );

  await sample("agent-device", "list-devices", "hot", options.reps, async () =>
    run(
      "agent-device",
      ["devices", "--platform", "ios", "--udid", udid, "--json"],
      { timeoutMs: 20000 },
    ),
  );

  await sample(
    "agent-device",
    "launch-settings",
    "hot",
    options.reps,
    async () =>
      run(
        "agent-device",
        ["open", "Settings", "--platform", "ios", "--udid", udid, "--json"],
        { timeoutMs: 30000 },
      ),
  );

  await sample("agent-device", "open-url", "hot", options.reps, async () =>
    run(
      "agent-device",
      ["open", benchmarkUrl, "--platform", "ios", "--udid", udid, "--json"],
      { timeoutMs: 30000 },
    ),
  );

  await withAgentDeviceSettingsRoot(udid);
  await sample("agent-device", "describe-full", "hot", options.reps, async () =>
    run("agent-device", ["snapshot", "--platform", "ios", "--udid", udid], {
      timeoutMs: 30000,
    }),
  );

  await withAgentDeviceSettingsRoot(udid);
  await sample(
    "agent-device",
    "describe-interactive",
    "hot",
    options.reps,
    async () =>
      run(
        "agent-device",
        ["snapshot", "-i", "--platform", "ios", "--udid", udid],
        { timeoutMs: 30000 },
      ),
  );

  await withAgentDeviceSettingsRoot(udid);
  await sample(
    "agent-device",
    "wait-visible-screen-time",
    "hot",
    options.reps,
    async () =>
      run(
        "agent-device",
        [
          "wait",
          "Screen Time",
          "5000",
          "--platform",
          "ios",
          "--udid",
          udid,
          "--json",
        ],
        { timeoutMs: 10000 },
      ),
  );

  await sample(
    "agent-device",
    "tap-screen-time-selector",
    "hot",
    options.reps,
    async () => {
      await withAgentDeviceSettingsRoot(udid);
      const result = run(
        "agent-device",
        [
          "find",
          "Screen Time",
          "click",
          "--first",
          "--platform",
          "ios",
          "--udid",
          udid,
          "--json",
        ],
        { timeoutMs: 30000 },
      );
      run("agent-device", ["back", "--platform", "ios", "--udid", udid], {
        timeoutMs: 10000,
        allowFailure: true,
      });
      return withNote(result, "Uses agent-device find text + first match.");
    },
  );

  await sample("agent-device", "back", "hot", options.reps, async () => {
    await withAgentDeviceSettingsPane(udid);
    return run(
      "agent-device",
      ["back", "--platform", "ios", "--udid", udid, "--json"],
      { timeoutMs: 20000 },
    );
  });

  await sample(
    "agent-device",
    "swipe-settings-list",
    "hot",
    options.reps,
    async () => {
      await withAgentDeviceSettingsRoot(udid);
      const size = currentScreenSize();
      return run(
        "agent-device",
        [
          "swipe",
          String(Math.round(size.width * 0.5)),
          String(Math.round(size.height * 0.75)),
          String(Math.round(size.width * 0.5)),
          String(Math.round(size.height * 0.25)),
          "250",
          "--platform",
          "ios",
          "--udid",
          udid,
          "--json",
        ],
        { timeoutMs: 20000 },
      );
    },
  );

  await sample(
    "agent-device",
    "screenshot",
    "hot",
    options.reps,
    async (_, index) =>
      run(
        "agent-device",
        [
          "screenshot",
          path.join(tempRoot, `agent-device-screen-${index}.png`),
          "--platform",
          "ios",
          "--udid",
          udid,
          "--json",
        ],
        { timeoutMs: 30000 },
      ),
  );

  await sample("agent-device", "home", "hot", options.reps, async () => {
    await withAgentDeviceSettingsRoot(udid);
    return run(
      "agent-device",
      ["home", "--platform", "ios", "--udid", udid, "--json"],
      { timeoutMs: 20000 },
    );
  });

  await sample(
    "agent-device",
    "tap-and-back-batch",
    "hot",
    options.reps,
    async () => {
      await withAgentDeviceSettingsRoot(udid);
      return withNote(
        run(
          "agent-device",
          [
            "batch",
            "--steps",
            JSON.stringify([
              {
                command: "find",
                positionals: ["Screen Time", "click"],
                flags: { platform: "ios", udid, first: true },
              },
              {
                command: "wait",
                positionals: ["App & Website Activity", "5000"],
                flags: { platform: "ios", udid },
              },
              {
                command: "find",
                positionals: ["Settings", "click"],
                flags: { platform: "ios", udid, first: true },
              },
            ]),
            "--platform",
            "ios",
            "--udid",
            udid,
            "--json",
          ],
          { timeoutMs: 30000 },
        ),
        "Uses text find for Screen Time and the Settings back control.",
      );
    },
  );

  run("agent-device", ["close", "--platform", "ios", "--udid", udid], {
    timeoutMs: 10000,
    allowFailure: true,
  });
  stopAgentDevice();
}

async function benchmarkArgent(udid) {
  section("argent");

  await sample("argent", "tool-start", "cold", 1, async () => {
    const started = await startArgentToolServer();
    argentEnv = started.env;
    cleanupTasks.push(started.stop);
    return started.result;
  });

  await sample("argent", "tool-start", "hot", 1, async () =>
    run("argent", ["tools", "--json"], { env: argentEnv, timeoutMs: 15000 }),
  );

  await sample("argent", "list-devices", "hot", options.reps, async () =>
    run("argent", ["run", "list-devices", "--json"], {
      env: argentEnv,
      timeoutMs: 20000,
    }),
  );

  await sample("argent", "launch-settings", "hot", options.reps, async () =>
    run(
      "argent",
      [
        "run",
        "launch-app",
        "--udid",
        udid,
        "--bundleId",
        settingsBundleId,
        "--json",
      ],
      { env: argentEnv, timeoutMs: 30000 },
    ),
  );

  await sample("argent", "open-url", "hot", options.reps, async () =>
    run(
      "argent",
      ["run", "open-url", "--udid", udid, "--url", benchmarkUrl, "--json"],
      { env: argentEnv, timeoutMs: 30000 },
    ),
  );

  await withSettingsRoot(udid);
  await sample("argent", "describe-full", "hot", options.reps, async () =>
    run("argent", ["run", "describe", "--udid", udid, "--json"], {
      env: argentEnv,
      timeoutMs: 30000,
    }),
  );

  recordUnsupported(
    "argent",
    "describe-interactive",
    "hot",
    "No interactive-only describe/snapshot flag in v0.8.0.",
  );

  recordUnsupported(
    "argent",
    "wait-visible-screen-time",
    "hot",
    "No wait-for-selector command in v0.8.0.",
  );

  await sample(
    "argent",
    "tap-screen-time-coordinate",
    "hot",
    options.reps,
    async () => {
      await withSettingsRoot(udid);
      const target = await argentElementCenter(udid, "Screen Time");
      const result = run(
        "argent",
        [
          "run",
          "gesture-tap",
          "--udid",
          udid,
          "--x",
          String(target.x),
          "--y",
          String(target.y),
          "--json",
        ],
        { env: argentEnv, timeoutMs: 20000 },
      );
      run(simdeckBin, ["back"], { timeoutMs: 5000, allowFailure: true });
      return withNote(
        result,
        `coordinate fallback from AX label "${target.label}"`,
      );
    },
  );

  await sample(
    "argent",
    "back-button-command",
    "hot",
    options.reps,
    async () => {
      await withSettingsPane(udid);
      const result = run(
        "argent",
        ["run", "button", "--udid", udid, "--button", "back", "--json"],
        { env: argentEnv, timeoutMs: 20000 },
      );
      run(simdeckBin, ["back"], { timeoutMs: 5000, allowFailure: true });
      return withNote(
        result,
        "Measures Argent's button back primitive; on iOS Settings it may be a no-op.",
      );
    },
  );

  await sample("argent", "back-coordinate", "hot", options.reps, async () => {
    await withSettingsPane(udid);
    const target = await argentElementCenter(udid, "Settings");
    return withNote(
      run(
        "argent",
        [
          "run",
          "gesture-tap",
          "--udid",
          udid,
          "--x",
          String(target.x),
          "--y",
          String(target.y),
          "--json",
        ],
        { env: argentEnv, timeoutMs: 20000 },
      ),
      `coordinate fallback from AX label "${target.label}"`,
    );
  });

  await sample(
    "argent",
    "swipe-settings-list",
    "hot",
    options.reps,
    async () => {
      await withSettingsRoot(udid);
      return run(
        "argent",
        [
          "run",
          "gesture-swipe",
          "--udid",
          udid,
          "--fromX",
          "0.5",
          "--fromY",
          "0.75",
          "--toX",
          "0.5",
          "--toY",
          "0.25",
          "--durationMs",
          "250",
          "--json",
        ],
        { env: argentEnv, timeoutMs: 20000 },
      );
    },
  );

  await sample("argent", "screenshot", "hot", options.reps, async (_, index) =>
    run(
      "argent",
      [
        "run",
        "screenshot",
        "--udid",
        udid,
        "--out",
        path.join(tempRoot, `argent-screen-${index}.png`),
      ],
      { env: argentEnv, timeoutMs: 30000 },
    ),
  );

  await sample("argent", "home", "hot", options.reps, async () => {
    await withSettingsRoot(udid);
    return run(
      "argent",
      ["run", "button", "--udid", udid, "--button", "home", "--json"],
      { env: argentEnv, timeoutMs: 20000 },
    );
  });

  await sample(
    "argent",
    "tap-and-back-sequence",
    "hot",
    options.reps,
    async () => {
      await withSettingsRoot(udid);
      const screenTime = await argentElementCenter(udid, "Screen Time");
      const result = run(
        "argent",
        [
          "run",
          "run-sequence",
          "--udid",
          udid,
          "--steps-json",
          JSON.stringify([
            {
              tool: "gesture-tap",
              args: { udid, x: screenTime.x, y: screenTime.y },
            },
          ]),
          "--json",
        ],
        { env: argentEnv, timeoutMs: 30000 },
      );
      run(simdeckBin, ["back"], { timeoutMs: 5000, allowFailure: true });
      return withNote(
        result,
        "Run sequence can replay known coordinates but cannot observe/wait between Settings steps.",
      );
    },
  );
}

async function withSettingsRoot(udid) {
  run("xcrun", ["simctl", "terminate", udid, settingsBundleId], {
    timeoutMs: 10000,
    allowFailure: true,
  });
  run(simdeckBin, ["launch", udid, settingsBundleId], {
    timeoutMs: 15000,
    allowFailure: true,
  });
  for (let index = 0; index < 5; index += 1) {
    const snapshot = run(
      simdeckBin,
      ["describe", "--format", "agent", "--max-depth", "4", "-i"],
      { timeoutMs: 10000, allowFailure: true },
    );
    const output = `${snapshot.stdout}\n${snapshot.stderr}`;
    if (output.includes(screenTimeId) && !output.includes(`#${backButtonId}`)) {
      return;
    }
    if (output.includes(`#${backButtonId}`)) {
      run(simdeckBin, ["back"], { timeoutMs: 5000, allowFailure: true });
      await sleep(250);
      continue;
    }
    await sleep(250);
  }
}

async function withSettingsPane(udid) {
  await withSettingsRoot(udid);
  run(
    simdeckBin,
    [
      "tap",
      "--id",
      screenTimeId,
      "--expect-id",
      backButtonId,
      "--expect-timeout-ms",
      "5000",
    ],
    { timeoutMs: 10000, allowFailure: true },
  );
}

async function withAgentDeviceSettingsRoot(udid) {
  run(
    "agent-device",
    [
      "open",
      "Settings",
      "--relaunch",
      "--platform",
      "ios",
      "--udid",
      udid,
      "--json",
    ],
    { timeoutMs: 30000, allowFailure: true },
  );
}

async function withAgentDeviceSettingsPane(udid) {
  await withAgentDeviceSettingsRoot(udid);
  run(
    "agent-device",
    [
      "find",
      "Screen Time",
      "click",
      "--first",
      "--platform",
      "ios",
      "--udid",
      udid,
      "--json",
    ],
    { timeoutMs: 30000, allowFailure: true },
  );
}

async function argentElementCenter(udid, label) {
  const result = run("argent", ["run", "describe", "--udid", udid, "--json"], {
    env: argentEnv,
    timeoutMs: 30000,
  });
  if (!result.ok) {
    throw new Error(`Argent describe failed while locating ${label}`);
  }
  let output = `${result.stdout}\n${result.stderr}`;
  try {
    const parsed = JSON.parse(result.stdout);
    if (typeof parsed.description === "string") {
      output = parsed.description;
    }
  } catch {
    // Text output is also supported.
  }
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const regex = new RegExp(
    `AX\\w+\\s+"${escaped}"[^\\n]*\\(([-0-9.]+),\\s*([-0-9.]+),\\s*([-0-9.]+),\\s*([-0-9.]+)\\)`,
    "i",
  );
  const match = output.match(regex);
  if (!match) {
    throw new Error(`Could not find Argent AX element "${label}"`);
  }
  const [, x, y, width, height] = match.map(Number);
  return {
    label,
    x: Number((x + width / 2).toFixed(4)),
    y: Number((y + height / 2).toFixed(4)),
  };
}

async function sample(tool, action, phase, count, fn) {
  for (let index = 0; index < count; index += 1) {
    const started = performance.now();
    let result;
    try {
      result = await fn(tool, index);
    } catch (error) {
      result = {
        ok: false,
        status: null,
        stdout: "",
        stderr: error?.stack || String(error),
        durationMs: performance.now() - started,
      };
    }
    rows.push({
      tool,
      action,
      phase,
      iteration: index + 1,
      durationMs: roundMs(result.durationMs ?? performance.now() - started),
      status: result.ok ? "ok" : "failed",
      exitCode: result.status ?? null,
      stdoutBytes: Buffer.byteLength(result.stdout || ""),
      stderrBytes: Buffer.byteLength(result.stderr || ""),
      note: result.note || conciseFailure(result),
    });
    const sampleLabel =
      count === 1 ? "" : ` ${String(index + 1).padStart(2, "0")}/${count}`;
    console.log(
      `${tool.padEnd(13)} ${phase.padEnd(4)} ${action.padEnd(28)}${sampleLabel} ${formatMs(
        result.durationMs,
      ).padStart(9)} ${result.ok ? "ok" : "failed"}`,
    );
  }
}

function recordUnsupported(tool, action, phase, note) {
  rows.push({
    tool,
    action,
    phase,
    iteration: 1,
    durationMs: null,
    status: "unsupported",
    exitCode: null,
    stdoutBytes: 0,
    stderrBytes: 0,
    note,
  });
  console.log(
    `${tool.padEnd(13)} ${phase.padEnd(4)} ${action.padEnd(28)} ${"n/a".padStart(
      9,
    )} unsupported`,
  );
}

function run(command, args, config = {}) {
  const started = performance.now();
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env: { ...process.env, ...(config.env || {}) },
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    timeout: config.timeoutMs || 30000,
  });
  const durationMs = performance.now() - started;
  const timedOut = result.error?.code === "ETIMEDOUT";
  const ok =
    !timedOut && !result.error && result.status === 0 && result.signal === null;
  return {
    ok: ok || Boolean(config.allowFailure),
    status: result.status,
    signal: result.signal,
    stdout: result.stdout || "",
    stderr:
      result.stderr ||
      (result.error ? `${result.error.name}: ${result.error.message}` : ""),
    durationMs,
    timedOut,
  };
}

function withNote(result, note) {
  return { ...result, note };
}

async function startArgentToolServer() {
  const port = await freePort();
  const packageRoot = globalPackageRoot("@swmansion/argent");
  const serverPath = path.join(packageRoot, "dist", "tool-server.cjs");
  const serverDir = path.join(packageRoot, "bin");
  const devtoolsDir = path.join(packageRoot, "dylibs");
  const env = {
    ...process.env,
    PORT: String(port),
    ARGENT_TOOLS_URL: `http://127.0.0.1:${port}`,
    ARGENT_SIMULATOR_SERVER_DIR: serverDir,
    ARGENT_NATIVE_DEVTOOLS_DIR: devtoolsDir,
  };
  const started = performance.now();
  const child = spawn("node", [serverPath, "start"], {
    cwd: repoRoot,
    env,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  child.stdout.on("data", (chunk) => {
    output += chunk.toString();
  });
  child.stderr.on("data", (chunk) => {
    output += chunk.toString();
  });

  const ready = await waitUntil(async () => {
    const response = run("argent", ["tools", "--json"], {
      env: { ARGENT_TOOLS_URL: env.ARGENT_TOOLS_URL },
      timeoutMs: 3000,
      allowFailure: true,
    });
    return response.status === 0;
  }, 15000);

  const result = {
    ok: ready,
    status: ready ? 0 : 1,
    stdout: output,
    stderr: ready ? "" : output || "Argent tool-server did not become ready.",
    durationMs: performance.now() - started,
  };

  return {
    env: { ARGENT_TOOLS_URL: env.ARGENT_TOOLS_URL },
    result,
    stop: async () => {
      if (!child.killed) {
        child.kill("SIGTERM");
        await sleep(500);
        if (!child.killed) {
          child.kill("SIGKILL");
        }
      }
    },
  };
}

function stopAgentDevice() {
  const pids = agentDevicePids();
  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // Process may already be gone.
    }
  }
  if (pids.length > 0) {
    spawnSync("sleep", ["0.5"]);
  }
  for (const pid of agentDevicePids()) {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // Process may already be gone.
    }
  }
}

function agentDevicePids() {
  const ps = spawnSync("ps", ["-axo", "pid=,command="], {
    encoding: "utf8",
    maxBuffer: 8 * 1024 * 1024,
  });
  if (ps.status !== 0) {
    return [];
  }
  const pids = [];
  const ownPid = process.pid;
  for (const line of ps.stdout.split("\n")) {
    const match = line.trim().match(/^(\d+)\s+(.*)$/);
    if (!match) {
      continue;
    }
    const pid = Number(match[1]);
    const command = match[2];
    if (pid === ownPid) {
      continue;
    }
    if (
      command.includes("agent-device") &&
      (command.includes("internal/daemon") ||
        command.includes("AgentDeviceRunnerUITests") ||
        command.includes("agent-device-daemon"))
    ) {
      pids.push(pid);
    }
  }
  return pids;
}

function selectBootedIphoneUdid() {
  const result = run(simdeckBin, ["list", "--format", "compact-json"], {
    timeoutMs: 15000,
  });
  if (!result.ok) {
    return null;
  }
  const parsed = JSON.parse(result.stdout);
  const simulators = Array.isArray(parsed.simulators) ? parsed.simulators : [];
  const candidates = simulators.filter(
    (simulator) =>
      simulator.isBooted &&
      String(simulator.deviceTypeName || simulator.name || "").includes(
        "iPhone",
      ) &&
      String(simulator.runtimeName || "").includes("iOS"),
  );
  for (const simulator of candidates) {
    run(simdeckBin, ["launch", simulator.udid, settingsBundleId], {
      timeoutMs: 15000,
      allowFailure: true,
    });
    const snapshot = run(
      simdeckBin,
      [
        "describe",
        simulator.udid,
        "--format",
        "agent",
        "--max-depth",
        "8",
        "-i",
      ],
      { timeoutMs: 15000, allowFailure: true },
    );
    if (snapshot.stdout.includes(screenTimeId)) {
      return simulator.udid;
    }
  }
  return (
    candidates[0]?.udid ||
    simulators.find((simulator) => simulator.isBooted)?.udid
  );
}

function currentScreenSize() {
  const result = run(
    simdeckBin,
    ["describe", "--format", "agent", "--max-depth", "1", "-i"],
    { timeoutMs: 10000 },
  );
  const match = result.stdout.match(
    /Application:[^\n]*@\S+\s+([0-9.]+)x([0-9.]+)/,
  );
  if (!match) {
    return { width: 402, height: 874 };
  }
  return { width: Number(match[1]), height: Number(match[2]) };
}

function readVersion(command, args) {
  const result = run(command, args, { timeoutMs: 5000, allowFailure: true });
  return (result.stdout || result.stderr || "unavailable")
    .trim()
    .split(/\s+/)
    .pop();
}

function globalPackageRoot(packageName) {
  const result = run("npm", ["root", "-g"], { timeoutMs: 10000 });
  if (!result.ok) {
    throw new Error("Could not resolve global npm root for Argent.");
  }
  return path.join(result.stdout.trim(), packageName);
}

function assertBinary(filePath, hint) {
  const result = spawnSync(filePath, ["--version"], {
    encoding: "utf8",
    timeout: 5000,
  });
  if (result.error) {
    throw new Error(`${filePath} is not runnable. ${hint}`);
  }
}

function buildReport(metadata, rawRows) {
  const groups = new Map();
  for (const row of rawRows) {
    const key = [
      row.tool,
      row.phase,
      row.action,
      row.status,
      row.note || "",
    ].join("\u0000");
    const group = groups.get(key) || {
      tool: row.tool,
      phase: row.phase,
      action: row.action,
      status: row.status,
      note: row.note || "",
      samples: [],
    };
    if (typeof row.durationMs === "number") {
      group.samples.push(row.durationMs);
    }
    if (row.status !== "ok") {
      group.status = row.status;
    }
    groups.set(key, group);
  }

  const lines = [
    "# Agent Control Benchmark",
    "",
    `- Generated: ${metadata.generatedAt}`,
    `- Simulator: ${metadata.udid}`,
    `- Repetitions: ${metadata.reps}`,
    `- Versions: SimDeck ${metadata.versions.simdeck}, agent-device ${metadata.versions["agent-device"]}, Argent ${metadata.versions.argent}`,
    "",
    "| Tool | Phase | Action | Samples | Median | Min | Max | Status | Notes |",
    "| --- | --- | --- | ---: | ---: | ---: | ---: | --- | --- |",
  ];

  for (const group of [...groups.values()].sort(compareGroups)) {
    const stats = summarize(group.samples);
    lines.push(
      [
        group.tool,
        group.phase,
        group.action,
        String(group.samples.length || 0),
        stats ? formatMs(stats.median) : "n/a",
        stats ? formatMs(stats.min) : "n/a",
        stats ? formatMs(stats.max) : "n/a",
        group.status,
        truncate(group.note, markdownWidth),
      ]
        .map(escapeMarkdownCell)
        .join(" | ")
        .replace(/^/, "| ")
        .replace(/$/, " |"),
    );
  }

  lines.push("");
  lines.push("## Notes");
  for (const note of metadata.notes) {
    lines.push(`- ${note}`);
  }
  lines.push("");
  return `${lines.join("\n")}\n`;
}

function summarize(samples) {
  if (!samples.length) {
    return null;
  }
  const sorted = samples.slice().sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  const median =
    sorted.length % 2 === 0
      ? (sorted[middle - 1] + sorted[middle]) / 2
      : sorted[middle];
  return {
    min: sorted[0],
    max: sorted[sorted.length - 1],
    median,
  };
}

function compareGroups(left, right) {
  const toolOrder = ["simdeck", "agent-device", "argent"];
  const phaseOrder = ["cold", "hot"];
  const actionOrder = [
    "tool-start",
    "list-devices",
    "launch-settings",
    "open-url",
    "describe-full",
    "describe-interactive",
    "wait-visible-screen-time",
    "tap-screen-time-selector",
    "tap-screen-time-coordinate",
    "back",
    "back-button-command",
    "back-coordinate",
    "swipe-settings-list",
    "screenshot",
    "home",
    "tap-and-back-batch",
    "tap-and-back-sequence",
  ];
  return (
    order(toolOrder, left.tool) - order(toolOrder, right.tool) ||
    order(phaseOrder, left.phase) - order(phaseOrder, right.phase) ||
    order(actionOrder, left.action) - order(actionOrder, right.action)
  );
}

function order(list, value) {
  const index = list.indexOf(value);
  return index === -1 ? list.length : index;
}

function formatMs(value) {
  if (typeof value !== "number" || Number.isNaN(value)) {
    return "n/a";
  }
  return `${Math.round(value)} ms`;
}

function roundMs(value) {
  return Math.round(value * 100) / 100;
}

function escapeMarkdownCell(value) {
  return String(value || "")
    .replace(/\|/g, "\\|")
    .replace(/\n/g, " ");
}

function truncate(value, width) {
  if (!value || value.length <= width) {
    return value || "";
  }
  return `${value.slice(0, width - 3)}...`;
}

function conciseFailure(result) {
  if (!result || result.ok) {
    return "";
  }
  const message = `${result.stderr || result.stdout || ""}`.trim();
  return truncate(message.replace(/\s+/g, " "), 160);
}

function section(name) {
  console.log("");
  console.log(`== ${name} ==`);
}

function parseArgs(args) {
  const parsed = {
    udid: null,
    reps: Number(process.env.SIMDECK_BENCH_REPS || defaultReps),
    outDir: process.env.SIMDECK_BENCH_OUT_DIR || null,
  };
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--udid") {
      parsed.udid = args[++index];
    } else if (arg === "--reps") {
      parsed.reps = Number(args[++index]);
    } else if (arg === "--out-dir") {
      parsed.outDir = path.resolve(args[++index]);
    } else if (arg === "--help" || arg === "-h") {
      console.log(`Usage: npm run bench:agent-control -- [--udid <UDID>] [--reps 3] [--out-dir <path>]

Benchmarks cold start and hot action latency for simdeck, agent-device, and Argent
against a booted iOS Settings simulator flow.`);
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  if (!Number.isFinite(parsed.reps) || parsed.reps < 1) {
    throw new Error("--reps must be a positive number.");
  }
  parsed.reps = Math.floor(parsed.reps);
  return parsed;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitUntil(predicate, timeoutMs) {
  const deadline = performance.now() + timeoutMs;
  while (performance.now() < deadline) {
    if (await predicate()) {
      return true;
    }
    await sleep(250);
  }
  return false;
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close(() => resolve(address.port));
    });
  });
}
