import { describe, expect, it } from "vitest";

import {
  buildStreamTarget,
  initialStreamBackend,
  preferredStreamBackend,
} from "./streamWorkerClient";

describe("streamWorkerClient", () => {
  it("uses the common H264 WebSocket preference for Android emulator streams", () => {
    const target = buildStreamTarget("android:emulator-5554", {
      platform: "android-emulator",
      transport: "h264",
    });

    expect(preferredStreamBackend(target)).toBe("h264-ws");
  });

  it("uses the common WebRTC preference for Android emulator streams", () => {
    const target = buildStreamTarget("android:Pixel_8", {
      transport: "webrtc",
    });

    expect(preferredStreamBackend(target)).toBe("webrtc");
  });

  it("treats explicit RGBA transport as a WebRTC backend", () => {
    const previousWindow = globalThis.window;
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: { location: { search: "?stream=rgba" } },
    });

    try {
      expect(preferredStreamBackend(buildStreamTarget("android:Pixel_8"))).toBe(
        "webrtc",
      );
    } finally {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: previousWindow,
      });
    }
  });

  it("defaults Android auto streams to WebRTC when the browser supports it", () => {
    const previousPeerConnection = globalThis.RTCPeerConnection;
    (
      globalThis as unknown as { RTCPeerConnection: unknown }
    ).RTCPeerConnection = function RTCPeerConnection() {};
    const target = buildStreamTarget("android:Pixel_8", {
      transport: "auto",
    });

    try {
      expect(preferredStreamBackend(target)).toBe("auto");
      expect(initialStreamBackend(target)).toBe("webrtc");
    } finally {
      (
        globalThis as unknown as { RTCPeerConnection: unknown }
      ).RTCPeerConnection = previousPeerConnection;
    }
  });
});
