import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const iosAction = readFileSync(
  new URL("../actions/run-ios-comment-session/action.yml", import.meta.url),
  "utf8",
);
const androidAction = readFileSync(
  new URL("../actions/run-android-comment-session/action.yml", import.meta.url),
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
      "default artifact lookup should include legacy prefix-only artifacts",
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

  test(`${platform} PR comment supervises recoverable daemon exits`, () => {
    const startStepBody = stepSlice(
      action,
      "Install tools, start SimDeck and tunnel",
      "Resolve PR head",
    );

    assert.match(
      startStepBody,
      /simdeck-daemon-supervisor\.sh/,
      "action should run SimDeck through a local supervisor",
    );
    assert.match(
      startStepBody,
      /"\$\{status\}" -eq 75/,
      "supervisor should restart recoverable SimDeck exits",
    );
    assert.match(
      startStepBody,
      /"\$\{status\}" -ge 128/,
      "supervisor should restart signal-terminated daemon children",
    );
    assert.match(
      startStepBody,
      /simdeck-child\.pid/,
      "supervisor should expose the active child pid for cleanup diagnostics",
    );
  });

  test(`${platform} PR comment keepalive tolerates transient daemon restarts`, () => {
    const keepaliveStepBody = stepSlice(
      action,
      "Keep session alive",
      "Stop session",
    );

    assert.match(
      keepaliveStepBody,
      /SIMDECK_DAEMON_HEALTH_GRACE_SECONDS/,
      "keepalive should have a grace window for daemon restarts",
    );
    assert.match(
      keepaliveStepBody,
      /health_failure_started/,
      "keepalive should track continuous daemon health failures",
    );
    assert.match(
      keepaliveStepBody,
      /cat simdeck-daemon\.log/,
      "keepalive should print daemon logs when the grace window expires",
    );
    assert.match(
      keepaliveStepBody,
      /continue/,
      "keepalive should continue polling after transient daemon failures",
    );
  });
}
