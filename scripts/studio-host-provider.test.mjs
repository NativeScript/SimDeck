import assert from "node:assert/strict";
import test from "node:test";

import {
  isWebSocketUpgradeRequest,
  parseArgs,
  redactConfig,
} from "./studio-host-provider.mjs";

test("parses long provider arguments", () => {
  assert.deepEqual(
    parseArgs([
      "--studio-url",
      "https://studio.example",
      "--host-id",
      "host-123",
      "--max-capacity",
      "3",
    ]),
    {
      "host-id": "host-123",
      "max-capacity": "3",
      "studio-url": "https://studio.example",
    },
  );
});

test("redacts provider token from status output", () => {
  assert.deepEqual(redactConfig({ hostToken: "secret", studioUrl: "x" }), {
    hostToken: "[redacted]",
    studioUrl: "x",
  });
});

test("detects websocket upgrade requests", () => {
  assert.equal(
    isWebSocketUpgradeRequest({
      headers: {
        connection: "keep-alive, Upgrade",
        upgrade: "websocket",
      },
    }),
    true,
  );
  assert.equal(
    isWebSocketUpgradeRequest({
      headers: {
        accept: "application/json",
      },
    }),
    false,
  );
});
