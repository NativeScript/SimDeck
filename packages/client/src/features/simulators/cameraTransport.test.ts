import { describe, expect, it } from "vitest";

import {
  cameraH264Codecs,
  cameraVideoConstraints,
  videoDevices,
} from "./cameraTransport";

describe("camera", () => {
  it("requests frame rate without changing browser camera geometry", () => {
    expect(cameraVideoConstraints("front-camera")).toEqual({
      deviceId: { exact: "front-camera" },
      frameRate: { ideal: 30, max: 30 },
    });
  });

  it("names cameras after permission reveals their labels", () => {
    expect(
      videoDevices([
        { deviceId: "audio", kind: "audioinput", label: "Mic" },
        { deviceId: "one", kind: "videoinput", label: "FaceTime Camera" },
        { deviceId: "two", kind: "videoinput", label: "" },
      ]),
    ).toEqual([
      { id: "one", name: "FaceTime Camera" },
      { id: "two", name: "Camera 2" },
    ]);
  });

  it("keeps only packetization-mode 1 H.264 and prefers Baseline", () => {
    const codecs = cameraH264Codecs([
      {
        clockRate: 90_000,
        mimeType: "video/VP8",
      },
      {
        clockRate: 90_000,
        mimeType: "video/H264",
        sdpFmtpLine: "packetization-mode=0;profile-level-id=42e01f",
      },
      {
        clockRate: 90_000,
        mimeType: "video/H264",
        sdpFmtpLine: "packetization-mode=1;profile-level-id=640c1f",
      },
      {
        clockRate: 90_000,
        mimeType: "video/H264",
        sdpFmtpLine: "packetization-mode=1;profile-level-id=42e01f",
      },
    ]);

    expect(codecs).toHaveLength(2);
    expect(codecs[0]?.sdpFmtpLine).toContain("profile-level-id=42");
    expect(codecs[1]?.sdpFmtpLine).toContain("profile-level-id=64");
  });
});
