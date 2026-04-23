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
