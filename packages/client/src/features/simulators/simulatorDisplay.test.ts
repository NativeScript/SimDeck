import { describe, expect, it } from "vitest";

import type { SimulatorMetadata } from "../../api/types";
import {
  shouldRenderNativeChrome,
  simulatorHasFixedOrientation,
  simulatorRuntimeLabel,
  simulatorUsesInsetChromeButtons,
} from "./simulatorDisplay";

function simulator(
  metadata: Partial<SimulatorMetadata> = {},
): SimulatorMetadata {
  return {
    isBooted: false,
    name: "Test Simulator",
    udid: "UDID",
    ...metadata,
  };
}

describe("simulatorDisplay", () => {
  it("formats runtime identifiers", () => {
    expect(
      simulatorRuntimeLabel(
        simulator({
          runtimeIdentifier: "com.apple.CoreSimulator.SimRuntime.watchOS-26-0",
        }),
      ),
    ).toBe("watchOS 26.0");
  });

  it("ignores non-string runtime metadata", () => {
    expect(
      simulatorRuntimeLabel(
        simulator({
          runtimeIdentifier: { identifier: "unexpected" } as unknown as string,
          runtimeName: null as unknown as string,
        }),
      ),
    ).toBe("");
  });

  it("enables native chrome for Apple Watch simulators", () => {
    expect(
      shouldRenderNativeChrome(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Ultra-3-49mm",
          name: "Apple Watch Ultra 3 (49mm)",
        }),
      ),
    ).toBe(true);
  });

  it("enables native chrome for Apple TV simulators", () => {
    expect(
      shouldRenderNativeChrome(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K",
          name: "Apple TV 4K (3rd generation)",
        }),
      ),
    ).toBe(true);
  });

  it("keeps native chrome off for Android emulators", () => {
    expect(
      shouldRenderNativeChrome(
        simulator({
          deviceTypeIdentifier: "android-emulator",
          name: "SimDeck Pixel",
          platform: "android-emulator",
        }),
      ),
    ).toBe(false);
  });

  it("marks Apple TV and Apple Watch simulators as fixed-orientation devices", () => {
    expect(
      simulatorHasFixedOrientation(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K",
          runtimeIdentifier: "com.apple.CoreSimulator.SimRuntime.tvOS-26-0",
        }),
      ),
    ).toBe(true);
    expect(
      simulatorHasFixedOrientation(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Ultra-3-49mm",
          runtimeIdentifier: "com.apple.CoreSimulator.SimRuntime.watchOS-26-0",
        }),
      ),
    ).toBe(true);
    expect(
      simulatorHasFixedOrientation(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.iPhone-17",
          runtimeIdentifier: "com.apple.CoreSimulator.SimRuntime.iOS-26-0",
        }),
      ),
    ).toBe(false);
  });

  it("uses inset overlay chrome buttons only for iPhone and iPad simulators", () => {
    expect(
      simulatorUsesInsetChromeButtons(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.iPhone-17",
        }),
      ),
    ).toBe(true);
    expect(
      simulatorUsesInsetChromeButtons(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M4",
        }),
      ),
    ).toBe(true);
    expect(
      simulatorUsesInsetChromeButtons(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Ultra-3-49mm",
        }),
      ),
    ).toBe(false);
  });
});
