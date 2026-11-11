import { describe, expect, it } from "vitest";

import {
  LIVE_VIDEO_UNAVAILABLE_MESSAGE,
  liveVideoUnavailableReason,
} from "./liveVideoCapability";

describe("liveVideoUnavailableReason", () => {
  it("treats a missing capability block as supported", () => {
    expect(liveVideoUnavailableReason(undefined)).toBe("");
    expect(liveVideoUnavailableReason(null)).toBe("");
  });

  it("returns nothing when the server supports live video", () => {
    expect(
      liveVideoUnavailableReason({ supported: true, requires: "macos" }),
    ).toBe("");
  });

  it("prefers the server-provided reason", () => {
    expect(
      liveVideoUnavailableReason({
        supported: false,
        requires: "macos",
        reason: "  Live H.264 video streaming requires macOS.  ",
      }),
    ).toBe("Live H.264 video streaming requires macOS.");
  });

  it("falls back to the client message when the reason is blank", () => {
    expect(
      liveVideoUnavailableReason({ supported: false, reason: "   " }),
    ).toBe(LIVE_VIDEO_UNAVAILABLE_MESSAGE);
    expect(liveVideoUnavailableReason({ supported: false })).toBe(
      LIVE_VIDEO_UNAVAILABLE_MESSAGE,
    );
  });
});
