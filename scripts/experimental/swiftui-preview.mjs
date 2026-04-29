#!/usr/bin/env node
import fs from "node:fs";
import crypto from "node:crypto";
import path from "node:path";
import process from "node:process";
import net from "node:net";
import { spawnSync } from "node:child_process";

const DEFAULT_BUNDLE_ID = "dev.simdeck.PreviewHost";
const DEFAULT_MIN_IOS = "15.0";
const RELOAD_PORT_START = 47440;
const RELOAD_PORT_LIMIT = 16;
const RELOAD_PROTOCOL_PREFIX = "SIMDECK_PREVIEW_RELOAD ";
const RELOAD_BYTES_PROTOCOL_PREFIX = "SIMDECK_PREVIEW_RELOAD_B64 ";

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help || !args.file) {
    printUsage();
    process.exit(args.help ? 0 : 2);
  }

  const sourceFile = path.resolve(String(args.file));
  const udid = String(args.udid ?? findBootedSimulatorUDID());
  if (!udid) {
    throw new Error(
      "No simulator UDID supplied and no booted simulator was found.",
    );
  }

  const buildRoot = path.resolve(
    String(args.buildRoot ?? path.join(".simdeck-preview", "build")),
  );
  const bundleId = String(args.bundleId ?? DEFAULT_BUNDLE_ID);
  const minIos = String(args.minIos ?? DEFAULT_MIN_IOS);
  const targetArch = String(
    args.arch ?? (process.arch === "arm64" ? "arm64" : "x86_64"),
  );
  const sdkPath = runText("xcrun", [
    "--sdk",
    "iphonesimulator",
    "--show-sdk-path",
  ]).trim();
  const context = {
    buildRoot,
    bundleId,
    minIos,
    sdkPath,
    target: `${targetArch}-apple-ios${minIos}-simulator`,
    udid,
  };
  fs.mkdirSync(buildRoot, { recursive: true });
  const xcode = resolveXcodeContext(args, context);

  const host = buildHostApp(context, Boolean(args.rebuildHost));
  const shouldInstallHost =
    host.rebuilt || Boolean(xcode && !args.skipXcodeBuild);
  if (xcode && shouldInstallHost) {
    overlayXcodeAppBundle(host.appPath, xcode);
  }
  installAndLaunchHost(context, host.appPath, shouldInstallHost);
  await reloadPreview(context, sourceFile, args, xcode);

  if (args.watch) {
    console.log(`[simdeck-preview] watching ${sourceFile}`);
    watchFiles(
      [
        sourceFile,
        ...arrayArg(args.extraSwift).map((item) => path.resolve(item)),
      ],
      () => {
        reloadPreview(context, sourceFile, args, xcode).catch((error) => {
          console.error(`[simdeck-preview] reload failed: ${error.message}`);
        });
      },
    );
    process.stdin.resume();
  }
}

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help" || arg === "-h") {
      args.help = true;
    } else if (arg === "--watch" || arg === "-w") {
      args.watch = true;
    } else if (arg === "--rebuild-host") {
      args.rebuildHost = true;
    } else if (arg === "--skip-xcode-build") {
      args.skipXcodeBuild = true;
    } else if (arg === "--skip-codesign") {
      args.skipCodesign = true;
    } else if (arg === "--profile") {
      args.profile = true;
    } else if (arg === "--split-compile") {
      args.splitCompile = true;
    } else if (arg.startsWith("--")) {
      const key = arg
        .slice(2)
        .replace(/-([a-z])/g, (_, char) => char.toUpperCase());
      const next = argv[index + 1];
      if (!next || next.startsWith("--")) {
        throw new Error(`Missing value for ${arg}.`);
      }
      index += 1;
      if (key === "extraSwift" || key === "swiftcArg") {
        args[key] = [...arrayArg(args[key]), next];
      } else {
        args[key] = next;
      }
    } else if (!args.file) {
      args.file = arg;
    } else {
      args.extraSwift = [...arrayArg(args.extraSwift), arg];
    }
  }
  return args;
}

function printUsage() {
  console.log(`Usage:
  node scripts/experimental/swiftui-preview.mjs --udid <sim> --file <Preview.swift> [options]

Options:
  --preview <name-or-index>  Select a #Preview block. Defaults to the first one.
  --watch, -w                Recompile and dlopen a new payload after file changes.
  --extra-swift <file>       Include another Swift file. Can be repeated.
  --swiftc-arg <arg>         Pass an extra argument to swiftc. Can be repeated.
  --workspace <path>         Use an Xcode workspace for compatibility mode.
  --project <path>           Use an Xcode project for compatibility mode.
  --scheme <name>            Xcode scheme to build for compatibility mode.
  --configuration <name>     Xcode configuration. Default: Debug.
  --derived-data-path <path> DerivedData path for Xcode builds.
  --skip-xcode-build         Reuse existing Xcode build artifacts.
  --skip-codesign            Do not ad-hoc sign reload dylibs.
  --profile                  Print reload-stage timings.
  --split-compile            Cache the preview source as a testable Swift module.
  --bundle-id <id>           Host bundle id. Default: ${DEFAULT_BUNDLE_ID}
  --build-root <path>        Build/cache directory. Default: .simdeck-preview/build
  --rebuild-host             Rebuild and reinstall the stable host app.

This is intentionally experimental. It extracts simple #Preview { ... } bodies,
builds them into a versioned simulator dylib, and asks a tiny host app to dlopen
the new dylib without reinstalling the host.

When --workspace/--project and --scheme are supplied, the runner builds the real
app target once, copies its resources/frameworks into the host, and links reload
dylibs against the target's Xcode-built debug dylib. That is the faster Xcode-ish
path: reloads compile the edited preview source plus a tiny wrapper, not the full
app target.`);
}

function buildHostApp(context, rebuildHost) {
  const appPath = path.join(context.buildRoot, "SimDeckPreviewHost.app");
  const executable = "SimDeckPreviewHost";
  const executablePath = path.join(appPath, executable);
  if (!rebuildHost && fs.existsSync(executablePath)) {
    return { appPath, rebuilt: false };
  }

  fs.rmSync(appPath, { recursive: true, force: true });
  fs.mkdirSync(appPath, { recursive: true });
  fs.writeFileSync(
    path.join(appPath, "Info.plist"),
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>${executable}</string>
  <key>CFBundleIdentifier</key><string>${context.bundleId}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>SimDeckPreview</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>MinimumOSVersion</key><string>${context.minIos}</string>
  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>SimDeckPreview</string>
      <key>CFBundleURLSchemes</key><array><string>simdeck-preview</string></array>
    </dict>
  </array>
</dict>
</plist>
`,
  );

  const mainPath = path.join(context.buildRoot, "SimDeckPreviewHost.swift");
  fs.writeFileSync(mainPath, hostSource());
  run("xcrun", [
    "--sdk",
    "iphonesimulator",
    "swiftc",
    "-target",
    context.target,
    "-sdk",
    context.sdkPath,
    "-parse-as-library",
    "-Onone",
    "-framework",
    "SwiftUI",
    "-framework",
    "UIKit",
    mainPath,
    "-o",
    executablePath,
  ]);
  console.log(`[simdeck-preview] built host ${appPath}`);
  return { appPath, rebuilt: true };
}

function installAndLaunchHost(context, appPath, shouldInstall) {
  if (!shouldInstall) {
    return;
  }
  run("xcrun", ["simctl", "install", context.udid, appPath]);
  run("xcrun", ["simctl", "launch", context.udid, context.bundleId], {
    allowFailure: true,
  });
}

async function reloadPreview(context, sourceFile, args, xcode) {
  const started = Date.now();
  const timings = [];
  const timed = (label, action) => {
    const stageStarted = Date.now();
    const result = action();
    timings.push([label, Date.now() - stageStarted]);
    return result;
  };
  const timedAsync = async (label, action) => {
    const stageStarted = Date.now();
    const result = await action();
    timings.push([label, Date.now() - stageStarted]);
    return result;
  };
  const source = timed("read source", () =>
    fs.readFileSync(sourceFile, "utf8"),
  );
  const previews = timed("extract previews", () => extractPreviews(source));
  if (previews.length === 0) {
    throw new Error(`No #Preview blocks found in ${sourceFile}.`);
  }
  const preview = timed("select preview", () =>
    selectPreview(previews, args.preview),
  );
  const stamp = `${Date.now()}-${process.pid}`;
  const payloadDir = path.join(context.buildRoot, "payloads", stamp);
  timed("prepare payload dir", () =>
    fs.mkdirSync(payloadDir, { recursive: true }),
  );

  const sanitizedPath = path.join(payloadDir, "PreviewSource.swift");
  const wrapperPath = path.join(payloadDir, "SimDeckPreviewPayload.swift");
  const dylibPath = path.join(
    payloadDir,
    `SimDeckPreviewPayload-${stamp}.dylib`,
  );
  const sanitizedSource = timed("sanitize source", () =>
    sanitizePreviewSource(
      removeRanges(
        source,
        previews.map((item) => item.range),
      ),
      xcode?.moduleName,
    ),
  );
  const sourceModule = args.splitCompile
    ? timed("resolve source cache", () =>
        previewSourceModule(context, sanitizedSource, args, xcode),
      )
    : null;
  timed("write generated swift", () => {
    fs.writeFileSync(sanitizedPath, sanitizedSource);
    fs.writeFileSync(
      wrapperPath,
      payloadWrapperSource(
        preview.body,
        xcode?.moduleName,
        sourceModule?.moduleName,
      ),
    );
  });

  if (sourceModule) {
    timed("swiftc source module", () =>
      compilePreviewSourceModule(
        context,
        sanitizedPath,
        sourceModule,
        args,
        xcode,
      ),
    );
  }

  const compileArgs = sourceModule
    ? splitWrapperCompileArgs(
        context,
        stamp,
        wrapperPath,
        dylibPath,
        sourceModule,
        args,
        xcode,
        sourceFile,
      )
    : monolithicCompileArgs(
        context,
        stamp,
        sanitizedPath,
        wrapperPath,
        dylibPath,
        args,
        xcode,
        sourceFile,
      );
  timed("swiftc emit dylib", () => run("xcrun", compileArgs));
  if (!args.skipCodesign) {
    timed("codesign", () =>
      run("codesign", ["-s", "-", "-f", dylibPath], { allowFailure: true }),
    );
  }

  const sentBytes = await timedAsync("tcp reload bytes", () =>
    sendTcpReloadBytes(dylibPath),
  );
  if (
    !sentBytes &&
    !(await timedAsync("tcp reload path", () =>
      sendTcpReloadPath(context, dylibPath),
    ))
  ) {
    const containerPath = timed("simctl get container", () =>
      appContainerPath(context),
    );
    const documentsDir = path.join(containerPath, "Documents");
    fs.mkdirSync(documentsDir, { recursive: true });
    const installedDylibPath = path.join(
      documentsDir,
      path.basename(dylibPath),
    );
    timed("copy dylib", () => fs.copyFileSync(dylibPath, installedDylibPath));
    const reloadUrl = `simdeck-preview://reload?path=${encodeURIComponent(installedDylibPath)}`;
    timed("simctl openurl", () =>
      run("xcrun", ["simctl", "openurl", context.udid, reloadUrl]),
    );
  }

  const elapsed = Date.now() - started;
  const label = preview.name ? `"${preview.name}"` : `#${preview.index + 1}`;
  console.log(`[simdeck-preview] reloaded preview ${label} in ${elapsed}ms`);
  console.log(
    `[simdeck-preview] SimDeck UI should show bundle ${context.bundleId} on ${context.udid}`,
  );
  if (args.profile) {
    const summary = timings.map(([label, ms]) => `${label}=${ms}ms`).join("  ");
    console.log(`[simdeck-preview] profile ${summary}`);
  }
}

function monolithicCompileArgs(
  context,
  stamp,
  sanitizedPath,
  wrapperPath,
  dylibPath,
  args,
  xcode,
  sourceFile,
) {
  return [
    "--sdk",
    "iphonesimulator",
    "swiftc",
    "-target",
    context.target,
    "-sdk",
    context.sdkPath,
    "-parse-as-library",
    "-Onone",
    "-emit-library",
    "-module-name",
    `SimDeckPreviewPayload_${stamp.replaceAll("-", "_")}`,
    "-framework",
    "SwiftUI",
    "-framework",
    "UIKit",
    ...xcodeSwiftcArgs(xcode),
    sanitizedPath,
    ...arrayArg(args.extraSwift).map((item) => path.resolve(item)),
    wrapperPath,
    ...xcodeObjectFiles(xcode, sourceFile),
    ...arrayArg(args.swiftcArg),
    "-o",
    dylibPath,
  ];
}

function splitWrapperCompileArgs(
  context,
  stamp,
  wrapperPath,
  dylibPath,
  sourceModule,
  args,
  xcode,
  sourceFile,
) {
  return [
    "--sdk",
    "iphonesimulator",
    "swiftc",
    "-target",
    context.target,
    "-sdk",
    context.sdkPath,
    "-parse-as-library",
    "-Onone",
    "-emit-library",
    "-module-name",
    `SimDeckPreviewPayload_${stamp.replaceAll("-", "_")}`,
    "-I",
    sourceModule.directory,
    "-framework",
    "SwiftUI",
    "-framework",
    "UIKit",
    ...xcodeSwiftcArgs(xcode),
    wrapperPath,
    sourceModule.objectPath,
    ...xcodeObjectFiles(xcode, sourceFile),
    ...arrayArg(args.swiftcArg),
    "-o",
    dylibPath,
  ];
}

function previewSourceModule(context, sanitizedSource, args, xcode) {
  const extraSwift = arrayArg(args.extraSwift);
  if (extraSwift.length > 0) {
    console.warn(
      "[simdeck-preview] warning: --split-compile currently falls back when --extra-swift is used.",
    );
    return null;
  }
  const swiftcArgs = [...xcodeSwiftcArgs(xcode), ...arrayArg(args.swiftcArg)];
  const hash = crypto
    .createHash("sha256")
    .update(context.target)
    .update("\0")
    .update(context.sdkPath)
    .update("\0")
    .update(xcode?.moduleName ?? "")
    .update("\0")
    .update(swiftcArgs.join("\0"))
    .update("\0")
    .update(sanitizedSource)
    .digest("hex")
    .slice(0, 16);
  const directory = path.join(context.buildRoot, "source-cache", hash);
  return {
    directory,
    moduleName: `SimDeckPreviewSource_${hash}`,
    modulePath: path.join(
      directory,
      `SimDeckPreviewSource_${hash}.swiftmodule`,
    ),
    objectPath: path.join(directory, `SimDeckPreviewSource_${hash}.o`),
  };
}

function compilePreviewSourceModule(
  context,
  sanitizedPath,
  sourceModule,
  args,
  xcode,
) {
  if (
    fs.existsSync(sourceModule.objectPath) &&
    fs.existsSync(sourceModule.modulePath)
  ) {
    return;
  }
  fs.mkdirSync(sourceModule.directory, { recursive: true });
  run("xcrun", [
    "--sdk",
    "iphonesimulator",
    "swiftc",
    "-target",
    context.target,
    "-sdk",
    context.sdkPath,
    "-parse-as-library",
    "-Onone",
    "-enable-testing",
    "-emit-module",
    "-emit-module-path",
    sourceModule.modulePath,
    "-emit-object",
    "-module-name",
    sourceModule.moduleName,
    "-framework",
    "SwiftUI",
    "-framework",
    "UIKit",
    ...xcodeSwiftcArgs(xcode),
    sanitizedPath,
    ...arrayArg(args.swiftcArg),
    "-o",
    sourceModule.objectPath,
  ]);
}

async function sendTcpReloadPath(context, dylibPath) {
  const containerPath = appContainerPath(context);
  const documentsDir = path.join(containerPath, "Documents");
  fs.mkdirSync(documentsDir, { recursive: true });
  const installedDylibPath = path.join(documentsDir, path.basename(dylibPath));
  fs.copyFileSync(dylibPath, installedDylibPath);
  for (let offset = 0; offset < RELOAD_PORT_LIMIT; offset += 1) {
    const port = RELOAD_PORT_START + offset;
    if (await sendTcpReloadToPort(port, installedDylibPath)) {
      return true;
    }
  }
  return false;
}

async function sendTcpReloadBytes(dylibPath) {
  const base64 = fs.readFileSync(dylibPath).toString("base64");
  const payload = `${RELOAD_BYTES_PROTOCOL_PREFIX}${path.basename(dylibPath)} ${base64}\n`;
  for (let offset = 0; offset < RELOAD_PORT_LIMIT; offset += 1) {
    const port = RELOAD_PORT_START + offset;
    if (await sendTcpMessageToPort(port, payload)) {
      return true;
    }
  }
  return false;
}

function sendTcpReloadToPort(port, dylibPath) {
  return sendTcpMessageToPort(port, `${RELOAD_PROTOCOL_PREFIX}${dylibPath}\n`);
}

function sendTcpMessageToPort(port, message) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    let settled = false;
    let response = "";
    const settle = (value) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      resolve(value);
    };
    const timer = setTimeout(() => {
      settle(false);
    }, 500);
    socket.once("connect", () => {
      socket.end(message);
    });
    socket.on("data", (data) => {
      response += data.toString("utf8");
      if (response.trim().startsWith("OK")) {
        settle(true);
      } else if (response.trim().startsWith("ERROR")) {
        settle(false);
      }
    });
    socket.once("error", () => {
      settle(false);
    });
    socket.once("close", () => {
      settle(response.trim().startsWith("OK"));
    });
  });
}

function appContainerPath(context) {
  if (context.containerPath) {
    return context.containerPath;
  }
  context.containerPath = runText("xcrun", [
    "simctl",
    "get_app_container",
    context.udid,
    context.bundleId,
    "data",
  ]).trim();
  return context.containerPath;
}

function resolveXcodeContext(args, context) {
  if (!args.workspace && !args.project && !args.scheme) {
    return null;
  }
  if (!args.scheme) {
    throw new Error(
      "--scheme is required when using --workspace or --project.",
    );
  }

  const derivedDataPath = path.resolve(
    String(args.derivedDataPath ?? path.join(context.buildRoot, "DerivedData")),
  );
  const cacheIdentity = xcodeCacheIdentity(args, derivedDataPath);
  if (args.skipXcodeBuild) {
    const cached = readCachedXcodeContext(context, cacheIdentity);
    if (cached) {
      return cached;
    }
  }
  const buildArgs = xcodebuildBaseArgs(args, derivedDataPath);
  if (!args.skipXcodeBuild) {
    const started = Date.now();
    run("xcodebuild", [
      ...buildArgs,
      "build",
      "CODE_SIGNING_ALLOWED=NO",
      "ENABLE_TESTABILITY=YES",
    ]);
    console.log(
      `[simdeck-preview] xcodebuild warm build completed in ${Date.now() - started}ms`,
    );
  }

  const settings = readXcodeBuildSettings(buildArgs, args.target);
  const buildSettings = settings.buildSettings ?? settings;
  const arch = context.target.split("-")[0];
  const appPath = findBuiltAppPath(buildSettings);
  const debugDylibPath = findXcodeDebugDylib(appPath, buildSettings);
  const xcode = {
    appPath,
    arch,
    buildSettings,
    debugDylibPath,
    moduleName:
      buildSettings.PRODUCT_MODULE_NAME || buildSettings.SWIFT_MODULE_NAME,
    objectFiles: debugDylibPath
      ? []
      : collectXcodeObjectFiles(buildSettings, arch),
  };
  writeCachedXcodeContext(context, cacheIdentity, xcode);
  return xcode;
}

function xcodeContextCachePath(context) {
  return path.join(context.buildRoot, "xcode-context-cache.json");
}

function xcodeCacheIdentity(args, derivedDataPath) {
  return {
    configuration: String(args.configuration ?? "Debug"),
    derivedDataPath,
    destination: String(args.destination ?? "generic/platform=iOS Simulator"),
    project: args.project ? path.resolve(String(args.project)) : "",
    scheme: String(args.scheme),
    target: String(args.target ?? ""),
    workspace: args.workspace ? path.resolve(String(args.workspace)) : "",
  };
}

function readCachedXcodeContext(context, identity) {
  const cachePath = xcodeContextCachePath(context);
  if (!fs.existsSync(cachePath)) {
    return null;
  }
  try {
    const cached = JSON.parse(fs.readFileSync(cachePath, "utf8"));
    if (JSON.stringify(cached.identity) !== JSON.stringify(identity)) {
      return null;
    }
    const xcode = cached.xcode;
    if (!xcode?.appPath || !fs.existsSync(xcode.appPath)) {
      return null;
    }
    if (xcode.debugDylibPath && !fs.existsSync(xcode.debugDylibPath)) {
      return null;
    }
    return xcode;
  } catch {
    return null;
  }
}

function writeCachedXcodeContext(context, identity, xcode) {
  fs.writeFileSync(
    xcodeContextCachePath(context),
    JSON.stringify({ identity, xcode }, null, 2),
  );
}

function xcodebuildBaseArgs(args, derivedDataPath) {
  const buildArgs = [];
  if (args.workspace) {
    buildArgs.push("-workspace", path.resolve(String(args.workspace)));
  } else if (args.project) {
    buildArgs.push("-project", path.resolve(String(args.project)));
  }
  buildArgs.push("-scheme", String(args.scheme));
  buildArgs.push("-configuration", String(args.configuration ?? "Debug"));
  buildArgs.push(
    "-destination",
    String(args.destination ?? "generic/platform=iOS Simulator"),
  );
  buildArgs.push("-derivedDataPath", derivedDataPath);
  return buildArgs;
}

function readXcodeBuildSettings(buildArgs, targetName) {
  const output = runText("xcodebuild", [
    ...buildArgs,
    "-showBuildSettings",
    "-json",
  ]);
  const settings = JSON.parse(output);
  if (!Array.isArray(settings) || settings.length === 0) {
    throw new Error(
      "xcodebuild -showBuildSettings returned no target settings.",
    );
  }
  if (targetName) {
    const match = settings.find((item) => item.target === targetName);
    if (match) {
      return match;
    }
  }
  return (
    settings.find((item) => item.buildSettings?.PRODUCT_MODULE_NAME) ??
    settings[0]
  );
}

function findBuiltAppPath(settings) {
  const wrapperName = settings.WRAPPER_NAME;
  const candidates = [
    settings.TARGET_BUILD_DIR && wrapperName
      ? path.join(settings.TARGET_BUILD_DIR, wrapperName)
      : "",
    settings.CODESIGNING_FOLDER_PATH,
    settings.BUILT_PRODUCTS_DIR && wrapperName
      ? path.join(settings.BUILT_PRODUCTS_DIR, wrapperName)
      : "",
  ].filter(Boolean);
  const appPath = candidates.find(
    (candidate) => candidate.endsWith(".app") && fs.existsSync(candidate),
  );
  if (!appPath) {
    throw new Error(
      `Unable to locate built .app for Xcode target. Tried: ${candidates.join(", ")}`,
    );
  }
  return appPath;
}

function findXcodeDebugDylib(appPath, settings) {
  const moduleName = settings.PRODUCT_MODULE_NAME || settings.SWIFT_MODULE_NAME;
  const candidates = [
    moduleName ? path.join(appPath, `${moduleName}.debug.dylib`) : "",
    ...fs
      .readdirSync(appPath)
      .filter((entry) => entry.endsWith(".debug.dylib"))
      .map((entry) => path.join(appPath, entry)),
  ].filter(Boolean);
  return candidates.find((candidate) => fs.existsSync(candidate)) ?? "";
}

function overlayXcodeAppBundle(hostAppPath, xcode) {
  const sourceApp = xcode.appPath;
  for (const entry of fs.readdirSync(sourceApp, { withFileTypes: true })) {
    if (
      entry.name === "Info.plist" ||
      entry.name === "_CodeSignature" ||
      entry.name === "PkgInfo" ||
      entry.name.endsWith(".app") ||
      entry.name === path.basename(sourceApp, ".app")
    ) {
      continue;
    }
    copyRecursive(
      path.join(sourceApp, entry.name),
      path.join(hostAppPath, entry.name),
    );
  }
  console.log(
    `[simdeck-preview] overlaid resources/frameworks from ${sourceApp}`,
  );
}

function copyRecursive(source, destination) {
  fs.rmSync(destination, { recursive: true, force: true });
  fs.cpSync(source, destination, { recursive: true, dereference: false });
}

function xcodeSwiftcArgs(xcode) {
  if (!xcode) {
    return [];
  }
  const settings = xcode.buildSettings;
  const args = [];
  pushSearchArgs(args, "-I", [
    settings.SWIFT_INCLUDE_PATHS,
    settings.BUILT_PRODUCTS_DIR,
    settings.TARGET_BUILD_DIR,
    settings.CONFIGURATION_BUILD_DIR,
  ]);
  pushSearchArgs(args, "-F", [
    settings.FRAMEWORK_SEARCH_PATHS,
    settings.BUILT_PRODUCTS_DIR,
    settings.TARGET_BUILD_DIR,
    settings.CONFIGURATION_BUILD_DIR,
  ]);
  pushSearchArgs(args, "-L", [
    settings.LIBRARY_SEARCH_PATHS,
    settings.BUILT_PRODUCTS_DIR,
    settings.TARGET_BUILD_DIR,
    settings.CONFIGURATION_BUILD_DIR,
  ]);
  for (const condition of splitBuildSetting(
    settings.SWIFT_ACTIVE_COMPILATION_CONDITIONS,
  )) {
    if (condition && condition !== "$(inherited)") {
      args.push("-D", condition);
    }
  }
  const bridgingHeader = settings.SWIFT_OBJC_BRIDGING_HEADER;
  if (bridgingHeader && fs.existsSync(bridgingHeader)) {
    args.push("-import-objc-header", bridgingHeader);
  }
  const moduleCache =
    settings.CLANG_MODULE_CACHE_PATH || settings.MODULE_CACHE_DIR;
  if (moduleCache) {
    args.push("-module-cache-path", moduleCache);
  }
  args.push(...splitBuildSetting(settings.OTHER_SWIFT_FLAGS));
  if (xcode.debugDylibPath) {
    args.push(
      "-Xlinker",
      "-rpath",
      "-Xlinker",
      "@executable_path",
      "-Xlinker",
      "-rpath",
      "-Xlinker",
      "@executable_path/Frameworks",
    );
  }
  return args.filter((item) => item !== "$(inherited)");
}

function xcodeObjectFiles(xcode, editedSourceFile) {
  if (!xcode) {
    return [];
  }
  if (xcode.debugDylibPath) {
    return [xcode.debugDylibPath];
  }
  const editedBaseName = path.basename(
    editedSourceFile,
    path.extname(editedSourceFile),
  );
  return xcode.objectFiles.filter((file) => {
    const name = path.basename(file, ".o");
    return name !== editedBaseName && !name.startsWith(`${editedBaseName}.`);
  });
}

function collectXcodeObjectFiles(settings, arch) {
  const roots = [
    settings.OBJECT_FILE_DIR_normal,
    settings.TARGET_TEMP_DIR,
    settings.OBJECT_FILE_DIR,
  ].filter(Boolean);
  const files = new Set();
  for (const root of roots) {
    if (fs.existsSync(root)) {
      for (const file of walkFiles(root)) {
        if (file.endsWith(".o") && objectFileMatchesArchPath(file, arch)) {
          files.add(file);
        }
      }
    }
  }
  const list = [...files].filter(
    (file) => !file.includes("SimDeckPreviewPayload"),
  );
  if (list.length === 0) {
    console.warn(
      "[simdeck-preview] warning: no Xcode object files found; compatibility will be limited.",
    );
  } else {
    console.log(
      `[simdeck-preview] found ${list.length} Xcode object files for fallback linking`,
    );
  }
  return list;
}

function objectFileMatchesArchPath(file, arch) {
  const normalized = file.split(path.sep).join("/");
  return (
    normalized.includes(`/Objects-normal/${arch}/`) ||
    !normalized.includes("/Objects-normal/")
  );
}

function* walkFiles(root) {
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const fullPath = path.join(root, entry.name);
    if (entry.isDirectory()) {
      yield* walkFiles(fullPath);
    } else if (entry.isFile()) {
      yield fullPath;
    }
  }
}

function pushSearchArgs(args, flag, values) {
  for (const value of values) {
    for (const item of splitBuildSetting(value)) {
      if (item && item !== "$(inherited)" && fs.existsSync(item)) {
        args.push(flag, item);
      }
    }
  }
}

function findBootedSimulatorUDID() {
  const data = JSON.parse(
    runText("xcrun", ["simctl", "list", "devices", "booted", "-j"]),
  );
  for (const runtimes of Object.values(data.devices ?? {})) {
    for (const device of runtimes) {
      if (device.state === "Booted") {
        return device.udid;
      }
    }
  }
  return "";
}

function extractPreviews(source) {
  const previews = [];
  let searchFrom = 0;
  while (true) {
    const macroIndex = source.indexOf("#Preview", searchFrom);
    if (macroIndex < 0) {
      break;
    }
    let cursor = macroIndex + "#Preview".length;
    cursor = skipWhitespace(source, cursor);
    let name = "";
    if (source[cursor] === "(") {
      const parens = readBalanced(source, cursor, "(", ")");
      name = firstStringLiteral(parens.text) ?? "";
      cursor = skipWhitespace(source, parens.end + 1);
    }
    if (source[cursor] !== "{") {
      searchFrom = cursor + 1;
      continue;
    }
    const body = readBalanced(source, cursor, "{", "}");
    previews.push({
      body: body.text.trim(),
      index: previews.length,
      name,
      range: [macroIndex, body.end + 1],
    });
    searchFrom = body.end + 1;
  }
  return previews;
}

function selectPreview(previews, selector) {
  if (selector == null || selector === "") {
    return previews[0];
  }
  const numeric = Number(selector);
  if (Number.isInteger(numeric)) {
    const match = previews[numeric - 1] ?? previews[numeric];
    if (match) {
      return match;
    }
  }
  const named = previews.find((item) => item.name === selector);
  if (!named) {
    throw new Error(
      `No preview matched ${JSON.stringify(selector)}. Available: ${previews
        .map((item) => item.name || `#${item.index + 1}`)
        .join(", ")}`,
    );
  }
  return named;
}

function removeRanges(source, ranges) {
  let output = "";
  let cursor = 0;
  for (const [start, end] of ranges.sort((a, b) => a[0] - b[0])) {
    output += source.slice(cursor, start);
    cursor = end;
  }
  output += source.slice(cursor);
  return output;
}

function sanitizePreviewSource(source, moduleName) {
  const stripped = source.replace(
    /(^|\n)(\s*)@main\b/g,
    "$1$2// @main stripped by SimDeck preview runner",
  );
  return xcodeImportPrelude(moduleName) + stripped;
}

function xcodeImportPrelude(moduleName) {
  if (!moduleName) {
    return "";
  }
  return `#if DEBUG
@testable import ${moduleName}
#else
import ${moduleName}
#endif

`;
}

function testableImportPrelude(moduleName) {
  if (!moduleName) {
    return "";
  }
  return `@testable import ${moduleName}

`;
}

function readBalanced(source, start, open, close) {
  let depth = 0;
  let textStart = start + 1;
  for (let index = start; index < source.length; index += 1) {
    const char = source[index];
    if (char === '"' || char === "'") {
      index = skipQuoted(source, index);
      continue;
    }
    if (char === "/" && source[index + 1] === "/") {
      index = skipLineComment(source, index);
      continue;
    }
    if (char === "/" && source[index + 1] === "*") {
      index = skipBlockComment(source, index);
      continue;
    }
    if (char === open) {
      depth += 1;
    } else if (char === close) {
      depth -= 1;
      if (depth === 0) {
        return { end: index, text: source.slice(textStart, index) };
      }
    }
  }
  throw new Error(`Unbalanced ${open}${close} block near offset ${start}.`);
}

function firstStringLiteral(text) {
  const match = text.match(/"((?:\\"|[^"])*)"/);
  return match ? match[1].replace(/\\"/g, '"') : null;
}

function skipWhitespace(source, index) {
  while (/\s/.test(source[index] ?? "")) {
    index += 1;
  }
  return index;
}

function skipQuoted(source, start) {
  const quote = source[start];
  for (let index = start + 1; index < source.length; index += 1) {
    if (source[index] === "\\") {
      index += 1;
      continue;
    }
    if (source[index] === quote) {
      return index;
    }
  }
  return source.length - 1;
}

function skipLineComment(source, start) {
  const next = source.indexOf("\n", start + 2);
  return next < 0 ? source.length - 1 : next;
}

function skipBlockComment(source, start) {
  const next = source.indexOf("*/", start + 2);
  return next < 0 ? source.length - 1 : next + 1;
}

function watchFiles(files, callback) {
  let timer = null;
  for (const file of files) {
    fs.watchFile(file, { interval: 250 }, () => {
      clearTimeout(timer);
      timer = setTimeout(callback, 120);
    });
  }
}

function payloadWrapperSource(previewBody, moduleName, sourceModuleName) {
  return `import SwiftUI
import UIKit
${sourceModuleName ? "" : xcodeImportPrelude(moduleName)}
${testableImportPrelude(sourceModuleName)}

@_cdecl("simdeck_make_preview_view_controller")
public func simdeck_make_preview_view_controller() -> UnsafeMutableRawPointer {
    let rootView = AnyView({
${indent(previewBody, 8)}
    }())
    let controller = UIHostingController(rootView: rootView)
    controller.view.backgroundColor = .systemBackground
    return Unmanaged.passRetained(controller).toOpaque()
}
`;
}

function hostSource() {
  return `import Darwin
import Foundation
import Network
import SwiftUI
import UIKit

private typealias PreviewFactory = @convention(c) () -> UnsafeMutableRawPointer
private let simDeckPreviewReloadPortStart: UInt16 = ${RELOAD_PORT_START}
private let simDeckPreviewReloadPortLimit: UInt16 = ${RELOAD_PORT_LIMIT}
private let simDeckPreviewReloadPrefix = "${RELOAD_PROTOCOL_PREFIX}"
private let simDeckPreviewReloadBytesPrefix = "${RELOAD_BYTES_PROTOCOL_PREFIX}"

@MainActor
final class PreviewStore: ObservableObject {
    @Published var controller: UIViewController?
    @Published var status = "Waiting for SimDeck preview dylib..."
    private var handles: [UnsafeMutableRawPointer] = []
    private var listener: NWListener?

    init() {
        startReloadListener()
    }

    @discardableResult
    func load(path rawPath: String) -> Bool {
        let path = rawPath.removingPercentEncoding ?? rawPath
        guard FileManager.default.fileExists(atPath: path) else {
            status = "Missing dylib: \\(path)"
            return false
        }
        guard let handle = dlopen(path, RTLD_NOW | RTLD_LOCAL) else {
            status = String(cString: dlerror())
            return false
        }
        guard let symbol = dlsym(handle, "simdeck_make_preview_view_controller") else {
            status = "Missing simdeck_make_preview_view_controller"
            return false
        }
        let factory = unsafeBitCast(symbol, to: PreviewFactory.self)
        let pointer = factory()
        let nextController = Unmanaged<UIViewController>.fromOpaque(pointer).takeRetainedValue()
        nextController.view.backgroundColor = .systemBackground
        handles.append(handle)
        controller = nextController
        status = "Loaded \\((path as NSString).lastPathComponent)"
        return true
    }

    private func startReloadListener() {
        for offset in 0..<simDeckPreviewReloadPortLimit {
            guard let port = NWEndpoint.Port(rawValue: simDeckPreviewReloadPortStart + offset) else {
                continue
            }
            do {
                let listener = try NWListener(using: .tcp, on: port)
                listener.newConnectionHandler = { [weak self] connection in
                    self?.accept(connection)
                }
                listener.start(queue: DispatchQueue(label: "dev.simdeck.preview.reload"))
                self.listener = listener
                status = "Waiting for preview reload on TCP \\(port.rawValue)..."
                return
            } catch {
                continue
            }
        }
        status = "Waiting for SimDeck preview URL reload..."
    }

    private nonisolated func accept(_ connection: NWConnection) {
        connection.start(queue: DispatchQueue(label: "dev.simdeck.preview.reload.connection"))
        receive(connection, buffer: Data())
    }

    private nonisolated func receive(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            var nextBuffer = buffer
            if let data {
                nextBuffer.append(data)
            }
            if isComplete || error != nil {
                self?.handleReloadMessage(nextBuffer, connection: connection)
            } else {
                self?.receive(connection, buffer: nextBuffer)
            }
        }
    }

    private nonisolated func handleReloadMessage(_ data: Data, connection: NWConnection) {
        guard let message = String(data: data, encoding: .utf8) else {
            sendResponse("ERROR", on: connection)
            return
        }
        let trimmed = message.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.hasPrefix(simDeckPreviewReloadBytesPrefix) {
            let body = trimmed.dropFirst(simDeckPreviewReloadBytesPrefix.count)
            guard let separator = body.firstIndex(of: " ") else {
                sendResponse("ERROR", on: connection)
                return
            }
            let filename = String(body[..<separator])
            let encoded = String(body[body.index(after: separator)...])
            guard let dylibData = Data(base64Encoded: encoded) else {
                sendResponse("ERROR", on: connection)
                return
            }
            Task { @MainActor [weak self] in
                var loaded = false
                do {
                    let documents = try FileManager.default.url(
                        for: .documentDirectory,
                        in: .userDomainMask,
                        appropriateFor: nil,
                        create: true
                    )
                    let safeName = (filename as NSString).lastPathComponent
                    let destination = documents.appendingPathComponent(safeName)
                    try dylibData.write(to: destination, options: .atomic)
                    loaded = self?.load(path: destination.path) ?? false
                } catch {
                    self?.status = "Reload write failed: \\(error.localizedDescription)"
                }
                self?.sendResponse(loaded ? "OK" : "ERROR", on: connection)
            }
            return
        }
        guard trimmed.hasPrefix(simDeckPreviewReloadPrefix) else {
            sendResponse("ERROR", on: connection)
            return
        }
        let rawPath = String(trimmed.dropFirst(simDeckPreviewReloadPrefix.count))
        Task { @MainActor [weak self] in
            let loaded = self?.load(path: rawPath) ?? false
            self?.sendResponse(loaded ? "OK" : "ERROR", on: connection)
        }
    }

    private nonisolated func sendResponse(_ text: String, on connection: NWConnection) {
        connection.send(content: Data("\\(text)\\n".utf8), completion: .contentProcessed { _ in
            connection.cancel()
        })
    }
}

struct PreviewHostRoot: View {
    @StateObject private var store = PreviewStore()

    var body: some View {
        ZStack {
            if let controller = store.controller {
                ControllerContainer(controller: controller)
                    .ignoresSafeArea()
            } else {
                VStack(spacing: 12) {
                    ProgressView()
                    Text(store.status)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 24)
                }
            }
        }
        .onOpenURL { url in
            guard url.host == "reload",
                  let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
                  let path = components.queryItems?.first(where: { $0.name == "path" })?.value
            else {
                return
            }
            store.load(path: path)
        }
    }
}

struct ControllerContainer: UIViewControllerRepresentable {
    let controller: UIViewController

    func makeUIViewController(context: Context) -> ContainerViewController {
        let container = ContainerViewController()
        container.set(controller)
        return container
    }

    func updateUIViewController(_ uiViewController: ContainerViewController, context: Context) {
        uiViewController.set(controller)
    }
}

final class ContainerViewController: UIViewController {
    private var current: UIViewController?

    func set(_ next: UIViewController) {
        guard current !== next else {
            return
        }
        current?.willMove(toParent: nil)
        current?.view.removeFromSuperview()
        current?.removeFromParent()

        addChild(next)
        next.view.frame = view.bounds
        next.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(next.view)
        next.didMove(toParent: self)
        current = next
    }
}

@main
struct SimDeckPreviewHostApp: App {
    var body: some Scene {
        WindowGroup {
            PreviewHostRoot()
        }
    }
}
`;
}

function indent(text, spaces) {
  const prefix = " ".repeat(spaces);
  return text
    .split("\n")
    .map((line) => `${prefix}${line}`)
    .join("\n");
}

function arrayArg(value) {
  if (value == null) {
    return [];
  }
  return Array.isArray(value) ? value : [value];
}

function splitBuildSetting(value) {
  if (!value) {
    return [];
  }
  const text = String(value);
  const items = [];
  let current = "";
  let quote = "";
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (quote) {
      if (char === "\\") {
        current += text[index + 1] ?? "";
        index += 1;
      } else if (char === quote) {
        quote = "";
      } else {
        current += char;
      }
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
    } else if (/\s/.test(char)) {
      if (current) {
        items.push(current);
        current = "";
      }
    } else {
      current += char;
    }
  }
  if (current) {
    items.push(current);
  }
  return items;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: process.cwd(),
    encoding: "utf8",
    stdio: options.capture ? "pipe" : "inherit",
  });
  if (result.status !== 0 && !options.allowFailure) {
    const details = options.capture
      ? `\n${result.stderr || result.stdout}`
      : "";
    throw new Error(`${command} ${args.join(" ")} failed.${details}`);
  }
  return result;
}

function runText(command, args) {
  const result = run(command, args, { capture: true });
  return result.stdout;
}
