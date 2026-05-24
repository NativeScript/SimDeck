import { startSimDeckReactNativeInspector } from "./index";

declare const __DEV__: boolean | undefined;
declare const process: { env?: Record<string, string | undefined> } | undefined;

const defaultPorts = Array.from({ length: 11 }, (_, index) => 4310 + index);

if (typeof __DEV__ !== "undefined" && __DEV__) {
  const envPort = Number(process?.env?.EXPO_PUBLIC_SIMDECK_PORT);
  const sourceRoot = process?.env?.EXPO_PUBLIC_SIMDECK_SOURCE_ROOT;
  startSimDeckReactNativeInspector({
    ports:
      Number.isInteger(envPort) && envPort > 0
        ? [envPort, ...defaultPorts]
        : defaultPorts,
    sourceRoot,
  });
}
