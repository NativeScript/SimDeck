import { describe, expect, it } from "vitest";

import {
  buildStreamTarget,
  initialStreamBackend,
  preferredStreamBackend,
} from "./streamWorkerClient";

describe("streamWorkerClient", () => {
  it("forces Android emulator streams onto WebRTC even when H264 is requested", () => {
    const target = buildStreamTarget("android:emulator-5554", {
      platform: "android-emulator",
      transport: "h264",
    });

    expect(preferredStreamBackend(target)).toBe("webrtc");
    expect(initialStreamBackend(target)).toBe("webrtc");
  });

  it("treats Android UDID prefixes as WebRTC-only stream targets", () => {
    const target = buildStreamTarget("android:Pixel_8", {
      transport: "h264",
    });

    expect(preferredStreamBackend(target)).toBe("webrtc");
    expect(initialStreamBackend(target)).toBe("webrtc");
  });
});
