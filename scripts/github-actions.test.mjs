import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const iosAction = readFileSync(
  new URL("../actions/run-ios-comment-session/action.yml", import.meta.url),
  "utf8",
);

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
