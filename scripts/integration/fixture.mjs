import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const executable = "SimDeckFixture";
const minimumIosVersion = "15.0";

export function buildCachedFixtureApp({
  root,
  tempRoot,
  bundleId,
  urlScheme,
  log = () => {},
}) {
  const targetArch = process.arch === "arm64" ? "arm64" : "x86_64";
  const sdkVersion = commandOutput("xcrun", [
    "--sdk",
    "iphonesimulator",
    "--show-sdk-version",
  ]);
  const swiftVersion = commandOutput("xcrun", [
    "--sdk",
    "iphonesimulator",
    "swiftc",
    "--version",
  ]);
  const plist = fixtureInfoPlist(bundleId, urlScheme);
  const source = fixtureSwiftSource();
  const fingerprint = crypto
    .createHash("sha256")
    .update(
      JSON.stringify({ targetArch, sdkVersion, swiftVersion, plist, source }),
    )
    .digest("hex")
    .slice(0, 16);
  const cacheRoot = path.join(
    root,
    ".cache",
    "simdeck",
    "fixture",
    `${targetArch}-iphonesimulator-${fingerprint}`,
  );
  const cachedAppPath = path.join(cacheRoot, `${executable}.app`);
  const appPath = path.join(tempRoot, `${executable}.app`);

  if (!isUsableApp(cachedAppPath)) {
    log(`building cached SwiftUI fixture ${fingerprint}`);
    buildFixtureIntoCache({
      cacheRoot,
      cachedAppPath,
      plist,
      source,
      targetArch,
    });
  } else {
    log(`using cached SwiftUI fixture ${fingerprint}`);
  }

  fs.rmSync(appPath, { recursive: true, force: true });
  fs.cpSync(cachedAppPath, appPath, { recursive: true });
  return { appPath };
}

function buildFixtureIntoCache({
  cacheRoot,
  cachedAppPath,
  plist,
  source,
  targetArch,
}) {
  const stagingRoot = `${cacheRoot}.tmp-${process.pid}-${Date.now()}`;
  const stagingApp = path.join(stagingRoot, `${executable}.app`);
  fs.rmSync(stagingRoot, { recursive: true, force: true });
  fs.mkdirSync(stagingApp, { recursive: true });

  const plistPath = path.join(stagingApp, "Info.plist");
  const sourcePath = path.join(stagingRoot, `${executable}.swift`);
  fs.writeFileSync(plistPath, plist);
  fs.writeFileSync(sourcePath, source);

  run("xcrun", [
    "--sdk",
    "iphonesimulator",
    "swiftc",
    "-target",
    `${targetArch}-apple-ios${minimumIosVersion}-simulator`,
    "-parse-as-library",
    "-Onone",
    "-framework",
    "SwiftUI",
    "-framework",
    "UIKit",
    sourcePath,
    "-o",
    path.join(stagingApp, executable),
  ]);

  fs.rmSync(cacheRoot, { recursive: true, force: true });
  fs.mkdirSync(path.dirname(cacheRoot), { recursive: true });
  fs.renameSync(stagingRoot, cacheRoot);

  if (!isUsableApp(cachedAppPath)) {
    throw new Error(`Cached fixture app was not created at ${cachedAppPath}`);
  }
}

function fixtureInfoPlist(bundleId, urlScheme) {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>${executable}</string>
  <key>CFBundleIdentifier</key><string>${bundleId}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>${executable}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>MinimumOSVersion</key><string>${minimumIosVersion}</string>
  <key>UIDeviceFamily</key><array><integer>1</integer></array>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>${executable}</string>
      <key>CFBundleURLSchemes</key>
      <array><string>${urlScheme}</string></array>
    </dict>
  </array>
</dict>
</plist>
`;
}

function fixtureSwiftSource() {
  return `import SwiftUI

struct FixtureView: View {
  @State private var status = "Integration Ready"
  @State private var tapCount = 0
  @State private var message = ""
  @FocusState private var messageFocused: Bool

  var body: some View {
    VStack(spacing: 24) {
      Text("SimDeck Fixture")
        .font(.title2)
        .accessibilityIdentifier("fixture.title")

      Text(status)
        .accessibilityIdentifier("fixture.status")

      Button("Continue") {
        tapCount += 1
        status = "Continue Tapped \\(tapCount)"
      }
        .buttonStyle(.borderedProminent)
        .accessibilityIdentifier("fixture.continue")

      TextField("Message", text: $message)
        .textFieldStyle(.roundedBorder)
        .accessibilityIdentifier("fixture.message")
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled(true)
        .focused($messageFocused)
        .frame(width: 240)
    }
    .padding()
    .onOpenURL { url in
      if url.host == "focus-message" {
        status = "Message Focused"
        messageFocused = true
      } else {
        status = "URL Opened"
      }
    }
  }
}

@main
struct SimDeckFixtureApp: App {
  var body: some Scene {
    WindowGroup {
      FixtureView()
    }
  }
}
`;
}

function isUsableApp(appPath) {
  const binary = path.join(appPath, executable);
  return (
    fs.existsSync(path.join(appPath, "Info.plist")) && isExecutable(binary)
  );
}

function isExecutable(filePath) {
  try {
    fs.accessSync(filePath, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function commandOutput(command, args) {
  return run(command, args).stdout.trim();
}

function run(command, args) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    timeout: 300_000,
    maxBuffer: 1024 * 1024 * 4,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with status ${result.status}: ${[
        result.stderr,
        result.stdout,
      ]
        .filter(Boolean)
        .join("\n")}`,
    );
  }
  return result;
}
