import { describe, expect, it } from "vitest";

import {
  buildStreamTarget,
  initialStreamBackend,
  preferredStreamBackend,
} from "./streamWorkerClient";

describe("streamWorkerClient", () => {
  it("forces Android emulator streams onto the raw frame socket even when H264 is requested", () => {
    const target = buildStreamTarget("android:emulator-5554", {
      platform: "android-emulator",
      transport: "h264",
    });

    expect(preferredStreamBackend(target)).toBe("android-raw");
    expect(initialStreamBackend(target)).toBe("android-raw");
  });

  it("treats Android UDID prefixes as raw frame stream targets", () => {
    const target = buildStreamTarget("android:Pixel_8", {
      transport: "h264",
    });

    expect(preferredStreamBackend(target)).toBe("android-raw");
    expect(initialStreamBackend(target)).toBe("android-raw");
  });

  it("keeps remote Android streams on encoded WebRTC", () => {
    const target = buildStreamTarget("android:Pixel_8", {
      remote: true,
      transport: "h264",
    });

    expect(preferredStreamBackend(target)).toBe("webrtc");
    expect(initialStreamBackend(target)).toBe("webrtc");
  });
});
