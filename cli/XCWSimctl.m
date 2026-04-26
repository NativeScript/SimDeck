#import "XCWSimctl.h"

#import <AppKit/AppKit.h>

#import "XCWPrivateSimulatorBooter.h"
#import "XCWProcessRunner.h"

static NSString * const XCWSimctlErrorDomain = @"SimDeck.Simctl";

static NSArray *XCWArrayPayload(id payload, NSString *nestedKey) {
    if ([payload isKindOfClass:[NSArray class]]) {
        return payload;
    }
    if ([payload isKindOfClass:[NSDictionary class]]) {
        id nested = [(NSDictionary *)payload objectForKey:nestedKey];
        if ([nested isKindOfClass:[NSArray class]]) {
            return nested;
        }
    }
    return @[];
}

static NSString *XCWStringValue(id value) {
    return [value isKindOfClass:[NSString class]] ? value : @"";
}

static NSString *XCWRuntimeDisplayName(NSDictionary *runtime, NSString *runtimeIdentifier) {
    NSString *name = XCWStringValue(runtime[@"name"]);
    if (name.length > 0 && ![name isEqualToString:runtimeIdentifier]) {
        return name;
    }

    NSString *platform = XCWStringValue(runtime[@"platform"]);
    NSString *version = XCWStringValue(runtime[@"version"]);
    if (platform.length > 0 && version.length > 0) {
        return [NSString stringWithFormat:@"%@ %@", platform, version];
    }

    NSString *prefix = @"com.apple.CoreSimulator.SimRuntime.";
    NSString *suffix = runtimeIdentifier ?: @"";
    if ([suffix hasPrefix:prefix]) {
        suffix = [suffix substringFromIndex:prefix.length];
    }
    for (NSString *candidate in @[@"iOS", @"watchOS", @"tvOS", @"visionOS", @"xrOS"]) {
        NSString *candidatePrefix = [candidate stringByAppendingString:@"-"];
        if ([suffix hasPrefix:candidatePrefix]) {
            NSString *versionSuffix = [suffix substringFromIndex:candidatePrefix.length];
            return [NSString stringWithFormat:@"%@ %@", candidate, [versionSuffix stringByReplacingOccurrencesOfString:@"-" withString:@"."]];
        }
    }
    return runtimeIdentifier.length > 0 ? runtimeIdentifier : @"Unknown Runtime";
}

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

    NSArray *deviceTypesArray = XCWArrayPayload(payload[@"devicetypes"], @"devicetypes");
    NSMutableDictionary<NSString *, NSDictionary *> *deviceTypesByIdentifier = [NSMutableDictionary dictionary];
    for (NSDictionary *deviceType in deviceTypesArray) {
        if (![deviceType isKindOfClass:[NSDictionary class]]) {
            continue;
        }
        NSString *identifier = deviceType[@"identifier"];
        if (identifier.length > 0) {
            deviceTypesByIdentifier[identifier] = deviceType;
        }
    }

    NSArray *runtimeArray = XCWArrayPayload(payload[@"runtimes"], @"runtimes");
    NSMutableDictionary<NSString *, NSDictionary *> *runtimesByIdentifier = [NSMutableDictionary dictionary];
    for (NSDictionary *runtime in runtimeArray) {
        if (![runtime isKindOfClass:[NSDictionary class]]) {
            continue;
        }
        NSString *identifier = runtime[@"identifier"];
        if (identifier.length > 0) {
            runtimesByIdentifier[identifier] = runtime;
        }
    }

    NSMutableArray<NSDictionary *> *flattened = [NSMutableArray array];
    NSDictionary *devicesByRuntime = payload[@"devices"];
    if (![devicesByRuntime isKindOfClass:[NSDictionary class]]) {
        devicesByRuntime = @{};
    }
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
                @"runtimeName": XCWRuntimeDisplayName(runtime, runtimeIdentifier),
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

- (BOOL)toggleAppearanceForSimulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *queryResult = [self.class runSimctl:@[@"ui", udid, @"appearance"] error:error];
    if (queryResult == nil) {
        return NO;
    }
    if (queryResult.terminationStatus != 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:queryResult.stderrString.length > 0 ? queryResult.stderrString : @"Unable to read simulator appearance." code:10];
        }
        return NO;
    }

    NSString *currentAppearance = [queryResult.stdoutString stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet].lowercaseString;
    if ([currentAppearance isEqualToString:@"unsupported"]) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:@"This simulator runtime does not support appearance switching." code:11];
        }
        return NO;
    }

    NSString *nextAppearance = [currentAppearance isEqualToString:@"dark"] ? @"light" : @"dark";
    XCWProcessResult *setResult = [self.class runSimctl:@[@"ui", udid, @"appearance", nextAppearance] error:error];
    if (setResult == nil) {
        return NO;
    }
    if (setResult.terminationStatus == 0) {
        return YES;
    }

    if (error != NULL) {
        *error = [self.class errorWithDescription:setResult.stderrString.length > 0 ? setResult.stderrString : @"Unable to set simulator appearance." code:12];
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
    XCWProcessResult *result = [self.class runSimctl:@[@"launch", @"--stdout=/dev/null", @"--stderr=/dev/null", udid, bundleID] error:error];
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

- (nullable NSData *)screenshotPNGForSimulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    NSString *filename = [NSString stringWithFormat:@"simdeck-%@.png", NSUUID.UUID.UUIDString];
    NSString *path = [NSTemporaryDirectory() stringByAppendingPathComponent:filename];
    XCWProcessResult *result = [self.class runSimctl:@[@"io", udid, @"screenshot", @"--type=png", path] error:error];
    if (result == nil) {
        return nil;
    }
    if (result.terminationStatus == 0) {
        NSData *data = [NSData dataWithContentsOfFile:path options:0 error:error];
        [[NSFileManager defaultManager] removeItemAtPath:path error:nil];
        if (data.length > 0) {
            return data;
        }
        if (error != NULL && *error == nil) {
            *error = [self.class errorWithDescription:@"Simulator screenshot command produced an empty PNG." code:13];
        }
        return nil;
    }
    [[NSFileManager defaultManager] removeItemAtPath:path error:nil];
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to capture simulator screenshot." code:13];
    }
    return nil;
}

- (BOOL)eraseSimulatorWithUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self.class runSimctl:@[@"erase", udid] error:error];
    if (result == nil) {
        return NO;
    }
    if (result.terminationStatus == 0) {
        return YES;
    }
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to erase simulator." code:14];
    }
    return NO;
}

- (BOOL)installAppAtPath:(NSString *)appPath simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self.class runSimctl:@[@"install", udid, appPath] error:error];
    if (result == nil) {
        return NO;
    }
    if (result.terminationStatus == 0) {
        return YES;
    }
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to install app in simulator." code:15];
    }
    return NO;
}

- (BOOL)uninstallBundleID:(NSString *)bundleID simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self.class runSimctl:@[@"uninstall", udid, bundleID] error:error];
    if (result == nil) {
        return NO;
    }
    if (result.terminationStatus == 0) {
        return YES;
    }
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to uninstall app from simulator." code:16];
    }
    return NO;
}

- (BOOL)setPasteboardText:(NSString *)text simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    NSData *inputData = [text dataUsingEncoding:NSUTF8StringEncoding] ?: [NSData data];
    XCWProcessResult *result = [XCWProcessRunner runLaunchPath:@"/usr/bin/xcrun"
                                                     arguments:@[@"simctl", @"pbcopy", udid]
                                                     inputData:inputData
                                                         error:error];
    if (result == nil) {
        return NO;
    }
    if (result.terminationStatus == 0) {
        return YES;
    }
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to write simulator pasteboard." code:17];
    }
    return NO;
}

- (nullable NSString *)pasteboardTextForSimulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self.class runSimctl:@[@"pbpaste", udid] error:error];
    if (result == nil) {
        return nil;
    }
    if (result.terminationStatus == 0) {
        return result.stdoutString ?: @"";
    }
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to read simulator pasteboard." code:18];
    }
    return nil;
}

- (nullable NSArray<NSDictionary *> *)recentLogEntriesForSimulatorUDID:(NSString *)udid
                                                               seconds:(NSTimeInterval)seconds
                                                                 error:(NSError * _Nullable __autoreleasing *)error {
    NSUInteger boundedSeconds = MIN(MAX((NSUInteger)ceil(seconds), 1), 1800);
    XCWProcessResult *result = [self.class runSimctl:@[
        @"spawn",
        udid,
        @"log",
        @"show",
        @"--style",
        @"ndjson",
        @"--last",
        [NSString stringWithFormat:@"%lus", (unsigned long)boundedSeconds],
        @"--info",
        @"--debug"
    ] error:error];
    if (result == nil) {
        return nil;
    }
    if (result.terminationStatus != 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to read simulator logs." code:10];
        }
        return nil;
    }

    NSMutableArray<NSDictionary *> *entries = [NSMutableArray array];
    NSArray<NSString *> *lines = [result.stdoutString componentsSeparatedByCharactersInSet:NSCharacterSet.newlineCharacterSet];
    for (NSString *line in lines) {
        NSString *trimmed = [line stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
        if (![trimmed hasPrefix:@"{"]) {
            continue;
        }

        NSData *lineData = [trimmed dataUsingEncoding:NSUTF8StringEncoding];
        if (lineData == nil) {
            continue;
        }
        NSDictionary *payload = [NSJSONSerialization JSONObjectWithData:lineData options:0 error:nil];
        if (![payload isKindOfClass:NSDictionary.class]) {
            continue;
        }

        NSString *processPath = [payload[@"processImagePath"] isKindOfClass:NSString.class] ? payload[@"processImagePath"] : @"";
        NSString *processName = processPath.lastPathComponent.length > 0 ? processPath.lastPathComponent : @"unknown";
        [entries addObject:@{
            @"timestamp": payload[@"timestamp"] ?: @"",
            @"level": payload[@"messageType"] ?: @"Default",
            @"process": processName,
            @"pid": payload[@"processID"] ?: [NSNull null],
            @"subsystem": payload[@"subsystem"] ?: @"",
            @"category": payload[@"category"] ?: @"",
            @"message": payload[@"eventMessage"] ?: payload[@"formatString"] ?: @"",
        }];
    }

    return entries;
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
