#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";

const root = path.resolve(new URL("../..", import.meta.url).pathname);
const simdeck = path.join(root, "build", "simdeck");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "simdeck-cli-it-"));
const serverPort = Number(process.env.SIMDECK_INTEGRATION_PORT ?? "4510");
const serverUrl = `http://127.0.0.1:${serverPort}`;
const origin = serverUrl;
const systemLaunchBundleId = "com.apple.mobilesafari";

let simulatorUDID = "";
let serverProcess = null;

process.on("SIGINT", () => {
  cleanup();
  process.exit(130);
});
process.on("SIGTERM", () => {
  cleanup();
  process.exit(143);
});
process.on("exit", cleanup);

main()
  .then(() => {
    cleanup();
    process.exit(0);
  })
  .catch((error) => {
    console.error(error?.stack ?? error);
    cleanup();
    process.exit(1);
  });

async function main() {
  if (process.platform !== "darwin") {
    throw new Error("SimDeck CLI integration tests require macOS.");
  }
  if (!fs.existsSync(simdeck)) {
    throw new Error(`Missing ${simdeck}. Run npm run build:cli first.`);
  }

  const runtime = latestAvailableIosRuntime();
  const deviceType = preferredIphoneDeviceType(runtime);
  const simulatorName = `SimDeck CLI Integration ${Date.now()}`;
  simulatorUDID = runText("xcrun", [
    "simctl",
    "create",
    simulatorName,
    deviceType.identifier,
    runtime.identifier,
  ]).trim();

  console.log(
    `created ${simulatorUDID} (${deviceType.name}, ${runtime.version})`,
  );
  simdeckJson(["boot", simulatorUDID]);
  runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
    timeoutMs: 600_000,
  });

  const fixture = buildFixtureApp();
  const bundleId = "dev.nativescript.simdeck.integration.fixture";

  assertSimulatorListed(simulatorUDID);
  assertJson(simdeckJson(["chrome-profile", simulatorUDID]), "chrome-profile");
  assertJson(
    simdeckJson(["logs", simulatorUDID, "--seconds", "5", "--limit", "5"]),
    "logs",
  );

  simdeckJson(["install", simulatorUDID, fixture.appPath]);

  startServer();
  await waitForHealth();

  const fullTree = simdeckJson([
    "describe-ui",
    simulatorUDID,
    "--direct",
    "--max-depth",
    "2",
  ]);
  assertRoots(fullTree, "direct describe-ui json");
  const queryPoint = pointFromSnapshot(fullTree);
  assertJson(
    simdeckJson([
      "describe-ui",
      simulatorUDID,
      "--direct",
      "--format",
      "compact-json",
      "--max-depth",
      "2",
    ]),
    "direct describe-ui compact-json",
  );
  const agentTree = simdeckText([
    "describe-ui",
    simulatorUDID,
    "--server-url",
    serverUrl,
    "--format",
    "agent",
    "--max-depth",
    "2",
  ]);
  if (!agentTree.includes("source:") || !agentTree.includes("- ")) {
    throw new Error("agent describe-ui output did not look like a hierarchy");
  }
  assertRoots(
    simdeckJson([
      "describe-ui",
      simulatorUDID,
      "--point",
      `${queryPoint.x},${queryPoint.y}`,
      "--format",
      "compact-json",
      "--direct",
    ]),
    "point describe-ui compact-json",
  );

  runCliControls();
  await runRestControls(queryPoint);

  const screenshotPath = path.join(tempRoot, "screen.png");
  simdeckJson(["screenshot", simulatorUDID, "--output", screenshotPath]);
  assertPng(screenshotPath);
  const stdoutPng = path.join(tempRoot, "screen-stdout.png");
  fs.writeFileSync(
    stdoutPng,
    runBuffer(simdeck, ["screenshot", simulatorUDID, "--stdout"], {
      timeoutMs: 120_000,
    }),
  );
  assertPng(stdoutPng);

  simdeckJson(["pasteboard", "set", simulatorUDID, "simdeck integration"]);
  const pasteboard = simdeckJson(["pasteboard", "get", simulatorUDID]);
  if (pasteboard.text !== "simdeck integration") {
    throw new Error(
      `pasteboard round-trip failed: ${JSON.stringify(pasteboard)}`,
    );
  }

  const fileInput = path.join(tempRoot, "type.txt");
  fs.writeFileSync(fileInput, "file input");
  simdeckJson(["type", simulatorUDID, "--file", fileInput]);
  simdeckJson(["type", simulatorUDID, "--stdin"], {
    input: "stdin input",
  });

  const batch = simdeckJson([
    "batch",
    simulatorUDID,
    "--step",
    "button home",
    "--step",
    "tap --x 200 --y 700 --duration-ms 20",
    "--step",
    "type batch",
    "--continue-on-error",
  ]);
  if (batch.ok !== true || batch.failureCount !== 0) {
    throw new Error(`batch command failed: ${JSON.stringify(batch)}`);
  }

  runHardwareButtonControls();

  simdeckJson(["uninstall", simulatorUDID, bundleId]);
  simdeckJson(["shutdown", simulatorUDID]);
  simdeckJson(["erase", simulatorUDID]);
  simdeckJson(["boot", simulatorUDID]);
  runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
    timeoutMs: 600_000,
  });
  assertRoots(
    simdeckJson([
      "describe-ui",
      simulatorUDID,
      "--direct",
      "--format",
      "compact-json",
      "--max-depth",
      "1",
    ]),
    "post-erase describe-ui",
  );

  console.log("SimDeck CLI integration suite passed");
}

function runCliControls() {
  simdeckJson(["home", simulatorUDID]);
  simdeckJson(["tap", simulatorUDID, "200", "700", "--duration-ms", "20"]);
  simdeckJson([
    "touch",
    simulatorUDID,
    "0.5",
    "0.5",
    "--phase",
    "began",
    "--normalized",
  ]);
  simdeckJson([
    "touch",
    simulatorUDID,
    "0.5",
    "0.5",
    "--phase",
    "ended",
    "--normalized",
  ]);
  simdeckJson([
    "touch",
    simulatorUDID,
    "120",
    "240",
    "--down",
    "--up",
    "--delay-ms",
    "20",
  ]);
  simdeckJson([
    "swipe",
    simulatorUDID,
    "200",
    "700",
    "200",
    "300",
    "--duration-ms",
    "100",
    "--steps",
    "4",
  ]);
  simdeckJson([
    "gesture",
    simulatorUDID,
    "scroll-down",
    "--duration-ms",
    "100",
    "--delta",
    "100",
  ]);
  simdeckJson([
    "gesture",
    simulatorUDID,
    "swipe-from-left-edge",
    "--duration-ms",
    "100",
  ]);
  simdeckJson([
    "pinch",
    simulatorUDID,
    "--start-distance",
    "0.15",
    "--end-distance",
    "0.25",
    "--normalized",
    "--duration-ms",
    "100",
    "--steps",
    "4",
  ]);
  simdeckJson([
    "rotate-gesture",
    simulatorUDID,
    "--radius",
    "0.10",
    "--degrees",
    "30",
    "--normalized",
    "--duration-ms",
    "100",
    "--steps",
    "4",
  ]);
  simdeckJson(["key", simulatorUDID, "enter"]);
  simdeckJson([
    "key-sequence",
    simulatorUDID,
    "--keycodes",
    "h,e,l,l,o",
    "--delay-ms",
    "5",
  ]);
  simdeckJson(["key-combo", simulatorUDID, "--modifiers", "cmd", "--key", "a"]);
  simdeckJson(["type", simulatorUDID, "qa"]);
  simdeckJson(["dismiss-keyboard", simulatorUDID]);
  simdeckJson(["app-switcher", simulatorUDID]);
  simdeckJson(["home", simulatorUDID]);
  simdeckJson(["rotate-left", simulatorUDID]);
  simdeckJson(["rotate-right", simulatorUDID]);
  simdeckJson(["toggle-appearance", simulatorUDID]);
  simdeckJson(["open-url", simulatorUDID, "https://example.com"]);
  simdeckJson(["launch", simulatorUDID, systemLaunchBundleId]);
}

function runHardwareButtonControls() {
  simdeckJson(["button", simulatorUDID, "home"]);
  simdeckJson(["button", simulatorUDID, "lock", "--duration-ms", "50"]);
  simdeckJson(["button", simulatorUDID, "lock", "--duration-ms", "50"]);
  simdeckJson(["button", simulatorUDID, "side-button", "--duration-ms", "50"]);
  simdeckJson(["button", simulatorUDID, "side-button", "--duration-ms", "50"]);
  simdeckJson(["button", simulatorUDID, "siri", "--duration-ms", "50"]);
  simdeckJson(["home", simulatorUDID]);
  simdeckJson(["button", simulatorUDID, "apple-pay", "--duration-ms", "50"]);
  simdeckJson(["home", simulatorUDID]);
}

async function runRestControls(queryPoint) {
  const simulators = await httpJson("GET", "/api/simulators");
  if (
    !simulators.simulators?.some(
      (simulator) => simulator.udid === simulatorUDID,
    )
  ) {
    throw new Error("REST simulator list did not include temp simulator");
  }
  assertRoots(
    await httpJson(
      "GET",
      `/api/simulators/${simulatorUDID}/accessibility-tree?maxDepth=1`,
    ),
    "REST accessibility-tree",
  );
  assertRoots(
    await httpJson(
      "GET",
      `/api/simulators/${simulatorUDID}/accessibility-point?x=${queryPoint.x}&y=${queryPoint.y}`,
    ),
    "REST accessibility-point",
  );
  assertJson(
    await httpJson("GET", `/api/simulators/${simulatorUDID}/chrome-profile`),
    "REST chrome-profile",
  );
  assertPngBuffer(
    await httpBuffer("GET", `/api/simulators/${simulatorUDID}/chrome.png`),
  );

  await httpJson("POST", `/api/simulators/${simulatorUDID}/touch`, {
    x: 120,
    y: 240,
    phase: "began",
  });
  await httpJson("POST", `/api/simulators/${simulatorUDID}/touch`, {
    x: 120,
    y: 240,
    phase: "ended",
  });
  await httpJson("POST", `/api/simulators/${simulatorUDID}/key`, {
    keyCode: 40,
    modifiers: 0,
  });
  await httpJson("POST", `/api/simulators/${simulatorUDID}/home`, {});
  await httpJson("POST", `/api/simulators/${simulatorUDID}/app-switcher`, {});
  await httpJson("POST", `/api/simulators/${simulatorUDID}/rotate-left`, {});
  await httpJson("POST", `/api/simulators/${simulatorUDID}/rotate-right`, {});
  await httpJson("POST", `/api/simulators/${simulatorUDID}/open-url`, {
    url: "https://example.com",
  });
  await httpJson("POST", `/api/simulators/${simulatorUDID}/launch`, {
    bundleId: systemLaunchBundleId,
  });
  await httpJson(
    "POST",
    `/api/simulators/${simulatorUDID}/toggle-appearance`,
    {},
  );
}

function latestAvailableIosRuntime() {
  const payload = runJson("xcrun", ["simctl", "list", "runtimes", "-j"]);
  const runtimes = payload.runtimes
    .filter(
      (runtime) => runtime.isAvailable && runtime.identifier?.includes("iOS"),
    )
    .sort(compareRuntimeVersions);
  const runtime = runtimes.at(-1);
  if (!runtime) {
    throw new Error("No available iOS simulator runtime found.");
  }
  return runtime;
}

function preferredIphoneDeviceType(runtime) {
  const runtimeSupported = Array.isArray(runtime.supportedDeviceTypes)
    ? runtime.supportedDeviceTypes
    : [];
  const allDeviceTypes = runJson("xcrun", [
    "simctl",
    "list",
    "devicetypes",
    "-j",
  ]).devicetypes;
  const supported =
    runtimeSupported.length > 0
      ? runtimeSupported
      : allDeviceTypes.filter(
          (device) =>
            device.productFamily === "iPhone" ||
            device.identifier?.includes("iPhone"),
        );
  const iphones = supported.filter(
    (device) =>
      device.productFamily === "iPhone" ||
      device.identifier?.includes("iPhone"),
  );
  const preferred = [
    "iPhone 17",
    "iPhone 16",
    "iPhone 15",
    "iPhone 14",
    "iPhone 13",
  ];
  for (const name of preferred) {
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

function compareRuntimeVersions(left, right) {
  const leftParts = String(left.version ?? "0")
    .split(".")
    .map(Number);
  const rightParts = String(right.version ?? "0")
    .split(".")
    .map(Number);
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
  return String(left.identifier).localeCompare(String(right.identifier));
}

function buildFixtureApp() {
  const appPath = path.join(tempRoot, "SimDeckFixture.app");
  fs.mkdirSync(appPath, { recursive: true });
  const executable = "SimDeckFixture";
  fs.writeFileSync(
    path.join(appPath, "Info.plist"),
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>${executable}</string>
  <key>CFBundleIdentifier</key><string>dev.nativescript.simdeck.integration.fixture</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>SimDeckFixture</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>MinimumOSVersion</key><string>15.0</string>
  <key>UIDeviceFamily</key><array><integer>1</integer></array>
</dict>
</plist>
`,
  );
  const main = path.join(tempRoot, "main.m");
  fs.writeFileSync(
    main,
    `#import <UIKit/UIKit.h>

@interface AppDelegate : UIResponder <UIApplicationDelegate>
@property(nonatomic, strong) UIWindow *window;
@end

@implementation AppDelegate
- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
  self.window = [[UIWindow alloc] initWithFrame:[UIScreen mainScreen].bounds];
  UIViewController *controller = [UIViewController new];
  controller.view.backgroundColor = UIColor.systemBackgroundColor;

  UILabel *label = [UILabel new];
  label.text = @"SimDeck Fixture";
  label.accessibilityIdentifier = @"fixture.label";
  label.textAlignment = NSTextAlignmentCenter;

  UIButton *button = [UIButton buttonWithType:UIButtonTypeSystem];
  [button setTitle:@"Continue" forState:UIControlStateNormal];
  button.accessibilityIdentifier = @"fixture.continue";

  UIStackView *stack = [[UIStackView alloc] initWithArrangedSubviews:@[label, button]];
  stack.axis = UILayoutConstraintAxisVertical;
  stack.spacing = 24;
  stack.translatesAutoresizingMaskIntoConstraints = NO;
  [controller.view addSubview:stack];

  [NSLayoutConstraint activateConstraints:@[
    [stack.centerXAnchor constraintEqualToAnchor:controller.view.centerXAnchor],
    [stack.centerYAnchor constraintEqualToAnchor:controller.view.centerYAnchor],
    [stack.widthAnchor constraintEqualToConstant:240],
  ]];

  self.window.rootViewController = controller;
  [self.window makeKeyAndVisible];
  return YES;
}
@end

int main(int argc, char * argv[]) {
  @autoreleasepool {
    return UIApplicationMain(argc, argv, nil, NSStringFromClass([AppDelegate class]));
  }
}
`,
  );
  const targetArch = process.arch === "arm64" ? "arm64" : "x86_64";
  runText("xcrun", [
    "--sdk",
    "iphonesimulator",
    "clang",
    "-target",
    `${targetArch}-apple-ios15.0-simulator`,
    "-fobjc-arc",
    "-mios-simulator-version-min=15.0",
    "-framework",
    "UIKit",
    "-framework",
    "Foundation",
    main,
    "-o",
    path.join(appPath, executable),
  ]);
  return { appPath };
}

function startServer() {
  killPortListeners(serverPort);
  serverProcess = spawn(
    simdeck,
    [
      "serve",
      "--port",
      String(serverPort),
      "--client-root",
      path.join(root, "client", "dist"),
      "--video-codec",
      "h264-software",
    ],
    {
      cwd: root,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  serverProcess.stdout.on("data", (data) =>
    process.stdout.write(`[serve] ${data}`),
  );
  serverProcess.stderr.on("data", (data) =>
    process.stderr.write(`[serve] ${data}`),
  );
}

async function waitForHealth() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const health = await httpJson("GET", "/api/health");
      if (health.httpPort === serverPort) {
        return;
      }
    } catch {
      await sleep(250);
    }
  }
  throw new Error("Timed out waiting for SimDeck integration server.");
}

function simdeckJson(args, options = {}) {
  return JSON.parse(simdeckText(args, options));
}

function simdeckText(args, options = {}) {
  return runText(simdeck, args, {
    timeoutMs: options.timeoutMs ?? 120_000,
    input: options.input,
  });
}

function runJson(command, args, options = {}) {
  return JSON.parse(runText(command, args, options));
}

function runText(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    input: options.input,
    timeout: options.timeoutMs ?? 120_000,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with ${result.status ?? result.signal}\n${result.stdout}\n${result.stderr}`,
    );
  }
  return result.stdout;
}

function runBuffer(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "buffer",
    timeout: options.timeoutMs ?? 120_000,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with ${result.status ?? result.signal}`,
    );
  }
  return result.stdout;
}

async function httpJson(method, requestPath, body) {
  const buffer = await httpBuffer(method, requestPath, body);
  return JSON.parse(buffer.toString("utf8"));
}

function httpBuffer(method, requestPath, body) {
  const payload = body === undefined ? null : Buffer.from(JSON.stringify(body));
  return new Promise((resolve, reject) => {
    const request = http.request(
      {
        hostname: "127.0.0.1",
        port: serverPort,
        path: requestPath,
        method,
        headers: {
          Origin: origin,
          Accept: "application/json",
          ...(payload
            ? {
                "Content-Type": "application/json",
                "Content-Length": payload.length,
              }
            : {}),
        },
      },
      (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => {
          const buffer = Buffer.concat(chunks);
          if (response.statusCode < 200 || response.statusCode >= 300) {
            reject(
              new Error(
                `${method} ${requestPath} returned ${response.statusCode}: ${buffer.toString("utf8")}`,
              ),
            );
            return;
          }
          resolve(buffer);
        });
      },
    );
    request.on("error", reject);
    if (payload) {
      request.write(payload);
    }
    request.end();
  });
}

function assertSimulatorListed(udid) {
  const payload = simdeckJson(["list"]);
  if (!payload.simulators?.some((simulator) => simulator.udid === udid)) {
    throw new Error(`simdeck list did not include ${udid}`);
  }
}

function assertRoots(payload, label) {
  assertJson(payload, label);
  if (!Array.isArray(payload.roots) || payload.roots.length === 0) {
    throw new Error(`${label} returned no roots: ${JSON.stringify(payload)}`);
  }
}

function assertJson(payload, label) {
  if (!payload || typeof payload !== "object") {
    throw new Error(`${label} did not return a JSON object`);
  }
}

function assertPng(filePath) {
  assertPngBuffer(fs.readFileSync(filePath));
}

function assertPngBuffer(buffer) {
  const pngSignature = "89504e470d0a1a0a";
  if (buffer.subarray(0, 8).toString("hex") !== pngSignature) {
    throw new Error("Expected PNG data.");
  }
}

function pointFromSnapshot(snapshot) {
  const root = snapshot.roots?.[0];
  const node = findPreferredPointNode(root) ?? findLeafPointNode(root) ?? root;
  const frame = node?.frame ?? node?.frameInScreen;
  if (!frame || typeof frame !== "object") {
    throw new Error(
      `Unable to derive point from snapshot: ${JSON.stringify(snapshot)}`,
    );
  }
  const x = Number(frame.x) + Number(frame.width) / 2;
  const y = Number(frame.y) + Number(frame.height) / 2;
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    throw new Error(`Snapshot root frame is invalid: ${JSON.stringify(frame)}`);
  }
  return {
    x: Math.round(x),
    y: Math.round(y),
  };
}

function findPreferredPointNode(node) {
  if (!node || typeof node !== "object") {
    return null;
  }
  const text = [
    node.label,
    node.title,
    node.value,
    node.text,
    node.name,
    node.identifier,
    node.accessibilityIdentifier,
  ]
    .filter((value) => typeof value === "string")
    .join(" ");
  if (/SimDeck Fixture|Continue|fixture\./.test(text) && hasUsableFrame(node)) {
    return node;
  }
  const children = Array.isArray(node.children) ? node.children : [];
  for (const child of children) {
    const match = findPreferredPointNode(child);
    if (match) {
      return match;
    }
  }
  return null;
}

function findLeafPointNode(node) {
  if (!node || typeof node !== "object") {
    return null;
  }
  const children = Array.isArray(node.children) ? node.children : [];
  for (const child of children) {
    const match = findLeafPointNode(child);
    if (match) {
      return match;
    }
  }
  if (children.length === 0 && hasUsableFrame(node)) {
    return node;
  }
  return null;
}

function hasUsableFrame(node) {
  const frame = node.frame ?? node.frameInScreen;
  return (
    frame &&
    Number(frame.width) > 4 &&
    Number(frame.height) > 4 &&
    Number.isFinite(Number(frame.x)) &&
    Number.isFinite(Number(frame.y))
  );
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function cleanup() {
  if (serverProcess && !serverProcess.killed) {
    serverProcess.kill("SIGTERM");
    serverProcess = null;
  }
  killPortListeners(serverPort);
  if (simulatorUDID) {
    spawnSync("xcrun", ["simctl", "shutdown", simulatorUDID], {
      stdio: "ignore",
    });
    spawnSync("xcrun", ["simctl", "delete", simulatorUDID], {
      stdio: "ignore",
    });
    simulatorUDID = "";
  }
  if (fs.existsSync(tempRoot)) {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

function killPortListeners(port) {
  const result = spawnSync(
    "lsof",
    ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN", "-t"],
    {
      encoding: "utf8",
    },
  );
  if (result.status !== 0 || !result.stdout.trim()) {
    return;
  }
  for (const pid of result.stdout.trim().split(/\s+/)) {
    if (pid && pid !== String(process.pid)) {
      spawnSync("kill", ["-TERM", pid], { stdio: "ignore" });
    }
  }
}
