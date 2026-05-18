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
    return false;
  }
  const metadata = simulatorMetadataText(simulator);
  return (
    metadata.includes("iphone") ||
    metadata.includes("ipad") ||
    metadata.includes("apple-watch") ||
    metadata.includes("apple watch") ||
    metadata.includes("apple-tv") ||
    metadata.includes("apple tv") ||
    metadata.includes("appletv")
  );
}

export function simulatorHasFixedOrientation(
  simulator: SimulatorMetadata | null,
): boolean {
  if (!simulator || simulator.platform === "android-emulator") {
    return false;
  }
  const metadata = simulatorMetadataText(simulator);
  return (
    metadata.includes("tvos") ||
    metadata.includes("watchos") ||
    metadata.includes("apple-tv") ||
    metadata.includes("apple tv") ||
    metadata.includes("appletv") ||
    metadata.includes("apple-watch") ||
    metadata.includes("apple watch")
  );
}

function simulatorMetadataText(simulator: SimulatorMetadata): string {
  return [
    simulator.name,
    simulator.deviceTypeName,
    simulator.deviceTypeIdentifier,
    simulator.runtimeName,
    simulator.runtimeIdentifier,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
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
