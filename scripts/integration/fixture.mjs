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
  const clangVersion = commandOutput("xcrun", [
    "--sdk",
    "iphonesimulator",
    "clang",
    "--version",
  ]);
  const plist = fixtureInfoPlist(bundleId, urlScheme);
  const source = fixtureSource();
  const fingerprint = crypto
    .createHash("sha256")
    .update(
      JSON.stringify({ targetArch, sdkVersion, clangVersion, plist, source }),
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
    log(`building cached UIKit fixture ${fingerprint}`);
    buildFixtureIntoCache({
      cacheRoot,
      cachedAppPath,
      plist,
      source,
      targetArch,
    });
  } else {
    log(`using cached UIKit fixture ${fingerprint}`);
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
  const sourcePath = path.join(stagingRoot, `${executable}.m`);
  fs.writeFileSync(plistPath, plist);
  fs.writeFileSync(sourcePath, source);

  run("xcrun", [
    "--sdk",
    "iphonesimulator",
    "clang",
    "-target",
    `${targetArch}-apple-ios${minimumIosVersion}-simulator`,
    "-fobjc-arc",
    "-fmodules",
    "-framework",
    "UIKit",
    "-framework",
    "Foundation",
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

function fixtureSource() {
  return `#import <QuartzCore/QuartzCore.h>
#import <UIKit/UIKit.h>

@interface FixtureViewController : UIViewController <UITextFieldDelegate>
@property (nonatomic, strong) UILabel *statusLabel;
@property (nonatomic, strong) UITextField *messageField;
@property (nonatomic, strong) UIView *animationBar;
@property (nonatomic, strong) CADisplayLink *displayLink;
@property (nonatomic) NSInteger tapCount;
@property (nonatomic) CFTimeInterval animationStartedAt;
- (void)openFixtureURL:(NSURL *)url;
@end

@implementation FixtureViewController

- (void)viewDidLoad {
  [super viewDidLoad];
  self.view.backgroundColor = UIColor.systemBackgroundColor;

  UILabel *titleLabel = [[UILabel alloc] init];
  titleLabel.text = @"SimDeck Fixture";
  titleLabel.font = [UIFont preferredFontForTextStyle:UIFontTextStyleTitle2];
  titleLabel.textAlignment = NSTextAlignmentCenter;
  titleLabel.accessibilityIdentifier = @"fixture.title";

  self.statusLabel = [[UILabel alloc] init];
  self.statusLabel.text = @"Integration Ready";
  self.statusLabel.font = [UIFont preferredFontForTextStyle:UIFontTextStyleBody];
  self.statusLabel.textAlignment = NSTextAlignmentCenter;
  self.statusLabel.accessibilityIdentifier = @"fixture.status";

  UIButton *continueButton = [UIButton buttonWithType:UIButtonTypeSystem];
  [continueButton setTitle:@"Continue" forState:UIControlStateNormal];
  continueButton.titleLabel.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline];
  continueButton.accessibilityIdentifier = @"fixture.continue";
  [continueButton addTarget:self action:@selector(continueTapped:) forControlEvents:UIControlEventTouchUpInside];

  self.messageField = [[UITextField alloc] init];
  self.messageField.placeholder = @"Message";
  self.messageField.borderStyle = UITextBorderStyleRoundedRect;
  self.messageField.autocapitalizationType = UITextAutocapitalizationTypeNone;
  self.messageField.autocorrectionType = UITextAutocorrectionTypeNo;
  self.messageField.accessibilityIdentifier = @"fixture.message";
  self.messageField.delegate = self;
  [self.messageField addTarget:self action:@selector(messageChanged:) forControlEvents:UIControlEventEditingChanged];
  [self.messageField.widthAnchor constraintEqualToConstant:240.0].active = YES;

  self.animationBar = [[UIView alloc] initWithFrame:CGRectZero];
  self.animationBar.backgroundColor = UIColor.systemBlueColor;
  self.animationBar.layer.cornerRadius = 6.0;
  self.animationBar.accessibilityIdentifier = @"fixture.animation";
  self.animationBar.translatesAutoresizingMaskIntoConstraints = NO;

  UIStackView *stack = [[UIStackView alloc] initWithArrangedSubviews:@[
    titleLabel,
    self.statusLabel,
    continueButton,
    self.messageField,
    self.animationBar,
  ]];
  stack.axis = UILayoutConstraintAxisVertical;
  stack.alignment = UIStackViewAlignmentCenter;
  stack.spacing = 24.0;
  stack.translatesAutoresizingMaskIntoConstraints = NO;
  [self.view addSubview:stack];

  [NSLayoutConstraint activateConstraints:@[
    [stack.centerXAnchor constraintEqualToAnchor:self.view.centerXAnchor],
    [stack.centerYAnchor constraintEqualToAnchor:self.view.centerYAnchor],
    [stack.leadingAnchor constraintGreaterThanOrEqualToAnchor:self.view.safeAreaLayoutGuide.leadingAnchor constant:24.0],
    [stack.trailingAnchor constraintLessThanOrEqualToAnchor:self.view.safeAreaLayoutGuide.trailingAnchor constant:-24.0],
    [self.animationBar.widthAnchor constraintEqualToConstant:220.0],
    [self.animationBar.heightAnchor constraintEqualToConstant:12.0],
  ]];
}

- (void)continueTapped:(id)sender {
  self.tapCount += 1;
  self.statusLabel.text = [NSString stringWithFormat:@"Continue Tapped %ld", (long)self.tapCount];
}

- (void)messageChanged:(UITextField *)sender {
  self.statusLabel.text = sender.text ?: @"";
}

- (void)startAnimation {
  if (self.displayLink) {
    return;
  }
  self.animationStartedAt = CACurrentMediaTime();
  self.statusLabel.text = @"Animating";
  self.displayLink = [CADisplayLink displayLinkWithTarget:self selector:@selector(animationTick:)];
  if (@available(iOS 15.0, *)) {
    self.displayLink.preferredFrameRateRange = CAFrameRateRangeMake(60.0, 60.0, 60.0);
  } else {
    self.displayLink.preferredFramesPerSecond = 60;
  }
  [self.displayLink addToRunLoop:NSRunLoop.mainRunLoop forMode:NSRunLoopCommonModes];
}

- (void)animationTick:(CADisplayLink *)displayLink {
  CFTimeInterval elapsed = CACurrentMediaTime() - self.animationStartedAt;
  CGFloat phase = (sin(elapsed * 4.0) + 1.0) * 0.5;
  CGFloat x = -80.0 + phase * 160.0;
  self.animationBar.transform = CGAffineTransformMakeTranslation(x, 0.0);
  self.animationBar.backgroundColor = [UIColor colorWithHue:fmod(elapsed * 0.18, 1.0)
                                                 saturation:0.85
                                                 brightness:0.92
                                                      alpha:1.0];
}

- (void)openFixtureURL:(NSURL *)url {
  if ([url.host isEqualToString:@"animate"]) {
    [self startAnimation];
  } else if ([url.host isEqualToString:@"focus-message"]) {
    dispatch_async(dispatch_get_main_queue(), ^{
      [self.messageField becomeFirstResponder];
      dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(500 * NSEC_PER_MSEC)), dispatch_get_main_queue(), ^{
        if (self.messageField.isFirstResponder) {
          self.statusLabel.text = @"Message Focused";
        } else {
          self.statusLabel.text = @"Message Focus Failed";
        }
      });
    });
  } else {
    self.statusLabel.text = @"URL Opened";
  }
}

- (BOOL)textFieldShouldReturn:(UITextField *)textField {
  [textField resignFirstResponder];
  return YES;
}

@end

@interface AppDelegate : UIResponder <UIApplicationDelegate>
@property (nonatomic, strong) UIWindow *window;
@property (nonatomic, strong) FixtureViewController *fixtureViewController;
@end

@implementation AppDelegate

- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
  self.fixtureViewController = [[FixtureViewController alloc] init];
  self.window = [[UIWindow alloc] initWithFrame:UIScreen.mainScreen.bounds];
  self.window.rootViewController = self.fixtureViewController;
  [self.window makeKeyAndVisible];

  NSURL *launchURL = launchOptions[UIApplicationLaunchOptionsURLKey];
  if ([launchURL isKindOfClass:NSURL.class]) {
    [self.fixtureViewController openFixtureURL:launchURL];
  }
  return YES;
}

- (BOOL)application:(UIApplication *)application openURL:(NSURL *)url options:(NSDictionary<UIApplicationOpenURLOptionsKey, id> *)options {
  [self.fixtureViewController openFixtureURL:url];
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
