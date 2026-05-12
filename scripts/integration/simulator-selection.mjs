const defaultDeviceNames = [
  "iPhone 17",
  "iPhone 16",
  "iPhone 15",
  "iPhone 14",
  "iPhone 13",
  "iPhone SE (3rd generation)",
];

export function selectIntegrationSimulator({
  runJson,
  runText,
  timeoutMs,
  env = process.env,
}) {
  const sdkVersion = activeSimulatorSdkVersion(runText, timeoutMs);
  const runtimes = availableIosRuntimes(runJson, timeoutMs);
  const runtime = selectRuntime(runtimes, sdkVersion, env);
  const deviceType = selectDeviceType(
    runtime,
    availableDeviceTypes(runJson, timeoutMs),
    env.SIMDECK_INTEGRATION_DEVICE_TYPE,
  );
  return {
    runtime,
    deviceType,
    sdkVersion,
  };
}

function activeSimulatorSdkVersion(runText, timeoutMs) {
  return runText("xcrun", ["--sdk", "iphonesimulator", "--show-sdk-version"], {
    timeoutMs,
  }).trim();
}

function availableIosRuntimes(runJson, timeoutMs) {
  const payload = runJson("xcrun", ["simctl", "list", "runtimes", "-j"], {
    timeoutMs,
  });
  return payload.runtimes
    .filter(
      (runtime) => runtime.isAvailable && runtime.identifier?.includes("iOS"),
    )
    .sort(compareRuntimeVersions);
}

function availableDeviceTypes(runJson, timeoutMs) {
  return (
    runJson("xcrun", ["simctl", "list", "devicetypes", "-j"], {
      timeoutMs,
    }).devicetypes ?? []
  );
}

function selectRuntime(runtimes, sdkVersion, env) {
  if (runtimes.length === 0) {
    throw new Error("No available iOS simulator runtime found.");
  }

  const requestedRuntime = env.SIMDECK_INTEGRATION_IOS_RUNTIME;
  if (requestedRuntime) {
    const match = runtimes.find((runtime) =>
      runtimeMatches(runtime, requestedRuntime),
    );
    if (!match) {
      throw new Error(
        `No available iOS simulator runtime matched ${JSON.stringify(requestedRuntime)}.`,
      );
    }
    return match;
  }

  const sdkMajor = versionParts(sdkVersion)[0];
  const sameMajor = runtimes.filter(
    (runtime) => versionParts(runtime.version)[0] === sdkMajor,
  );
  const notNewerThanSdk = sameMajor.filter(
    (runtime) => compareVersionStrings(runtime.version, sdkVersion) <= 0,
  );
  return notNewerThanSdk.at(-1) ?? sameMajor.at(-1) ?? runtimes.at(-1);
}

function runtimeMatches(runtime, requestedRuntime) {
  const requested = requestedRuntime.trim();
  const normalizedRequested = normalizeIdentifier(requested);
  return [
    runtime.identifier,
    runtime.name,
    runtime.version,
    `iOS ${runtime.version}`,
    `com.apple.CoreSimulator.SimRuntime.iOS-${String(runtime.version).replaceAll(".", "-")}`,
  ].some((candidate) => normalizeIdentifier(candidate) === normalizedRequested);
}

function selectDeviceType(runtime, allDeviceTypes, requestedDeviceType) {
  const supported = supportedDeviceTypes(runtime, allDeviceTypes);
  const iphones = supported.filter(
    (device) =>
      device.productFamily === "iPhone" ||
      device.identifier?.includes("iPhone"),
  );

  if (requestedDeviceType) {
    const match = iphones.find((device) =>
      deviceTypeMatches(device, requestedDeviceType),
    );
    if (!match) {
      throw new Error(
        `Runtime ${runtime.identifier} does not support requested device ${JSON.stringify(requestedDeviceType)}.`,
      );
    }
    return match;
  }

  for (const name of defaultDeviceNames) {
    const match = iphones.find((device) => device.name === name);
    if (match) {
      return match;
    }
  }

  const fallback = iphones[0];
  if (!fallback) {
    throw new Error(
      `Runtime ${runtime.identifier} does not support an iPhone device.`,
    );
  }
  return fallback;
}

function supportedDeviceTypes(runtime, allDeviceTypes) {
  const runtimeSupported = Array.isArray(runtime.supportedDeviceTypes)
    ? runtime.supportedDeviceTypes
    : [];
  if (runtimeSupported.length > 0) {
    return runtimeSupported;
  }
  return allDeviceTypes.filter(
    (device) =>
      device.productFamily === "iPhone" ||
      device.identifier?.includes("iPhone"),
  );
}

function deviceTypeMatches(device, requestedDeviceType) {
  const requested = normalizeIdentifier(requestedDeviceType);
  return [device.identifier, device.name].some(
    (candidate) => normalizeIdentifier(candidate) === requested,
  );
}

function normalizeIdentifier(value) {
  return String(value ?? "")
    .trim()
    .toLowerCase();
}

function compareRuntimeVersions(left, right) {
  const delta = compareVersionStrings(left.version, right.version);
  if (delta !== 0) {
    return delta;
  }
  return String(left.identifier).localeCompare(String(right.identifier));
}

function compareVersionStrings(left, right) {
  const leftParts = versionParts(left);
  const rightParts = versionParts(right);
  for (
    let index = 0;
    index < Math.max(leftParts.length, rightParts.length);
    index += 1
  ) {
    const delta = (leftParts[index] ?? 0) - (rightParts[index] ?? 0);
    if (delta !== 0) {
      return delta;
    }
  }
  return 0;
}

function versionParts(version) {
  return String(version ?? "0")
    .split(".")
    .map((part) => Number(part) || 0);
}
