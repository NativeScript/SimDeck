#import "XCWSimctl.h"

#import <AppKit/AppKit.h>

#import "XCWChromeRenderer.h"
#import "XCWPrivateSimulatorBooter.h"
#import "XCWProcessRunner.h"

#import <errno.h>
#import <math.h>
#import <signal.h>
#import <stdlib.h>
#import <string.h>

static NSString * const XCWSimctlErrorDomain = @"SimDeck.Simctl";

@interface XCWSimctl ()

- (nullable NSString *)createSingleSimulatorWithName:(NSString *)name
                                deviceTypeIdentifier:(NSString *)deviceTypeIdentifier
                                   runtimeIdentifier:(nullable NSString *)runtimeIdentifier
                                               error:(NSError * _Nullable __autoreleasing *)error;
+ (nullable XCWProcessResult *)runSimctl:(NSArray<NSString *> *)arguments
                                   error:(NSError * _Nullable __autoreleasing *)error;
+ (nullable XCWProcessResult *)runSimctl:(NSArray<NSString *> *)arguments
                              timeoutSec:(NSTimeInterval)timeoutSec
                                   error:(NSError * _Nullable __autoreleasing *)error;
+ (nullable XCWProcessResult *)runSimctl:(NSArray<NSString *> *)arguments
                              timeoutSec:(NSTimeInterval)timeoutSec
                           timeoutSignal:(int)timeoutSignal
                                   error:(NSError * _Nullable __autoreleasing *)error;
+ (nullable NSDictionary *)listJSONPayloadWithError:(NSError * _Nullable __autoreleasing *)error;
+ (NSError *)errorWithDescription:(NSString *)description code:(NSInteger)code;
- (BOOL)installAppBundleAtPath:(NSString *)appPath simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error;
- (BOOL)installIPAAtPath:(NSString *)ipaPath simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error;

@end

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

static NSNumber *XCWNumberValue(id value) {
    return [value isKindOfClass:[NSNumber class]] ? value : nil;
}

static NSString * _Nullable XCWCreateTemporaryDirectory(NSString *prefix, NSError * _Nullable __autoreleasing *error) {
    NSString *templatePath = [NSTemporaryDirectory() stringByAppendingPathComponent:[NSString stringWithFormat:@"%@-XXXXXX", prefix]];
    char *directoryTemplate = strdup(templatePath.fileSystemRepresentation);
    if (directoryTemplate == NULL) {
        if (error != NULL) {
            *error = [XCWSimctl errorWithDescription:@"Failed to allocate temporary IPA extraction path." code:15];
        }
        return nil;
    }

    char *createdPath = mkdtemp(directoryTemplate);
    if (createdPath == NULL) {
        if (error != NULL) {
            *error = [XCWSimctl errorWithDescription:[NSString stringWithFormat:@"Failed to create temporary IPA extraction directory: %s", strerror(errno)] code:15];
        }
        free(directoryTemplate);
        return nil;
    }

    NSString *path = [[NSFileManager defaultManager] stringWithFileSystemRepresentation:createdPath
                                                                                 length:strlen(createdPath)];
    free(directoryTemplate);
    return path;
}

static NSString * _Nullable XCWAppBundlePathInExtractedIPA(NSString *extractedPath, NSError * _Nullable __autoreleasing *error) {
    NSFileManager *fileManager = [NSFileManager defaultManager];
    NSString *payloadPath = [extractedPath stringByAppendingPathComponent:@"Payload"];
    BOOL isDirectory = NO;
    if (![fileManager fileExistsAtPath:payloadPath isDirectory:&isDirectory] || !isDirectory) {
        if (error != NULL) {
            *error = [XCWSimctl errorWithDescription:@"IPA archive did not contain a Payload directory." code:15];
        }
        return nil;
    }

    NSArray<NSString *> *entries = [fileManager contentsOfDirectoryAtPath:payloadPath error:error];
    if (entries == nil) {
        return nil;
    }

    NSMutableArray<NSString *> *appPaths = [NSMutableArray array];
    for (NSString *entry in entries) {
        if ([entry.pathExtension caseInsensitiveCompare:@"app"] != NSOrderedSame) {
            continue;
        }
        NSString *candidatePath = [payloadPath stringByAppendingPathComponent:entry];
        BOOL candidateIsDirectory = NO;
        if ([fileManager fileExistsAtPath:candidatePath isDirectory:&candidateIsDirectory] && candidateIsDirectory) {
            [appPaths addObject:candidatePath];
        }
    }

    if (appPaths.count == 0) {
        if (error != NULL) {
            *error = [XCWSimctl errorWithDescription:@"IPA archive did not contain a Payload/*.app bundle." code:15];
        }
        return nil;
    }

    [appPaths sortUsingSelector:@selector(localizedStandardCompare:)];
    return appPaths.firstObject;
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
    NSDictionary *payload = [self.class listJSONPayloadWithError:error];
    if (payload == nil) {
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
    NSMutableDictionary<NSString *, NSDictionary *> *pairInfoByDeviceUDID = [NSMutableDictionary dictionary];
    NSDictionary *pairs = payload[@"pairs"];
    if ([pairs isKindOfClass:[NSDictionary class]]) {
        [pairs enumerateKeysAndObjectsUsingBlock:^(id pairIdentifierValue, id pairValue, __unused BOOL *stop) {
            if (![pairValue isKindOfClass:[NSDictionary class]]) {
                return;
            }
            NSDictionary *pair = (NSDictionary *)pairValue;
            NSDictionary *phone = [pair[@"phone"] isKindOfClass:[NSDictionary class]] ? pair[@"phone"] : nil;
            NSDictionary *watch = [pair[@"watch"] isKindOfClass:[NSDictionary class]] ? pair[@"watch"] : nil;
            NSString *phoneUDID = XCWStringValue(phone[@"udid"]);
            NSString *watchUDID = XCWStringValue(watch[@"udid"]);
            if (phoneUDID.length == 0 || watchUDID.length == 0) {
                return;
            }

            NSString *pairIdentifier = XCWStringValue(pairIdentifierValue);
            NSString *pairState = XCWStringValue(pair[@"state"]);
            NSString *phoneName = XCWStringValue(phone[@"name"]);
            NSString *watchName = XCWStringValue(watch[@"name"]);

            NSMutableDictionary *phonePairInfo = [NSMutableDictionary dictionary];
            phonePairInfo[@"pairedWatchUDID"] = watchUDID;
            if (watchName.length > 0) {
                phonePairInfo[@"pairedWatchName"] = watchName;
            }
            if (pairIdentifier.length > 0) {
                phonePairInfo[@"devicePairIdentifier"] = pairIdentifier;
            }
            if (pairState.length > 0) {
                phonePairInfo[@"devicePairState"] = pairState;
            }
            pairInfoByDeviceUDID[phoneUDID] = phonePairInfo;

            NSMutableDictionary *watchPairInfo = [NSMutableDictionary dictionary];
            watchPairInfo[@"pairedPhoneUDID"] = phoneUDID;
            if (phoneName.length > 0) {
                watchPairInfo[@"pairedPhoneName"] = phoneName;
            }
            if (pairIdentifier.length > 0) {
                watchPairInfo[@"devicePairIdentifier"] = pairIdentifier;
            }
            if (pairState.length > 0) {
                watchPairInfo[@"devicePairState"] = pairState;
            }
            pairInfoByDeviceUDID[watchUDID] = watchPairInfo;
        }];
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

            NSMutableDictionary *entry = [@{
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
            } mutableCopy];
            NSDictionary *pairInfo = pairInfoByDeviceUDID[udid];
            if (pairInfo != nil) {
                [entry addEntriesFromDictionary:pairInfo];
            }
            [flattened addObject:entry];
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

- (nullable NSDictionary *)simulatorCreationOptionsWithError:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *payload = [self.class listJSONPayloadWithError:error];
    if (payload == nil) {
        return nil;
    }

    NSArray *runtimeArray = XCWArrayPayload(payload[@"runtimes"], @"runtimes");
    NSMutableArray<NSDictionary *> *runtimes = [NSMutableArray array];
    NSMutableDictionary<NSString *, NSMutableArray<NSString *> *> *runtimeIdentifiersByDeviceType = [NSMutableDictionary dictionary];

    for (NSDictionary *runtime in runtimeArray) {
        if (![runtime isKindOfClass:[NSDictionary class]]) {
            continue;
        }

        NSString *identifier = XCWStringValue(runtime[@"identifier"]);
        if (identifier.length == 0) {
            continue;
        }

        BOOL isAvailable = [runtime[@"isAvailable"] respondsToSelector:@selector(boolValue)] ? [runtime[@"isAvailable"] boolValue] : YES;
        if (!isAvailable) {
            continue;
        }

        NSMutableArray<NSString *> *supportedDeviceTypeIdentifiers = [NSMutableArray array];
        NSArray *supportedDeviceTypes = XCWArrayPayload(runtime[@"supportedDeviceTypes"], @"supportedDeviceTypes");
        for (NSDictionary *deviceType in supportedDeviceTypes) {
            if (![deviceType isKindOfClass:[NSDictionary class]]) {
                continue;
            }
            NSString *deviceTypeIdentifier = XCWStringValue(deviceType[@"identifier"]);
            if (deviceTypeIdentifier.length == 0) {
                continue;
            }
            [supportedDeviceTypeIdentifiers addObject:deviceTypeIdentifier];
            NSMutableArray<NSString *> *runtimeIdentifiers = runtimeIdentifiersByDeviceType[deviceTypeIdentifier];
            if (runtimeIdentifiers == nil) {
                runtimeIdentifiers = [NSMutableArray array];
                runtimeIdentifiersByDeviceType[deviceTypeIdentifier] = runtimeIdentifiers;
            }
            [runtimeIdentifiers addObject:identifier];
        }

        NSMutableDictionary *entry = [NSMutableDictionary dictionary];
        entry[@"identifier"] = identifier;
        entry[@"name"] = XCWRuntimeDisplayName(runtime, identifier);
        entry[@"isAvailable"] = @YES;
        entry[@"supportedDeviceTypeIdentifiers"] = supportedDeviceTypeIdentifiers;

        NSString *platform = XCWStringValue(runtime[@"platform"]);
        if (platform.length > 0) {
            entry[@"platform"] = platform;
        }
        NSString *version = XCWStringValue(runtime[@"version"]);
        if (version.length > 0) {
            entry[@"version"] = version;
        }
        NSString *buildVersion = XCWStringValue(runtime[@"buildversion"]);
        if (buildVersion.length > 0) {
            entry[@"buildVersion"] = buildVersion;
        }

        [runtimes addObject:entry];
    }

    NSArray *deviceTypesArray = XCWArrayPayload(payload[@"devicetypes"], @"devicetypes");
    NSMutableArray<NSDictionary *> *deviceTypes = [NSMutableArray array];
    for (NSDictionary *deviceType in deviceTypesArray) {
        if (![deviceType isKindOfClass:[NSDictionary class]]) {
            continue;
        }

        NSString *identifier = XCWStringValue(deviceType[@"identifier"]);
        if (identifier.length == 0) {
            continue;
        }
        NSArray<NSString *> *supportedRuntimeIdentifiers = runtimeIdentifiersByDeviceType[identifier];
        if (supportedRuntimeIdentifiers.count == 0) {
            continue;
        }

        NSMutableDictionary *entry = [NSMutableDictionary dictionary];
        entry[@"identifier"] = identifier;
        NSString *name = XCWStringValue(deviceType[@"name"]);
        entry[@"name"] = name.length > 0 ? name : identifier;
        entry[@"supportedRuntimeIdentifiers"] = supportedRuntimeIdentifiers;

        NSString *productFamily = XCWStringValue(deviceType[@"productFamily"]);
        if (productFamily.length > 0) {
            entry[@"productFamily"] = productFamily;
        }
        NSString *modelIdentifier = XCWStringValue(deviceType[@"modelIdentifier"]);
        if (modelIdentifier.length > 0) {
            entry[@"modelIdentifier"] = modelIdentifier;
        }
        NSString *minRuntimeVersionString = XCWStringValue(deviceType[@"minRuntimeVersionString"]);
        if (minRuntimeVersionString.length > 0) {
            entry[@"minRuntimeVersionString"] = minRuntimeVersionString;
        }
        NSString *maxRuntimeVersionString = XCWStringValue(deviceType[@"maxRuntimeVersionString"]);
        if (maxRuntimeVersionString.length > 0) {
            entry[@"maxRuntimeVersionString"] = maxRuntimeVersionString;
        }
        NSNumber *minRuntimeVersion = XCWNumberValue(deviceType[@"minRuntimeVersion"]);
        if (minRuntimeVersion != nil) {
            entry[@"minRuntimeVersion"] = minRuntimeVersion;
        }
        NSNumber *maxRuntimeVersion = XCWNumberValue(deviceType[@"maxRuntimeVersion"]);
        if (maxRuntimeVersion != nil) {
            entry[@"maxRuntimeVersion"] = maxRuntimeVersion;
        }

        [deviceTypes addObject:entry];
    }

    return @{
        @"deviceTypes": deviceTypes,
        @"runtimes": runtimes,
    };
}

- (nullable NSDictionary *)createSimulatorWithName:(NSString *)name
                              deviceTypeIdentifier:(NSString *)deviceTypeIdentifier
                                 runtimeIdentifier:(nullable NSString *)runtimeIdentifier
                                   pairedWatchName:(nullable NSString *)pairedWatchName
                   pairedWatchDeviceTypeIdentifier:(nullable NSString *)pairedWatchDeviceTypeIdentifier
                      pairedWatchRuntimeIdentifier:(nullable NSString *)pairedWatchRuntimeIdentifier
                                             error:(NSError * _Nullable __autoreleasing *)error {
    NSString *primaryUDID = [self createSingleSimulatorWithName:name
                                           deviceTypeIdentifier:deviceTypeIdentifier
                                              runtimeIdentifier:runtimeIdentifier
                                                          error:error];
    if (primaryUDID.length == 0) {
        return nil;
    }

    NSMutableDictionary *result = [@{
        @"udid": primaryUDID,
    } mutableCopy];

    NSString *watchName = [pairedWatchName stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    NSString *watchDeviceTypeIdentifier = [pairedWatchDeviceTypeIdentifier stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    if (watchName.length == 0 && watchDeviceTypeIdentifier.length == 0) {
        return result;
    }
    if (watchName.length == 0 || watchDeviceTypeIdentifier.length == 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:@"Paired watch creation requires both a watch name and device type." code:23];
        }
        return nil;
    }

    NSString *watchUDID = [self createSingleSimulatorWithName:watchName
                                         deviceTypeIdentifier:watchDeviceTypeIdentifier
                                            runtimeIdentifier:pairedWatchRuntimeIdentifier
                                                        error:error];
    if (watchUDID.length == 0) {
        return nil;
    }

    XCWProcessResult *pairResult = [self.class runSimctl:@[@"pair", watchUDID, primaryUDID]
                                             timeoutSec:120
                                                  error:error];
    if (pairResult == nil) {
        return nil;
    }
    if (pairResult.terminationStatus != 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:pairResult.stderrString.length > 0 ? pairResult.stderrString : @"Unable to pair watch simulator." code:24];
        }
        return nil;
    }

    result[@"pairedWatchUDID"] = watchUDID;
    return result;
}

- (nullable NSString *)createSingleSimulatorWithName:(NSString *)name
                                deviceTypeIdentifier:(NSString *)deviceTypeIdentifier
                                   runtimeIdentifier:(nullable NSString *)runtimeIdentifier
                                               error:(NSError * _Nullable __autoreleasing *)error {
    NSString *trimmedName = [name stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    NSString *trimmedDeviceTypeIdentifier = [deviceTypeIdentifier stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    NSString *trimmedRuntimeIdentifier = [runtimeIdentifier stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    if (trimmedName.length == 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:@"Simulator name is required." code:19];
        }
        return nil;
    }
    if (trimmedDeviceTypeIdentifier.length == 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:@"Device type identifier is required." code:20];
        }
        return nil;
    }

    NSMutableArray<NSString *> *arguments = [@[@"create", trimmedName, trimmedDeviceTypeIdentifier] mutableCopy];
    if (trimmedRuntimeIdentifier.length > 0) {
        [arguments addObject:trimmedRuntimeIdentifier];
    }

    XCWProcessResult *result = [self.class runSimctl:arguments timeoutSec:120 error:error];
    if (result == nil) {
        return nil;
    }
    if (result.terminationStatus != 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to create simulator." code:21];
        }
        return nil;
    }

    NSString *udid = [result.stdoutString stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    if (udid.length == 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:@"simctl create did not return a simulator UDID." code:22];
        }
        return nil;
    }
    return udid;
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

    if (error != NULL) {
        *error = privateError ?: [self.class errorWithDescription:@"Private CoreSimulator boot failed. SimDeck does not fall back to `xcrun simctl boot` because that can launch Simulator.app." code:4];
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
    XCWProcessResult *result = [self.class runSimctl:@[@"openurl", udid, urlString]
                                          timeoutSec:90
                                               error:error];
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
    XCWProcessResult *result = [self.class runSimctl:@[@"launch", @"--stdout=/dev/null", @"--stderr=/dev/null", udid, bundleID]
                                          timeoutSec:120
                                               error:error];
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
    return [self screenshotPNGForSimulatorUDID:udid includeBezel:NO error:error];
}

- (nullable NSData *)screenshotPNGForSimulatorUDID:(NSString *)udid
                                      includeBezel:(BOOL)includeBezel
                                             error:(NSError * _Nullable __autoreleasing *)error {
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
            if (!includeBezel) {
                return data;
            }
            NSDictionary *simulator = [self simulatorWithUDID:udid error:error];
            if (simulator == nil) {
                return nil;
            }
            NSString *deviceName = simulator[@"deviceTypeName"] ?: simulator[@"name"] ?: @"";
            return [XCWChromeRenderer screenshotPNGDataForDeviceName:deviceName
                                                       screenPNGData:data
                                                               error:error];
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

- (nullable NSData *)screenRecordingMP4ForSimulatorUDID:(NSString *)udid
                                        durationSeconds:(NSTimeInterval)durationSeconds
                                                  error:(NSError * _Nullable __autoreleasing *)error {
    if (!isfinite(durationSeconds) || durationSeconds <= 0.0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:@"Screen recording duration must be finite and greater than zero." code:33];
        }
        return nil;
    }

    NSString *filename = [NSString stringWithFormat:@"simdeck-%@.mp4", NSUUID.UUID.UUIDString];
    NSString *path = [NSTemporaryDirectory() stringByAppendingPathComponent:filename];
    XCWProcessResult *result = [self.class runSimctl:@[
        @"io",
        udid,
        @"recordVideo",
        @"--codec=h264",
        @"--force",
        path,
    ]
                                          timeoutSec:durationSeconds
                                       timeoutSignal:SIGINT
                                               error:error];
    if (result == nil) {
        [[NSFileManager defaultManager] removeItemAtPath:path error:nil];
        return nil;
    }

    BOOL expectedStop = result.terminationStatus == 0 || result.terminationStatus == 124 || result.terminationStatus == 130;
    if (expectedStop) {
        NSData *data = [NSData dataWithContentsOfFile:path options:0 error:error];
        [[NSFileManager defaultManager] removeItemAtPath:path error:nil];
        if (data.length > 0) {
            return data;
        }
        if (error != NULL && *error == nil) {
            *error = [self.class errorWithDescription:@"Simulator screen recording command produced an empty MP4." code:34];
        }
        return nil;
    }

    [[NSFileManager defaultManager] removeItemAtPath:path error:nil];
    if (error != NULL) {
        *error = [self.class errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"Unable to record simulator screen." code:34];
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
    NSString *extension = appPath.pathExtension.lowercaseString;
    if ([extension isEqualToString:@"ipa"]) {
        return [self installIPAAtPath:appPath simulatorUDID:udid error:error];
    }
    if (![extension isEqualToString:@"app"]) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:@"iOS simulator install expects an `.app` bundle or `.ipa` archive." code:15];
        }
        return NO;
    }
    return [self installAppBundleAtPath:appPath simulatorUDID:udid error:error];
}

- (BOOL)installIPAAtPath:(NSString *)ipaPath simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    NSString *extractedPath = XCWCreateTemporaryDirectory(@"simdeck-ipa", error);
    if (extractedPath == nil) {
        return NO;
    }

    XCWProcessResult *extractResult = [XCWProcessRunner runLaunchPath:@"/usr/bin/ditto"
                                                            arguments:@[@"-x", @"-k", ipaPath, extractedPath]
                                                            inputData:nil
                                                           timeoutSec:180
                                                                error:error];
    if (extractResult == nil) {
        [[NSFileManager defaultManager] removeItemAtPath:extractedPath error:nil];
        return NO;
    }
    if (extractResult.terminationStatus != 0) {
        if (error != NULL) {
            *error = [self.class errorWithDescription:extractResult.stderrString.length > 0 ? extractResult.stderrString : @"Unable to extract IPA archive." code:15];
        }
        [[NSFileManager defaultManager] removeItemAtPath:extractedPath error:nil];
        return NO;
    }

    NSString *appBundlePath = XCWAppBundlePathInExtractedIPA(extractedPath, error);
    if (appBundlePath == nil) {
        [[NSFileManager defaultManager] removeItemAtPath:extractedPath error:nil];
        return NO;
    }

    BOOL ok = [self installAppBundleAtPath:appBundlePath simulatorUDID:udid error:error];
    [[NSFileManager defaultManager] removeItemAtPath:extractedPath error:nil];
    return ok;
}

- (BOOL)installAppBundleAtPath:(NSString *)appPath simulatorUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
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
                                                                 limit:(NSUInteger)limit
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

    NSUInteger boundedLimit = limit == 0 ? NSUIntegerMax : limit;
    NSMutableArray<NSDictionary *> *entries = [NSMutableArray arrayWithCapacity:MIN(boundedLimit, 256)];
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
        if (entries.count > boundedLimit) {
            [entries removeObjectAtIndex:0];
        }
    }

    return entries;
}

+ (nullable XCWProcessResult *)runSimctl:(NSArray<NSString *> *)arguments
                                   error:(NSError * _Nullable __autoreleasing *)error {
    return [self runSimctl:arguments timeoutSec:0 error:error];
}

+ (nullable NSDictionary *)listJSONPayloadWithError:(NSError * _Nullable __autoreleasing *)error {
    XCWProcessResult *result = [self runSimctl:@[@"list", @"--json"] error:error];
    if (result == nil) {
        return nil;
    }
    if (result.terminationStatus != 0) {
        if (error != NULL) {
            *error = [self errorWithDescription:result.stderrString.length > 0 ? result.stderrString : @"simctl list failed" code:1];
        }
        return nil;
    }

    NSDictionary *payload = [NSJSONSerialization JSONObjectWithData:result.stdoutData options:0 error:error];
    if (![payload isKindOfClass:[NSDictionary class]]) {
        if (error != NULL && *error == nil) {
            *error = [self errorWithDescription:@"Unable to parse simctl JSON output." code:2];
        }
        return nil;
    }
    return payload;
}

+ (nullable XCWProcessResult *)runSimctl:(NSArray<NSString *> *)arguments
                              timeoutSec:(NSTimeInterval)timeoutSec
                                   error:(NSError * _Nullable __autoreleasing *)error {
    return [self runSimctl:arguments timeoutSec:timeoutSec timeoutSignal:SIGTERM error:error];
}

+ (nullable XCWProcessResult *)runSimctl:(NSArray<NSString *> *)arguments
                              timeoutSec:(NSTimeInterval)timeoutSec
                           timeoutSignal:(int)timeoutSignal
                                   error:(NSError * _Nullable __autoreleasing *)error {
    return [XCWProcessRunner runLaunchPath:@"/usr/bin/xcrun"
                                 arguments:[@[@"simctl"] arrayByAddingObjectsFromArray:arguments]
                                 inputData:nil
                                timeoutSec:timeoutSec
                             timeoutSignal:timeoutSignal
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
