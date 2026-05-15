#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { selectIntegrationSimulator } from "./simulator-selection.mjs";

const root = path.resolve(new URL("../..", import.meta.url).pathname);
const simdeck = path.join(root, "build", "simdeck");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "simdeck-camera-it-"));
const executable = "SimDeckCameraFixture";
const bundleId = "dev.nativescript.simdeck.integration.camera";
const minimumIosVersion = "15.0";
const verbose = process.env.SIMDECK_INTEGRATION_VERBOSE === "1";
const showSimulator = process.env.SIMDECK_INTEGRATION_SHOW_SIMULATOR === "1";
const keepSimulator = process.env.SIMDECK_INTEGRATION_KEEP_SIMULATOR === "1";
const commandTimeoutMs = Number(
  process.env.SIMDECK_INTEGRATION_SIMCTL_TIMEOUT_MS ?? "300000",
);

let simulatorUDID = "";

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
    console.log("SimDeck camera integration suite passed");
    process.exit(0);
  })
  .catch((error) => {
    console.error(error?.stack ?? error);
    cleanup();
    process.exit(1);
  });

async function main() {
  if (process.platform !== "darwin") {
    throw new Error("SimDeck camera integration tests require macOS.");
  }
  if (!fs.existsSync(simdeck)) {
    throw new Error(`Missing ${simdeck}. Run npm run build:cli first.`);
  }

  step("select simulator runtime");
  const { runtime, deviceType, sdkVersion } = selectIntegrationSimulator({
    runJson,
    runText,
    timeoutMs: commandTimeoutMs,
  });
  const simulatorName = `SimDeck Camera Integration ${Date.now()}`;
  simulatorUDID = runText(
    "xcrun",
    [
      "simctl",
      "create",
      simulatorName,
      deviceType.identifier,
      runtime.identifier,
    ],
    { timeoutMs: commandTimeoutMs },
  ).trim();
  console.log(
    `created ${simulatorUDID} (${deviceType.name}, ${runtime.version}; iphonesimulator SDK ${sdkVersion})`,
  );

  step("boot simulator");
  runText("xcrun", ["simctl", "boot", simulatorUDID], {
    allowFailure: true,
    timeoutMs: commandTimeoutMs,
  });
  runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
    timeoutMs: 600_000,
  });
  if (showSimulator) {
    runText(
      "open",
      ["-a", "Simulator", "--args", "-CurrentDeviceUDID", simulatorUDID],
      {
        allowFailure: true,
        timeoutMs: 30_000,
      },
    );
  }

  step("build camera fixture app");
  const appPath = buildCameraFixtureApp();
  const imagePath = path.join(tempRoot, "solid-red.bmp");
  const videoPath = path.join(tempRoot, "solid-green.mov");
  writeSolidBmp(imagePath, 32, 24, { r: 255, g: 0, b: 0 });
  writeSolidMov(videoPath, 64, 48, { r: 0, g: 255, b: 0 });

  step("install camera fixture app");
  runText("xcrun", ["simctl", "install", simulatorUDID, appPath], {
    timeoutMs: commandTimeoutMs,
  });

  step("check camera sources");
  const sources = simdeckJson(["camera", "sources"]);
  if (!Array.isArray(sources.webcams)) {
    throw new Error(
      `camera sources did not return a webcams array: ${JSON.stringify(sources)}`,
    );
  }

  step("verify initial camera status");
  const initialStatus = simdeckJson(["camera", "status", simulatorUDID]);
  if (initialStatus.alive !== false) {
    throw new Error(
      `expected daemon camera feed to be stopped: ${JSON.stringify(initialStatus)}`,
    );
  }

  step("start injected app with image source");
  const startStatus = simdeckJson([
    "camera",
    "start",
    simulatorUDID,
    bundleId,
    "--file",
    imagePath,
    "--mirror",
    "off",
  ]);
  assertCameraStatus(startStatus, "image");

  const imageMarker = await waitForMarker(
    "solid red image frames",
    (marker) => {
      return (
        marker.frames > 0 &&
        marker.width === 1280 &&
        marker.height === 720 &&
        marker.avgRed > 180 &&
        marker.avgGreen < 90 &&
        marker.avgBlue < 90
      );
    },
  );
  console.log(
    `received image frames: frames=${imageMarker.frames} rgb=${Math.round(imageMarker.avgRed)},${Math.round(imageMarker.avgGreen)},${Math.round(imageMarker.avgBlue)}`,
  );

  step("switch to static video source");
  const videoStatus = simdeckJson([
    "camera",
    "switch",
    simulatorUDID,
    "--file",
    videoPath,
    "--mirror",
    "off",
  ]);
  assertCameraStatus(videoStatus, "video");

  const videoMarker = await waitForMarker(
    "solid green video frames",
    (marker) => {
      return (
        marker.frames > imageMarker.frames &&
        marker.avgRed < 90 &&
        marker.avgGreen > 180 &&
        marker.avgBlue < 90
      );
    },
  );
  console.log(
    `received video frames: frames=${videoMarker.frames} rgb=${Math.round(videoMarker.avgRed)},${Math.round(videoMarker.avgGreen)},${Math.round(videoMarker.avgBlue)}`,
  );

  step("switch to placeholder source");
  const switchStatus = simdeckJson([
    "camera",
    "switch",
    simulatorUDID,
    "--placeholder",
    "--mirror",
    "off",
  ]);
  assertCameraStatus(switchStatus, "placeholder");

  const placeholderMarker = await waitForMarker(
    "placeholder frames",
    (marker) => {
      return (
        marker.frames > videoMarker.frames &&
        marker.avgRed > 120 &&
        marker.avgGreen > 60 &&
        marker.avgBlue > 60
      );
    },
  );
  console.log(
    `received placeholder frames: frames=${placeholderMarker.frames} rgb=${Math.round(placeholderMarker.avgRed)},${Math.round(placeholderMarker.avgGreen)},${Math.round(placeholderMarker.avgBlue)}`,
  );

  step("stop daemon camera feed");
  const stopStatus = simdeckJson(["camera", "stop", simulatorUDID]);
  if (stopStatus.alive !== false) {
    throw new Error(
      `camera stop did not report alive=false: ${JSON.stringify(stopStatus)}`,
    );
  }
}

function buildCameraFixtureApp() {
  const targetArch = process.arch === "arm64" ? "arm64" : "x86_64";
  const appPath = path.join(tempRoot, `${executable}.app`);
  const sourcePath = path.join(tempRoot, `${executable}.m`);
  fs.mkdirSync(appPath, { recursive: true });
  fs.writeFileSync(path.join(appPath, "Info.plist"), fixtureInfoPlist());
  fs.writeFileSync(sourcePath, fixtureSource());
  runText("xcrun", [
    "--sdk",
    "iphonesimulator",
    "clang",
    "-target",
    `${targetArch}-apple-ios${minimumIosVersion}-simulator`,
    "-fobjc-arc",
    "-fmodules",
    "-framework",
    "AVFoundation",
    "-framework",
    "CoreGraphics",
    "-framework",
    "CoreMedia",
    "-framework",
    "CoreVideo",
    "-framework",
    "Foundation",
    "-framework",
    "UIKit",
    sourcePath,
    "-o",
    path.join(appPath, executable),
  ]);
  return appPath;
}

function writeSolidMov(outputPath, width, height, color) {
  const sourcePath = path.join(tempRoot, "WriteSolidMov.m");
  const binaryPath = path.join(tempRoot, "WriteSolidMov");
  fs.writeFileSync(
    sourcePath,
    `#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>

int main(int argc, const char *argv[]) {
  @autoreleasepool {
    if (argc < 2) return 64;
    NSString *path = [NSString stringWithUTF8String:argv[1]];
    [[NSFileManager defaultManager] removeItemAtPath:path error:nil];
    NSURL *url = [NSURL fileURLWithPath:path];
    NSError *error = nil;
    AVAssetWriter *writer = [AVAssetWriter assetWriterWithURL:url fileType:AVFileTypeQuickTimeMovie error:&error];
    if (!writer) {
      fprintf(stderr, "%s\\n", error.localizedDescription.UTF8String);
      return 1;
    }
    NSDictionary *settings = @{
      AVVideoCodecKey: AVVideoCodecTypeH264,
      AVVideoWidthKey: @(${width}),
      AVVideoHeightKey: @(${height}),
    };
    AVAssetWriterInput *input = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:settings];
    input.expectsMediaDataInRealTime = NO;
    NSDictionary *attributes = @{
      (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
      (id)kCVPixelBufferWidthKey: @(${width}),
      (id)kCVPixelBufferHeightKey: @(${height}),
    };
    AVAssetWriterInputPixelBufferAdaptor *adaptor = [AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:input sourcePixelBufferAttributes:attributes];
    if (![writer canAddInput:input]) return 2;
    [writer addInput:input];
    if (![writer startWriting]) return 3;
    [writer startSessionAtSourceTime:kCMTimeZero];
    for (int frame = 0; frame < 90; frame += 1) {
      while (!input.readyForMoreMediaData) {
        [NSThread sleepForTimeInterval:0.01];
      }
      CVPixelBufferRef pixelBuffer = NULL;
      CVReturn status = CVPixelBufferPoolCreatePixelBuffer(NULL, adaptor.pixelBufferPool, &pixelBuffer);
      if (status != kCVReturnSuccess || !pixelBuffer) return 4;
      CVPixelBufferLockBaseAddress(pixelBuffer, 0);
      uint8_t *base = (uint8_t *)CVPixelBufferGetBaseAddress(pixelBuffer);
      size_t bytesPerRow = CVPixelBufferGetBytesPerRow(pixelBuffer);
      for (int y = 0; y < ${height}; y += 1) {
        uint8_t *row = base + (size_t)y * bytesPerRow;
        for (int x = 0; x < ${width}; x += 1) {
          uint8_t *pixel = row + (size_t)x * 4;
          pixel[0] = ${color.b};
          pixel[1] = ${color.g};
          pixel[2] = ${color.r};
          pixel[3] = 255;
        }
      }
      CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);
      CMTime presentationTime = CMTimeMake(frame, 30);
      if (![adaptor appendPixelBuffer:pixelBuffer withPresentationTime:presentationTime]) return 5;
      CVPixelBufferRelease(pixelBuffer);
    }
    [input markAsFinished];
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    [writer finishWritingWithCompletionHandler:^{
      dispatch_semaphore_signal(semaphore);
    }];
    dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
    return writer.status == AVAssetWriterStatusCompleted ? 0 : 6;
  }
}
`,
  );
  runText("clang", [
    "-fobjc-arc",
    "-fmodules",
    "-framework",
    "AVFoundation",
    "-framework",
    "CoreMedia",
    "-framework",
    "CoreVideo",
    "-framework",
    "Foundation",
    sourcePath,
    "-o",
    binaryPath,
  ]);
  runText(binaryPath, [outputPath]);
}

function fixtureInfoPlist() {
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
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>${executable}</string>
      <key>CFBundleURLSchemes</key>
      <array><string>simdeck-camera-fixture</string></array>
    </dict>
  </array>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>MinimumOSVersion</key><string>${minimumIosVersion}</string>
  <key>NSCameraUsageDescription</key><string>Camera fixture validates SimDeck camera simulation.</string>
  <key>UIDeviceFamily</key><array><integer>1</integer></array>
  <key>UIApplicationSceneManifest</key>
  <dict>
    <key>UIApplicationSupportsMultipleScenes</key><false/>
    <key>UISceneConfigurations</key>
    <dict>
      <key>UIWindowSceneSessionRoleApplication</key>
      <array>
        <dict>
          <key>UISceneConfigurationName</key><string>Default Configuration</string>
          <key>UISceneDelegateClassName</key><string>SceneDelegate</string>
        </dict>
      </array>
    </dict>
  </dict>
</dict>
</plist>
`;
}

function fixtureSource() {
  return `#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <UIKit/UIKit.h>
#import <objc/runtime.h>

@interface CameraViewController : UIViewController <AVCaptureVideoDataOutputSampleBufferDelegate>
@property (nonatomic, strong) AVCaptureSession *session;
@property (nonatomic, strong) UILabel *statusLabel;
@property (nonatomic) NSInteger frames;
@end

@implementation CameraViewController

- (void)viewDidLoad {
  [super viewDidLoad];
  self.view.backgroundColor = UIColor.systemBackgroundColor;
  self.statusLabel = [[UILabel alloc] init];
  self.statusLabel.text = @"Camera Starting";
  self.statusLabel.textAlignment = NSTextAlignmentCenter;
  self.statusLabel.numberOfLines = 0;
  self.statusLabel.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline];
  self.statusLabel.accessibilityIdentifier = @"camera.status";
  self.statusLabel.translatesAutoresizingMaskIntoConstraints = NO;
  [self.view addSubview:self.statusLabel];
  [NSLayoutConstraint activateConstraints:@[
    [self.statusLabel.centerXAnchor constraintEqualToAnchor:self.view.centerXAnchor],
    [self.statusLabel.centerYAnchor constraintEqualToAnchor:self.view.centerYAnchor],
    [self.statusLabel.leadingAnchor constraintGreaterThanOrEqualToAnchor:self.view.safeAreaLayoutGuide.leadingAnchor constant:24.0],
    [self.statusLabel.trailingAnchor constraintLessThanOrEqualToAnchor:self.view.safeAreaLayoutGuide.trailingAnchor constant:-24.0],
  ]];
  [self writeMarkerWithStatus:@"view-loaded" width:0 height:0 red:0 green:0 blue:0];
  dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(2 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
      [self startCamera];
    });
  });
}

- (void)startCamera {
  [self writeMarkerWithStatus:@"camera-enter" width:0 height:0 red:0 green:0 blue:0];
  const char *shmName = getenv("SIMDECK_CAMERA_SHM_NAME");
  [self writeMarkerWithStatus:(shmName && shmName[0] != '\\0') ? @"env-present" : @"env-missing" width:0 height:0 red:0 green:0 blue:0];
  AVCaptureDevice *device = [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeVideo];
  if (!device) {
    [self writeMarkerWithStatus:@"no-device" width:0 height:0 red:0 green:0 blue:0];
    dispatch_async(dispatch_get_main_queue(), ^{
      self.statusLabel.text = @"No Camera Device";
    });
    return;
  }
  NSString *deviceStatus = [device.localizedName isEqualToString:@"SimDeck Camera"] ? @"device-simdeck" : @"device-other";
  [self writeMarkerWithStatus:deviceStatus width:0 height:0 red:0 green:0 blue:0];
  NSError *error = nil;
  AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
  if (!input) {
    [self writeMarkerWithStatus:error.localizedDescription ?: @"input-error" width:0 height:0 red:0 green:0 blue:0];
    dispatch_async(dispatch_get_main_queue(), ^{
      self.statusLabel.text = @"Camera Input Failed";
    });
    return;
  }
  [self writeMarkerWithStatus:@"input" width:0 height:0 red:0 green:0 blue:0];
  AVCaptureVideoDataOutput *output = [[AVCaptureVideoDataOutput alloc] init];
  NSString *outputStatus = [NSString stringWithFormat:@"output-%@", NSStringFromClass(object_getClass(output))];
  [self writeMarkerWithStatus:outputStatus width:0 height:0 red:0 green:0 blue:0];
  dispatch_queue_t sampleQueue = dispatch_queue_create("dev.nativescript.simdeck.camera.fixture", DISPATCH_QUEUE_SERIAL);
  [self writeMarkerWithStatus:@"queue" width:0 height:0 red:0 green:0 blue:0];
  [output setSampleBufferDelegate:self queue:sampleQueue];
  [self writeMarkerWithStatus:@"output" width:0 height:0 red:0 green:0 blue:0];
  self.session = [[AVCaptureSession alloc] init];
  self.session.sessionPreset = AVCaptureSessionPreset1280x720;
  [self writeMarkerWithStatus:@"session" width:0 height:0 red:0 green:0 blue:0];
  if (![self.session canAddInput:input] || ![self.session canAddOutput:output]) {
    [self writeMarkerWithStatus:@"cannot-add-io" width:0 height:0 red:0 green:0 blue:0];
    dispatch_async(dispatch_get_main_queue(), ^{
      self.statusLabel.text = @"Camera Session Failed";
    });
    return;
  }
  [self.session addInput:input];
  [self.session addOutput:output];
  [self writeMarkerWithStatus:@"starting" width:0 height:0 red:0 green:0 blue:0];
  [self.session startRunning];
  [self writeMarkerWithStatus:@"started" width:0 height:0 red:0 green:0 blue:0];
}

- (void)captureOutput:(AVCaptureOutput *)output
 didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
       fromConnection:(AVCaptureConnection *)connection {
  (void)output;
  (void)connection;
  CVPixelBufferRef pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer);
  if (!pixelBuffer) return;
  CVPixelBufferLockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
  uint8_t *base = (uint8_t *)CVPixelBufferGetBaseAddress(pixelBuffer);
  size_t width = CVPixelBufferGetWidth(pixelBuffer);
  size_t height = CVPixelBufferGetHeight(pixelBuffer);
  size_t bytesPerRow = CVPixelBufferGetBytesPerRow(pixelBuffer);
  double red = 0;
  double green = 0;
  double blue = 0;
  NSInteger samples = 0;
  size_t yStep = MAX((size_t)1, height / 24);
  size_t xStep = MAX((size_t)1, width / 24);
  for (size_t y = 0; y < height; y += yStep) {
    uint8_t *row = base + y * bytesPerRow;
    for (size_t x = 0; x < width; x += xStep) {
      uint8_t *pixel = row + x * 4;
      blue += pixel[0];
      green += pixel[1];
      red += pixel[2];
      samples += 1;
    }
  }
  CVPixelBufferUnlockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
  if (samples > 0) {
    red /= samples;
    green /= samples;
    blue /= samples;
  }
  self.frames += 1;
  [self writeMarkerWithStatus:@"frame" width:width height:height red:red green:green blue:blue];
  if (self.frames % 10 == 0) {
    dispatch_async(dispatch_get_main_queue(), ^{
      self.statusLabel.text = [NSString stringWithFormat:@"Camera Frame %ld", (long)self.frames];
    });
  }
}

- (void)writeMarkerWithStatus:(NSString *)status
                        width:(size_t)width
                       height:(size_t)height
                          red:(double)red
                        green:(double)green
                         blue:(double)blue {
  NSString *directory = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES).firstObject;
  [[NSFileManager defaultManager] createDirectoryAtPath:directory withIntermediateDirectories:YES attributes:nil error:nil];
  NSString *path = [directory stringByAppendingPathComponent:@"camera-frame.json"];
  NSString *payload = [NSString stringWithFormat:
    @"{\\"status\\":\\"%@\\",\\"frames\\":%ld,\\"width\\":%zu,\\"height\\":%zu,\\"avgRed\\":%.3f,\\"avgGreen\\":%.3f,\\"avgBlue\\":%.3f}",
    status ?: @"unknown",
    (long)self.frames,
    width,
    height,
    red,
    green,
    blue];
  [payload writeToFile:path atomically:YES encoding:NSUTF8StringEncoding error:nil];
}

@end

@interface SceneDelegate : UIResponder <UIWindowSceneDelegate>
@property (nonatomic, strong) UIWindow *window;
@end

@implementation SceneDelegate

- (void)startCamera {
  CameraViewController *controller = (CameraViewController *)self.window.rootViewController;
  dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
    [controller startCamera];
  });
}

- (void)scene:(UIScene *)scene
willConnectToSession:(UISceneSession *)session
      options:(UISceneConnectionOptions *)connectionOptions {
  (void)session;
  (void)connectionOptions;
  UIWindowScene *windowScene = (UIWindowScene *)scene;
  self.window = [[UIWindow alloc] initWithWindowScene:windowScene];
  self.window.rootViewController = [[CameraViewController alloc] init];
  [self.window makeKeyAndVisible];
  if (connectionOptions.URLContexts.count > 0) {
    [self startCamera];
  }
}

- (void)scene:(UIScene *)scene openURLContexts:(NSSet<UIOpenURLContext *> *)URLContexts {
  (void)scene;
  if (URLContexts.count > 0) {
    [self startCamera];
  }
}

@end

@interface AppDelegate : UIResponder <UIApplicationDelegate>
@end

@implementation AppDelegate

- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
  (void)application;
  (void)launchOptions;
  return YES;
}

@end

int main(int argc, char *argv[]) {
  @autoreleasepool {
    return UIApplicationMain(argc, argv, nil, NSStringFromClass(AppDelegate.class));
  }
}
`;
}

async function waitForMarker(label, predicate) {
  const deadline = Date.now() + 45_000;
  let lastMarker = null;
  while (Date.now() < deadline) {
    const marker = readMarker();
    if (marker) {
      lastMarker = marker;
      if (predicate(marker)) {
        return marker;
      }
    }
    await sleep(500);
  }
  throw new Error(
    `Timed out waiting for ${label}. Last marker: ${JSON.stringify(lastMarker)}`,
  );
}

function readMarker() {
  let container = "";
  try {
    container = runText(
      "xcrun",
      ["simctl", "get_app_container", simulatorUDID, bundleId, "data"],
      { timeoutMs: 30_000 },
    ).trim();
  } catch {
    return null;
  }
  const markerPath = path.join(container, "Documents", "camera-frame.json");
  if (!fs.existsSync(markerPath)) {
    return null;
  }
  try {
    return JSON.parse(fs.readFileSync(markerPath, "utf8"));
  } catch {
    return null;
  }
}

function assertCameraStatus(status, source) {
  if (status.ok !== true || status.alive !== true || status.source !== source) {
    throw new Error(
      `unexpected camera status for ${source}: ${JSON.stringify(status)}`,
    );
  }
  if (status.width !== 1280 || status.height !== 720) {
    throw new Error(`unexpected camera dimensions: ${JSON.stringify(status)}`);
  }
}

function writeSolidBmp(filePath, width, height, color) {
  const rowStride = Math.ceil((width * 3) / 4) * 4;
  const pixelOffset = 54;
  const fileSize = pixelOffset + rowStride * height;
  const buffer = Buffer.alloc(fileSize);
  buffer.write("BM", 0, "ascii");
  buffer.writeUInt32LE(fileSize, 2);
  buffer.writeUInt32LE(pixelOffset, 10);
  buffer.writeUInt32LE(40, 14);
  buffer.writeInt32LE(width, 18);
  buffer.writeInt32LE(height, 22);
  buffer.writeUInt16LE(1, 26);
  buffer.writeUInt16LE(24, 28);
  buffer.writeUInt32LE(rowStride * height, 34);
  for (let y = 0; y < height; y += 1) {
    const row = pixelOffset + y * rowStride;
    for (let x = 0; x < width; x += 1) {
      const offset = row + x * 3;
      buffer[offset] = color.b;
      buffer[offset + 1] = color.g;
      buffer[offset + 2] = color.r;
    }
  }
  fs.writeFileSync(filePath, buffer);
}

function simdeckJson(args, options = {}) {
  return JSON.parse(
    runText(simdeck, args, { timeoutMs: commandTimeoutMs, ...options }),
  );
}

function runJson(command, args, options = {}) {
  return JSON.parse(runText(command, args, options));
}

function runText(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    timeout: options.timeoutMs ?? commandTimeoutMs,
    maxBuffer: 1024 * 1024 * 8,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0 && !options.allowFailure) {
    throw new Error(
      `${command} ${args.join(" ")} failed with status ${result.status}: ${[
        result.stderr,
        result.stdout,
      ]
        .filter(Boolean)
        .join("\n")}`,
    );
  }
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  if (verbose && output.trim()) {
    process.stderr.write(output);
  }
  return result.stdout ?? "";
}

function cleanup() {
  if (simulatorUDID) {
    try {
      simdeckJson(["camera", "stop", simulatorUDID], { timeoutMs: 30_000 });
    } catch {}
    if (!keepSimulator) {
      try {
        runText("xcrun", ["simctl", "shutdown", simulatorUDID], {
          allowFailure: true,
          timeoutMs: 120_000,
        });
      } catch {}
      try {
        runText("xcrun", ["simctl", "delete", simulatorUDID], {
          allowFailure: true,
          timeoutMs: 120_000,
        });
      } catch {}
    }
  }
  if (!keepSimulator) {
    try {
      fs.rmSync(tempRoot, { recursive: true, force: true });
    } catch {}
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function step(label) {
  console.log(`[camera-it] ${label}`);
}
