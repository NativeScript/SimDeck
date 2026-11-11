import { describe, expect, it } from "vitest";

import {
  buildStreamTarget,
  initialStreamBackend,
  preferredStreamBackend,
  responseErrorMessage,
} from "./streamWorkerClient";

function fakeResponse(
  status: number,
  body: string,
  contentType?: string,
): Pick<Response, "headers" | "status" | "text"> {
  return {
    headers: new Headers(contentType ? { "content-type": contentType } : {}),
    status,
    text: () => Promise.resolve(body),
  };
}

describe("responseErrorMessage", () => {
  it("unwraps the server error field from JSON bodies", async () => {
    const message = await responseErrorMessage(
      fakeResponse(
        501,
        '{"error":"Live H.264 video streaming requires macOS."}',
        "application/json",
      ),
    );

    expect(message).toBe("Live H.264 video streaming requires macOS.");
  });

  it("unwraps JSON error bodies even without a content type", async () => {
    const message = await responseErrorMessage(
      fakeResponse(500, '{"error":"encoder failed"}'),
    );

    expect(message).toBe("encoder failed");
  });

  it("returns plain text bodies unchanged", async () => {
    expect(await responseErrorMessage(fakeResponse(502, "Bad gateway"))).toBe(
      "Bad gateway",
    );
  });

  it("falls back to the status when the body is empty or malformed", async () => {
    expect(await responseErrorMessage(fakeResponse(503, ""))).toBe(
      "Request failed with status 503",
    );
    expect(await responseErrorMessage(fakeResponse(500, "{not json"))).toBe(
      "{not json",
    );
    expect(
      await responseErrorMessage(
        fakeResponse(500, '{"error":""}', "application/json"),
      ),
    ).toBe('{"error":""}');
  });
});

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
});
