#import "XCWSimctl.h"

#import <AppKit/AppKit.h>

#import "XCWPrivateSimulatorBooter.h"
#import "XCWProcessRunner.h"

static NSString * const XCWSimctlErrorDomain = @"XcodeCanvasWeb.Simctl";

@implementation XCWSimctl

- (nullable NSArray<NSDictionary *> *)listSimulatorsWithError:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self.class runSimctl:@[@"list", @"--json"] error:error];
    if (result == nil) {
        return nil;
    }
    if (result.terminationStatus != 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"simctl list failed" code:1];
        }
        return nil;
    }

    NSDictionary *payload = [NSJSONSerialization JSONObjectWithData:result.stdoutData options:0 error:error];
    if (![payload isKindOfClass:[NSDictionary class]]) {
        if (error != NULL && *error == nil) {
            *error = [self.class errorWithDescription:@"Unable to parse simctl JSON output." code:2];
        }
        return nil;
    }

    NSDictionary *deviceTypesPayload = payload[@"devicetypes"];
    NSArray *deviceTypesArray = [deviceTypesPayload isKindOfClass:[NSDictionary class]] ? deviceTypesPayload[@"devicetypes"] : nil;
    if (![deviceTypesArray isKindOfClass:[NSArray class]]) {
        deviceTypesArray = @[];
    }
    NSMutableDictionary<NSString *, NSDictionary *> *deviceTypesByIdentifier = [NSMutableDictionary dictionary];
    for (NSDictionary *deviceType in deviceTypesArray) {
        NSString *identifier = deviceType[@"identifier"];
        if (identifier.length > 0) {
            deviceTypesByIdentifier[identifier] = deviceType;
        }
    }

    NSDictionary *runtimesPayload = payload[@"runtimes"];
    NSArray *runtimeArray = [runtimesPayload isKindOfClass:[NSDictionary class]] ? runtimesPayload[@"runtimes"] : nil;
    if (![runtimeArray isKindOfClass:[NSArray class]]) {
        runtimeArray = @[];
    }
    NSMutableDictionary<NSString *, NSDictionary *> *runtimesByIdentifier = [NSMutableDictionary dictionary];
    for (NSDictionary *runtime in runtimeArray) {
        NSString *identifier = runtime[@"identifier"];
        if (identifier.length > 0) {
            runtimesByIdentifier[identifier] = runtime;
        }
    }

    NSMutableArray<NSDictionary *> *flattened = [NSMutableArray array];
    NSDictionary *devicesByRuntime = payload[@"devices"];
    [devicesByRuntime enumerateKeysAndObjectsUsingBlock:^(NSString *runtimeIdentifier, NSArray *devices, __unused BOOL *stop) {
        if (![devices isKindOfClass:[NSArray class]]) {
            return;
        }

        NSDictionary *runtime = runtimesByIdentifier[runtimeIdentifier] ?: @{};
        for (NSDictionary *device in devices) {
            if (![device isKindOfClass:[NSDictionary class]]) {
                continue;
            }

            NSString *udid = device[@"udid"] ?: @"";
            NSString *deviceTypeIdentifier = device[@"deviceTypeIdentifier"] ?: @"";
            NSDictionary *deviceType = deviceTypesByIdentifier[deviceTypeIdentifier] ?: @{};
            NSString *state = device[@"state"] ?: @"Unknown";
            BOOL isAvailable = [device[@"isAvailable"] respondsToSelector:@selector(boolValue)] ? [device[@"isAvailable"] boolValue] : YES;

            [flattened addObject:@{
                @"udid": udid,
                @"name": device[@"name"] ?: @"Unknown Simulator",
                @"state": state,
                @"isBooted": @([state caseInsensitiveCompare:@"Booted"] == NSOrderedSame),
                @"isAvailable": @(isAvailable),
                @"lastBootedAt": device[@"lastBootedAt"] ?: [NSNull null],
                @"dataPath": device[@"dataPath"] ?: [NSNull null],
                @"logPath": device[@"logPath"] ?: [NSNull null],
                @"deviceTypeIdentifier": deviceTypeIdentifier.length > 0 ? deviceTypeIdentifier : [NSNull null],
                @"deviceTypeName": deviceType[@"name"] ?: device[@"name"] ?: @"Unknown Simulator",
                @"runtimeIdentifier": runtimeIdentifier ?: [NSNull null],
                @"runtimeName": runtime[@"name"] ?: runtimeIdentifier ?: @"Unknown Runtime",
            }];
        }
    }];

    [flattened sortUsingComparator:^NSComparisonResult(NSDictionary *lhs, NSDictionary *rhs) {
        NSNumber *lhsBooted = lhs[@"isBooted"];
        NSNumber *rhsBooted = rhs[@"isBooted"];
        if (lhsBooted.boolValue != rhsBooted.boolValue) {
            return lhsBooted.boolValue ? NSOrderedAscending : NSOrderedDescending;
        }

        NSString *lhsRuntime = lhs[@"runtimeName"] ?: @"";
        NSString *rhsRuntime = rhs[@"runtimeName"] ?: @"";
        NSComparisonResult runtimeOrder = [rhsRuntime localizedStandardCompare:lhsRuntime];
        if (runtimeOrder != NSOrderedSame) {
            return runtimeOrder;
        }

        NSString *lhsName = lhs[@"name"] ?: @"";
        NSString *rhsName = rhs[@"name"] ?: @"";
        return [lhsName localizedStandardCompare:rhsName];
    }];

    return flattened;
}

- (nullable NSDictionary *)simulatorWithUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    for (NSDictionary *simulator in [self listSimulatorsWithError:error] ?: @[]) {
        if ([simulator[@"udid"] isEqualToString:udid]) {
            return simulator;
        }
    }
    if (error != NULL && *error == nil) {
        *error = [self.class errorWithDescription:[NSString stringWithFormat:@"Unknown simulator %@", udid] code:3];
    }
    return nil;
}

- (BOOL)bootSimulatorWithUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    NSError *privateError = nil;
    if ([XCWPrivateSimulatorBooter bootDeviceWithUDID:udid error:&privateError]) {
        return YES;
    }

    XCWProcessResult *result = [self.class runSimctl:@[@"boot", udid] error:error];
    if (result == nil) {
        return NO;
    }
    if (result.terminationStatus == 0) {
        return YES;
    }

    NSString *stderrString = result.stderrString.lowercaseString;
    if ([stderrString containsString:@"unable to boot device in current state: booted"] || [stderrString containsString:@"already booted"]) {
        return YES;
    }

    if (error != NULL) {
        NSString *description = result.stderrString.length > 0 ? result.stderrString : privateError.localizedDescription ?: @"Unable to boot simulator.";
        *error = [self.class errorWithDescription:description code:4];
    }
    return NO;
}

- (BOOL)shutdownSimulatorWithUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self.class runSimctl:@[@"shutdown", udid] error:error];
    if (result == nil) {
        return NO;
    }
    if (result.terminationStatus == 0) {
        return YES;
    }

    NSString *stderrString = result.stderrString.lowercaseString;
    if ([stderrString containsString:@"shutdown commands can only be sent to booted devices"]) {
        return YES;
    }

    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to shut down simulator." code:5];
    }
    return NO;
}

- (BOOL)openURL:(NSString *)urlString simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self.class runSimctl:@[@"openurl", udid, urlString] error:error];
    if (result == nil) {
        return NO;
    }
    if (result.terminationStatus == 0) {
        return YES;
    }
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to open URL in simulator." code:6];
    }
    return NO;
}

- (BOOL)launchBundleID:(NSString *)bundleID simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self.class runSimctl:@[@"launch", udid, bundleID] error:error];
    if (result == nil) {
        return NO;
    }
    if (result.terminationStatus == 0) {
        return YES;
    }
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to launch app in simulator." code:7];
    }
    return NO;
}

- (nullable NSData *)screenshotJPEGDataForSimulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    NSString *temporaryPath = [NSTemporaryDirectory() stringByAppendingPathComponent:[NSString stringWithFormat:@"xcode-canvas-web-%@.jpg", NSUUID.UUID.UUIDString]];
    XCWProcessResult *result = [self.class runSimctl:@[@"io", udid, @"screenshot", @"--type=jpeg", temporaryPath] error:error];
    if (result == nil) {
        return nil;
    }
    if (result.terminationStatus != 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to capture simulator screenshot." code:8];
        }
        return nil;
    }

    NSData *imageData = [NSData dataWithContentsOfFile:temporaryPath options:0 error:error];
    [[NSFileManager defaultManager] removeItemAtPath:temporaryPath error:nil];
    if (imageData.length == 0) {
        if (error != NULL && *error == nil) {
            *error = [self.class errorWithDescription:@"simctl completed without producing a screenshot file." code:9];
        }
        return nil;
    }
    return imageData;
}

+ (nullable XCWProcessResult *)runSimctl:(NSArray<NSString *> *)arguments
                                   error:(NSError * _Nullable __autoreleasing *)error {
    return [XCWProcessRunner runLaunchPath:@"/usr/bin/xcrun"
                                 arguments:[@[@"simctl"] arrayByAddingObjectsFromArray:arguments]
                                 inputData:nil
                                     error:error];
}

+ (NSError *)errorWithDescription:(NSString *)description code:(NSInteger)code {
    return [NSError errorWithDomain:XCWSimctlErrorDomain
                               code:code
                           userInfo:@{
        NSLocalizedDescriptionKey: description,
    }];
}

@end
