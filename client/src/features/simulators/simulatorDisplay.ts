import type { SimulatorMetadata } from "../../api/types";

const RUNTIME_IDENTIFIER_PREFIX = "com.apple.CoreSimulator.SimRuntime.";
const RUNTIME_PLATFORMS = ["visionOS", "watchOS", "tvOS", "iOS", "xrOS"];

export function simulatorRuntimeLabel(simulator: SimulatorMetadata): string {
  return (
    formatRuntimeLabel(simulator.runtimeName) ??
    formatRuntimeLabel(simulator.runtimeIdentifier) ??
    ""
  );
}

export function shouldRenderNativeChrome(
  simulator: SimulatorMetadata,
): boolean {
  if (simulator.platform === "android-emulator") {
    return true;
  }
  const identifier = simulator.deviceTypeIdentifier ?? "";
  const name = simulator.name ?? "";
  const deviceTypeName = simulator.deviceTypeName ?? "";
  return (
    identifier.includes(".iPhone-") ||
    identifier.includes(".iPad-") ||
    identifier.includes(".Apple-Watch-") ||
    name.startsWith("iPhone") ||
    name.startsWith("iPad") ||
    name.startsWith("Apple Watch") ||
    deviceTypeName.startsWith("Apple Watch")
  );
}

function formatRuntimeLabel(value: string | undefined): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  if (!trimmed.startsWith(RUNTIME_IDENTIFIER_PREFIX)) {
    return trimmed;
  }

  const suffix = trimmed.slice(RUNTIME_IDENTIFIER_PREFIX.length);
  for (const platform of RUNTIME_PLATFORMS) {
    const prefix = `${platform}-`;
    if (suffix.startsWith(prefix)) {
      const version = suffix.slice(prefix.length).replaceAll("-", ".");
      return version ? `${platform} ${version}` : platform;
    }
  }
  return trimmed;
}
