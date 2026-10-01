import { describe, expect, it } from "vitest";

import {
  buildStreamTarget,
  createWebRtcPeerConnection,
  initialStreamBackend,
  preferredStreamBackend,
} from "./streamWorkerClient";

describe("streamWorkerClient", () => {
  it("ignores removed legacy stream transport preferences", () => {
    const target = buildStreamTarget("android:emulator-5554", {
      platform: "android-emulator",
      transport: "h264" as never,
    });

    expect(preferredStreamBackend(target)).toBe("auto");
  });

  it("uses the common WebRTC preference for Android emulator streams", () => {
    const target = buildStreamTarget("android:Pixel_8", {
      transport: "webrtc",
    });

    expect(preferredStreamBackend(target)).toBe("webrtc");
  });

  it("ignores unknown stream query parameters", () => {
    const previousWindow = globalThis.window;
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: { location: { search: "?stream=unknown" } },
    });

    try {
      expect(preferredStreamBackend(buildStreamTarget("android:Pixel_8"))).toBe(
        "auto",
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

  it("creates exactly the requested connection without a disposable probe", () => {
    const previous = globalThis.RTCPeerConnection;
    const configuration = { iceServers: [] };
    let constructions = 0;
    class Peer {
      constructor(readonly config: RTCConfiguration) {
        constructions++;
      }
      close() {
        throw new Error("The live connection must not be closed by a probe");
      }
    }
    globalThis.RTCPeerConnection = Peer as unknown as typeof RTCPeerConnection;
    try {
      const peer = createWebRtcPeerConnection(configuration);
      expect(constructions).toBe(1);
      expect((peer as unknown as Peer).config).toBe(configuration);
    } finally {
      globalThis.RTCPeerConnection = previous;
    }
  });

  it("preserves the actual initialization error without a second attempt", () => {
    const previous = globalThis.RTCPeerConnection;
    const cause = new Error("Connection denied by browser policy");
    let constructions = 0;
    globalThis.RTCPeerConnection = class {
      constructor() {
        constructions++;
        throw cause;
      }
    } as unknown as typeof RTCPeerConnection;
    try {
      expect(() => createWebRtcPeerConnection({})).toThrow(
        "Connection denied by browser policy",
      );
      expect(constructions).toBe(1);
    } finally {
      globalThis.RTCPeerConnection = previous;
    }
  });
});
