import assert from "node:assert/strict";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { test } from "node:test";

const iosAction = readFileSync(
  new URL("../actions/run-ios-comment-session/action.yml", import.meta.url),
  "utf8",
);
const androidAction = readFileSync(
  new URL("../actions/run-android-comment-session/action.yml", import.meta.url),
  "utf8",
);
const ciWorkflow = readFileSync(
  new URL("../.github/workflows/ci.yml", import.meta.url),
  "utf8",
);
const packageJson = JSON.parse(
  readFileSync(new URL("../package.json", import.meta.url), "utf8"),
);
const cliWrapper = readFileSync(
  new URL("../packages/cli/bin/simdeck.mjs", import.meta.url),
  "utf8",
);
const androidIntegration = readFileSync(
  new URL("../scripts/integration/android.mjs", import.meta.url),
  "utf8",
);

function indexOfStep(action, name) {
  const index = action.indexOf(`- name: ${name}`);
  assert.notEqual(index, -1, `${name} step should exist`);
  return index;
}

function stepSlice(action, name, nextName) {
  const startIndex = indexOfStep(action, name);
  const endIndex =
    nextName === undefined ? action.length : indexOfStep(action, nextName);
  assert(endIndex > startIndex, `${nextName} should run after ${name}`);
  return action.slice(startIndex, endIndex);
}

const darwinTest = process.platform === "darwin" ? test : test.skip;

test("npm package declares Windows Android host support", () => {
  assert.ok(packageJson.os.includes("darwin"));
  assert.ok(packageJson.os.includes("linux"));
  assert.ok(packageJson.os.includes("win32"));
  assert.ok(packageJson.files.includes("build/simdeck-bin-win32-x64.exe"));
});

test("npm CLI wrapper resolves Windows x64 native binary", () => {
  assert.match(cliWrapper, /win32-x64/);
  assert.match(cliWrapper, /simdeck-bin-win32-x64\.exe/);
});

test("CI runs Android emulator integration on Linux and Windows", () => {
  assert.match(ciWorkflow, /integration-android:/);
  assert.match(ciWorkflow, /os:\s*\[\s*ubuntu-latest,\s*windows-latest\s*\]/);
  assert.match(ciWorkflow, /matrix\.os/);
  assert.match(ciWorkflow, /SimDeck_Pixel_CI/);
  assert.match(ciWorkflow, /test:integration:android/);
});

test("Windows Android CI boot path is bounded and diagnostic", () => {
  const windowsBootStep = stepSlice(
    ciWorkflow,
    "Create and boot Android emulator (Windows)",
    "Android emulator integration tests (Windows)",
  );

  assert.match(windowsBootStep, /\$accelSupported = \$LASTEXITCODE -eq 0/);
  assert.match(windowsBootStep, /google_atd/);
  assert.match(windowsBootStep, /"-qt-hide-window"/);
  assert.match(windowsBootStep, /"-feature", "-Vulkan"/);
  assert.match(windowsBootStep, /"-accel", "on"/);
  assert.match(windowsBootStep, /"-accel", "off"/);
  assert.match(windowsBootStep, /\$serial = "emulator-5554"/);
  assert.match(windowsBootStep, /\$devices -match "\$serial\\s\+device"/);
  assert.doesNotMatch(windowsBootStep, /device\|offline/);
  assert.match(windowsBootStep, /-RedirectStandardOutput \$stdout/);
  assert.match(windowsBootStep, /Write-EmulatorDiagnostics/);
  assert.match(
    windowsBootStep,
    /deviceDeadline = \(Get-Date\)\.AddMinutes\(10\)/,
  );
  assert.doesNotMatch(windowsBootStep, /wait-for-device/);
});

test("Android integration runner resolves Windows executables", () => {
  assert.match(androidIntegration, /fileURLToPath/);
  assert.doesNotMatch(
    androidIntegration,
    /new URL\("\.\.\/\.\.", import\.meta\.url\)\.pathname/,
  );
  assert.match(androidIntegration, /simdeck-bin\.exe/);
  assert.match(androidIntegration, /simdeck-bin-win32-x64\.exe/);
  assert.match(androidIntegration, /AppData", "Local", "Android", "Sdk/);
  assert.match(androidIntegration, /\.exe/);
});

test("iOS PR comment waits for public simulator list access", () => {
  const prebootIndex = iosAction.indexOf(
    "- name: Select and preboot simulator",
  );
  const readinessIndex = iosAction.indexOf(
    "- name: Wait for public SimDeck iOS session access",
  );
  const commentIndex = iosAction.indexOf(
    "- name: Update status comment with booted simulator URL",
  );

  assert.notEqual(prebootIndex, -1, "preboot step should exist");
  assert.notEqual(
    commentIndex,
    -1,
    "booted simulator comment step should exist",
  );
  assert(
    readinessIndex > prebootIndex,
    "readiness check should run after simulator preboot",
  );
  assert(
    readinessIndex < commentIndex,
    "readiness check should run before posting the PR URL",
  );

  const readinessStep = iosAction.slice(readinessIndex, commentIndex);
  assert.match(
    readinessStep,
    /\$\{\{ steps\.stream\.outputs\.url \}\}\/api\/simulators\?simdeckToken=/,
    "readiness check should use the public tunnel URL",
  );
  assert.match(
    readinessStep,
    /SIMULATOR_UDID/,
    "readiness check should look for the selected simulator",
  );
  assert.match(
    readinessStep,
    /isBooted/,
    "readiness check should require the selected simulator to be booted",
  );
});

for (const [platform, action, startStep, waitStep] of [
  [
    "iOS",
    iosAction,
    "Start simulator artifact download",
    "Wait for simulator artifact download",
  ],
  [
    "Android",
    androidAction,
    "Start APK artifact download",
    "Wait for APK artifact download",
  ],
]) {
  test(`${platform} PR comment resolves an actual matching artifact before download`, () => {
    const artifactStep = stepSlice(action, startStep, waitStep);

    assert.match(
      artifactStep,
      /artifact_candidates\+=\$'\\n'"\$\{ARTIFACT_PREFIX\}"/,
      "default artifact lookup should include prefix-only artifacts",
    );
    assert.match(
      artifactStep,
      /run\.get\("head_sha"\) == sha/,
      "repository artifact lookup should match the PR head SHA",
    );
    assert.match(
      artifactStep,
      /find_artifact_by_run/,
      "workflow-run fallback should inspect the run's artifacts",
    );
    assert.match(
      artifactStep,
      /--name "\$\{download_artifact_name\}"/,
      "download should use the artifact name that was actually found",
    );
    assert.doesNotMatch(
      artifactStep,
      /gh run download "\$\{run_id\}" --repo "\$\{REPO\}" --name "\$\{artifact_name\}"/,
      "workflow-run fallback must not assume the generated artifact name exists",
    );
  });

  test(`${platform} PR comment reports artifact startup failure explicitly`, () => {
    const waitStepBody = stepSlice(action, waitStep, "Install and launch");

    assert.match(
      waitStepBody,
      /SIMDECK_SESSION_START_FAILED=1/,
      "artifact failure should mark startup failure",
    );
    assert.match(
      waitStepBody,
      /session could not start for commit/,
      "artifact failure comment should not read like a completed session",
    );
    assert.match(
      waitStepBody,
      /No unexpired .* artifact was available/,
      "artifact failure comment should explain the missing or expired artifact",
    );
  });

  test(`${platform} PR comment only posts ended status after app launch`, () => {
    const launchIndex = indexOfStep(action, "Install and launch");
    const sessionOpenIndex = action.indexOf("SIMDECK_SESSION_OPEN=1");
    const finalStep = stepSlice(action, "Update status comment at end");

    assert(
      sessionOpenIndex > launchIndex,
      "session should only be marked open after the app is launched",
    );
    assert.match(
      finalStep,
      /if: always\(\) && env\.SIMDECK_SESSION_OPEN == '1'/,
      "ended status should only run for sessions that opened",
    );
  });

  test(`${platform} PR comment relies on the packaged CLI service supervisor`, () => {
    const startStepBody = stepSlice(
      action,
      "Install tools, start SimDeck and tunnel",
      "Resolve PR head",
    );

    assert.doesNotMatch(
      startStepBody,
      /simdeck-service-supervisor\.sh/,
      "action should not carry a second workflow-local service supervisor",
    );
    assert.match(startStepBody, /simdeck service run/);
    assert.match(startStepBody, /echo "\$!" > simdeck\.pid/);
  });

  test(`${platform} PR comment keepalive tolerates transient service restarts`, () => {
    const keepaliveStepBody = stepSlice(
      action,
      "Keep session alive",
      "Stop session",
    );

    assert.match(
      keepaliveStepBody,
      /SIMDECK_SERVICE_HEALTH_GRACE_SECONDS/,
      "keepalive should have a grace window for service restarts",
    );
    assert.match(
      keepaliveStepBody,
      /health_failure_started/,
      "keepalive should track continuous service health failures",
    );
    assert.match(
      keepaliveStepBody,
      /cat simdeck-service\.log/,
      "keepalive should print service logs when the grace window expires",
    );
    assert.match(
      keepaliveStepBody,
      /continue/,
      "keepalive should continue polling after transient service failures",
    );
  });

  test(`${platform} PR comment uses the public session URL output`, () => {
    assert.match(
      action,
      /session_password:/,
      "action should expose a session password input",
    );
    assert.match(
      action,
      /ci_proxy_url:/,
      "action should expose a CI proxy URL input",
    );
    assert.match(
      action,
      /default: https:\/\/ci\.simdeck\.sh/,
      "CI proxy links should default to the production SimDeck domain",
    );
    assert.match(
      action,
      /proxy_links:/,
      "action should expose a proxy_links input",
    );
    assert.match(
      action,
      /default: "true"/,
      "proxy_links should default to true",
    );
    assert.match(
      action,
      /public_url=/,
      "stream step should build a public URL output",
    );
    assert.match(
      action,
      /steps\.stream\.outputs\.public_url/,
      "PR comments should use the public URL output",
    );
    assert.match(
      action,
      /session_password requires proxy_links: true/,
      "password-protected sessions should not allow raw tunnel links",
    );
  });
}

darwinTest(
  "npm CLI wrapper restarts service run after recoverable native exit",
  () => {
    const root = mkdtempSync(join(tmpdir(), "simdeck-wrapper-test-"));
    try {
      mkdirSync(join(root, "bin"), { recursive: true });
      mkdirSync(join(root, "build"), { recursive: true });
      const wrapperPath = join(root, "bin", "simdeck.mjs");
      const nativePath = join(root, "build", "simdeck-bin");
      const logPath = join(root, "native.log");
      const countPath = join(root, "count");

      copyFileSync(
        new URL("../packages/cli/bin/simdeck.mjs", import.meta.url),
        wrapperPath,
      );
      chmodSync(wrapperPath, 0o755);
      writeFileSync(
        nativePath,
        `#!/usr/bin/env bash
set -euo pipefail
count="$(cat "${countPath}" 2>/dev/null || echo 0)"
count="$((count + 1))"
echo "$count" > "${countPath}"
echo "$$:\${SIMDECK_SERVICE_METADATA_PID:-}:\$*" >> "${logPath}"
if [[ "$count" == "1" ]]; then
  exit 75
fi
exit 0
`,
      );
      chmodSync(nativePath, 0o755);

      const result = spawnSync(
        process.execPath,
        [wrapperPath, "service", "run", "--port", "4310"],
        {
          encoding: "utf8",
        },
      );

      assert.equal(result.status, 0, result.stderr);
      const logLines = readFileSync(logPath, "utf8").trim().split("\n");
      assert.equal(logLines.length, 2, "service run should be retried once");

      const entries = logLines.map((line) => {
        const [pid, metadataPid, args] = line.split(":");
        return { pid, metadataPid, args };
      });
      assert.notEqual(entries[0].pid, entries[1].pid);
      assert.match(entries[0].metadataPid, /^\d+$/);
      assert.equal(entries[0].metadataPid, entries[1].metadataPid);
      assert.notEqual(entries[0].pid, entries[0].metadataPid);
      assert.equal(entries[0].args, "service run --port 4310");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  },
);

darwinTest(
  "npm CLI wrapper does not restart non-service commands on exit 75",
  () => {
    const root = mkdtempSync(join(tmpdir(), "simdeck-wrapper-test-"));
    try {
      mkdirSync(join(root, "bin"), { recursive: true });
      mkdirSync(join(root, "build"), { recursive: true });
      const wrapperPath = join(root, "bin", "simdeck.mjs");
      const nativePath = join(root, "build", "simdeck-bin");
      const logPath = join(root, "native.log");

      copyFileSync(
        new URL("../packages/cli/bin/simdeck.mjs", import.meta.url),
        wrapperPath,
      );
      chmodSync(wrapperPath, 0o755);
      writeFileSync(
        nativePath,
        `#!/usr/bin/env bash
set -euo pipefail
echo "$$:\${SIMDECK_SERVICE_METADATA_PID:-}:\$*" >> "${logPath}"
exit 75
`,
      );
      chmodSync(nativePath, 0o755);

      const result = spawnSync(process.execPath, [wrapperPath, "list"], {
        encoding: "utf8",
      });

      assert.equal(result.status, 75);
      const logLines = readFileSync(logPath, "utf8").trim().split("\n");
      assert.equal(logLines.length, 1);
      assert.equal(logLines[0].split(":")[1], "");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  },
);
