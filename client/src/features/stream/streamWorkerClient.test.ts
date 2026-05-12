import { describe, expect, it } from "vitest";

import {
  buildStreamTarget,
  initialStreamBackend,
  preferredStreamBackend,
  shouldUseLocalAndroidRgbaWebRtc,
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

  it("uses RGBA WebRTC transport for local loopback Android streams", () => {
    const previousWindow = globalThis.window;
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: { location: { hostname: "127.0.0.1", search: "" } },
    });

    try {
      expect(
        shouldUseLocalAndroidRgbaWebRtc(
          buildStreamTarget("android:Pixel_8", {
            platform: "android-emulator",
            transport: "auto",
          }),
        ),
      ).toBe(true);
    } finally {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: previousWindow,
      });
    }
  });

  it("keeps Android RGBA disabled for h264 or remote streams", () => {
    const previousWindow = globalThis.window;
    const location = { hostname: "127.0.0.1", search: "?stream=h264" };
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: { location },
    });

    try {
      expect(
        shouldUseLocalAndroidRgbaWebRtc(
          buildStreamTarget("android:Pixel_8", {
            platform: "android-emulator",
            transport: "auto",
          }),
        ),
      ).toBe(false);
      location.search = "";
      expect(
        shouldUseLocalAndroidRgbaWebRtc(
          buildStreamTarget("android:Pixel_8", {
            platform: "android-emulator",
            remote: true,
            transport: "auto",
          }),
        ),
      ).toBe(false);
    } finally {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: previousWindow,
      });
    }
  });
});
