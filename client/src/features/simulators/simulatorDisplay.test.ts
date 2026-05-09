import { describe, expect, it } from "vitest";

import type { SimulatorMetadata } from "../../api/types";
import {
  shouldRenderNativeChrome,
  simulatorRuntimeLabel,
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

  it("keeps native chrome off for device families without supported bezels", () => {
    expect(
      shouldRenderNativeChrome(
        simulator({
          deviceTypeIdentifier:
            "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K",
          name: "Apple TV 4K (3rd generation)",
        }),
      ),
    ).toBe(false);
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
});
