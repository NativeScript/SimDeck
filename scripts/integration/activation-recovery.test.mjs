import test from "node:test";
import assert from "node:assert/strict";

import {
  activationRecoveryReason,
  isCoreSimulatorActivationTimeout,
  shouldRecycleSimulatorForFixtureLaunch,
} from "./activation-recovery.mjs";

test("recognizes simctl launch timeouts as CoreSimulator activation stalls", () => {
  const error = new Error(
    'SimDeck service returned HTTP 500: {"error":"xcrun simctl launch --stdout=/dev/null --stderr=/dev/null 5FAFF5E2 dev.nativescript.simdeck.integration.fixture timed out after 120s."}',
  );

  assert.equal(isCoreSimulatorActivationTimeout(error), true);
  assert.match(activationRecoveryReason({ launchError: error }), /launch/);
});

test("recognizes simctl openurl timeouts as CoreSimulator activation stalls", () => {
  const error = new Error(
    'SimDeck service returned HTTP 500: {"error":"xcrun simctl openurl 5FAFF5E2 simdeck-fixture://integration timed out after 90s."}',
  );

  assert.equal(isCoreSimulatorActivationTimeout(error), true);
  assert.match(activationRecoveryReason({ urlError: error }), /open-url/);
});

test("does not recycle for ordinary UI lookup misses", () => {
  const verifyError = new Error(
    'POST /api/simulators/5FAFF5E2/action returned 404: {"error":"No accessibility element matched."}',
  );

  assert.equal(isCoreSimulatorActivationTimeout(verifyError), false);
  assert.equal(
    shouldRecycleSimulatorForFixtureLaunch({
      launchError: null,
      verifyError,
      recoveryCount: 0,
      maxRecoveries: 1,
    }),
    false,
  );
});

test("recycles once for activation timeouts", () => {
  const launchError = new Error(
    'SimDeck service returned HTTP 500: {"error":"xcrun simctl launch 5FAFF5E2 dev.nativescript.simdeck.integration.fixture timed out after 120s."}',
  );

  assert.equal(
    shouldRecycleSimulatorForFixtureLaunch({
      launchError,
      recoveryCount: 0,
      maxRecoveries: 1,
    }),
    true,
  );
  assert.equal(
    shouldRecycleSimulatorForFixtureLaunch({
      launchError,
      recoveryCount: 1,
      maxRecoveries: 1,
    }),
    false,
  );
});
