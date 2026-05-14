import { describe, expect, it } from "vitest";

import {
  resolveDevToolsTargetSelection,
  withSafariAutoTarget,
  type DevToolsTarget,
} from "./DevToolsPanel";

function safariTarget(
  id: string,
  url: string,
  metadata: Partial<DevToolsTarget> = {},
): DevToolsTarget {
  return {
    appActive: true,
    appName: "Safari",
    frameUrl: `/inspector/${id}`,
    id,
    meta: url,
    pageActive: false,
    source: "Safari",
    title: url,
    url,
    ...metadata,
  };
}

describe("resolveDevToolsTargetSelection", () => {
  it("adds a Safari auto target that follows the active Safari page", () => {
    const inactive = safariTarget("webkit:old", "https://old.example/");
    const active = safariTarget("webkit:active", "https://active.example/", {
      pageActive: true,
    });

    const targets = withSafariAutoTarget([inactive, active]);

    expect(targets[0]).toMatchObject({
      frameUrl: active.frameUrl,
      id: "webkit:safari:auto",
      meta: active.url,
      pageActive: true,
      safariAuto: true,
      title: "Auto",
    });
  });

  it("uses the newest Safari page when no active Safari page is reported", () => {
    const newest = safariTarget("webkit:newest", "https://newest.example/", {
      pageId: 5,
    });
    const oldest = safariTarget("webkit:oldest", "https://oldest.example/", {
      pageId: 1,
    });

    const targets = withSafariAutoTarget([newest, oldest]);

    expect(targets[0]).toMatchObject({
      frameUrl: newest.frameUrl,
      id: "webkit:safari:auto",
      meta: newest.url,
      pageActive: false,
      safariAuto: true,
      title: "Auto",
    });
  });

  it("uses Safari auto when Safari is foreground and selection is automatic", () => {
    const inactive = safariTarget("webkit:old", "https://old.example/");
    const active = safariTarget("webkit:active", "https://active.example/", {
      pageActive: true,
    });
    const targets = withSafariAutoTarget([inactive, active]);

    expect(
      resolveDevToolsTargetSelection({
        currentForegroundKey: "com.apple.mobilesafari",
        currentTargetId: inactive.id,
        foregroundApp: {
          appName: "MobileSafari",
          bundleIdentifier: "com.apple.mobilesafari",
          processIdentifier: 123,
        },
        manualOverride: false,
        pendingForegroundApp: null,
        pendingForegroundKey: "",
        targets,
      }),
    ).toMatchObject({
      automaticTargetId: "webkit:safari:auto",
      targetId: "webkit:safari:auto",
    });
  });

  it("does not replace a manually selected target with Safari auto", () => {
    const manual = safariTarget("webkit:manual", "https://manual.example/");
    const auto = withSafariAutoTarget([
      manual,
      safariTarget("webkit:active", "https://active.example/", {
        pageActive: true,
      }),
    ]);

    expect(
      resolveDevToolsTargetSelection({
        currentForegroundKey: "com.apple.mobilesafari",
        currentTargetId: manual.id,
        foregroundApp: {
          appName: "MobileSafari",
          bundleIdentifier: "com.apple.mobilesafari",
          processIdentifier: 123,
        },
        manualOverride: true,
        pendingForegroundApp: null,
        pendingForegroundKey: "",
        targets: auto,
      }),
    ).toEqual({
      automaticTargetId: "",
      shouldClearPendingForeground: false,
      targetId: manual.id,
    });
  });

  it("keeps manual override from applying automatic selection after the target disappears", () => {
    const first = safariTarget("webkit:first", "https://first.example/");
    const active = safariTarget("webkit:active", "https://active.example/");

    expect(
      resolveDevToolsTargetSelection({
        currentForegroundKey: "",
        currentTargetId: "webkit:removed",
        foregroundApp: null,
        manualOverride: true,
        pendingForegroundApp: null,
        pendingForegroundKey: "",
        targets: [first, active],
      }),
    ).toEqual({
      automaticTargetId: "",
      shouldClearPendingForeground: false,
      targetId: first.id,
    });
  });
});
