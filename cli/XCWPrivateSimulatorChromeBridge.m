#import "XCWPrivateSimulatorChromeBridge.h"

#import <objc/message.h>

@implementation XCWPrivateSimulatorChromeBridge

+ (nullable NSView *)chromeViewForDeviceName:(NSString *)deviceName
                                 displaySize:(CGSize)displaySize {
    NSBundle *simulatorKitBundle = [NSBundle bundleWithPath:@"/Applications/Xcode.app/Contents/Developer/Library/PrivateFrameworks/SimulatorKit.framework"];
    if (![simulatorKitBundle isLoaded] && ![simulatorKitBundle load]) {
        NSLog(@"[XCW] Failed to load SimulatorKit");
        return nil;
    }

    NSBundle *coreSimulatorBundle = [NSBundle bundleWithPath:@"/Library/Developer/PrivateFrameworks/CoreSimulator.framework"];
    if (![coreSimulatorBundle isLoaded] && ![coreSimulatorBundle load]) {
        NSLog(@"[XCW] Failed to load CoreSimulator");
        return nil;
    }

    id deviceType = [self deviceTypeForName:deviceName];
    if (deviceType == nil) {
        NSLog(@"[XCW] No device type found for '%@'", deviceName);
        return nil;
    }

    Class chromeViewClass = NSClassFromString(@"_TtC12SimulatorKit20SimDisplayChromeView");
    if (chromeViewClass == Nil) {
        NSLog(@"[XCW] SimDisplayChromeView class not found");
        return nil;
    }

    NSView *chromeView = [[chromeViewClass alloc] initWithFrame:NSZeroRect];
    if (chromeView == nil) {
        return nil;
    }

    @try {
        [chromeView setValue:deviceType forKey:@"deviceType"];
        [chromeView setValue:[NSValue valueWithSize:displaySize] forKey:@"displaySize"];
        [chromeView setValue:@YES forKey:@"preferSimpleChrome"];
    } @catch (NSException *exception) {
        NSLog(@"[XCW] Failed to configure chrome view: %@", exception);
        return nil;
    }

    return chromeView;
}

+ (nullable id)deviceTypeForName:(NSString *)deviceName {
    NSString *basePath = @"/Library/Developer/CoreSimulator/Profiles/DeviceTypes";
    NSString *bundlePath = [NSString stringWithFormat:@"%@/%@.simdevicetype", basePath, deviceName];

    Class simDeviceTypeClass = NSClassFromString(@"SimDeviceType");
    if (simDeviceTypeClass == Nil) {
        NSLog(@"[XCW] SimDeviceType class not found");
        return nil;
    }

    NSBundle *deviceBundle = [NSBundle bundleWithPath:bundlePath];
    if (deviceBundle == nil) {
        NSLog(@"[XCW] Device type bundle not found at '%@'", bundlePath);
        return nil;
    }

    typedef id (*InitWithBundleIMP)(id, SEL, NSBundle *, NSError **);

    SEL initSelector = NSSelectorFromString(@"initWithBundle:error:");
    if (![simDeviceTypeClass instancesRespondToSelector:initSelector]) {
        NSLog(@"[XCW] SimDeviceType does not respond to initWithBundle:error:");
        return nil;
    }

    id instance = [simDeviceTypeClass alloc];
    NSError *error = nil;

    InitWithBundleIMP implementation = (InitWithBundleIMP)objc_msgSend;
    id deviceType = implementation(instance, initSelector, deviceBundle, &error);
    if (error != nil) {
        NSLog(@"[XCW] Failed to init device type for '%@': %@", deviceName, error);
        return nil;
    }

    return deviceType;
}

@end
