import assert from "node:assert/strict";
import test from "node:test";

import { normalizeStudioPublicUrlWithCloud } from "./studio-provider-bridge.mjs";

const cloudUrl = "https://simdeck.djdev.me";

test("normalizes relative studio expose paths against the cloud URL", () => {
  assert.equal(
    normalizeStudioPublicUrlWithCloud("/simulator/preview-123", cloudUrl),
    "https://simdeck.djdev.me/simulator/preview-123",
  );
});

test("collapses duplicated full cloud URL prefixes", () => {
  assert.equal(
    normalizeStudioPublicUrlWithCloud(
      "https://simdeck.djdev.mehttps://simdeck.djdev.me/simulator/preview-123",
      cloudUrl,
    ),
    "https://simdeck.djdev.me/simulator/preview-123",
  );
});

test("collapses duplicated cloud origins when base URL has a path", () => {
  assert.equal(
    normalizeStudioPublicUrlWithCloud(
      "https://simdeck.djdev.mehttps://simdeck.djdev.me/simulator/preview-123",
      "https://simdeck.djdev.me/actions",
    ),
    "https://simdeck.djdev.me/simulator/preview-123",
  );
});

test("preserves valid external tunnel URLs", () => {
  assert.equal(
    normalizeStudioPublicUrlWithCloud(
      "https://preview.example.test/simulator/preview-123",
      cloudUrl,
    ),
    "https://preview.example.test/simulator/preview-123",
  );
});
