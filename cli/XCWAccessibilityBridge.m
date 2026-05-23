#import "XCWAccessibilityBridge.h"

#import "XCWProcessRunner.h"

#import <AppKit/AppKit.h>
#import <dlfcn.h>
#import <float.h>
#import <libproc.h>
#import <limits.h>
#import <math.h>
#import <objc/message.h>
#import <objc/runtime.h>

static NSString * const XCWAccessibilityBridgeErrorDomain = @"SimDeck.AccessibilityBridge";
static NSString * const XCWCoreSimulatorPath = @"/Library/Developer/PrivateFrameworks/CoreSimulator.framework/CoreSimulator";
static NSString * const XCWAccessibilityPlatformTranslationPath = @"/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework/AccessibilityPlatformTranslation";
static const NSUInteger XCWAXMaxDepth = 80;
static NSObject *XCWAXDeviceCacheLock = nil;
static id XCWAXCachedServiceContext = nil;
static id XCWAXCachedDeviceSet = nil;
static NSMutableDictionary<NSString *, id> *XCWAXCachedDevicesByUDID = nil;
static id XCWAXSharedTranslator = nil;
static id XCWAXSharedDispatcher = nil;

typedef id _Nullable (^XCWAXTranslationCallback)(id request);

static id XCWAXObject(id object, const char *selectorName);
static pid_t XCWAXTranslationPID(id translation);

static BOOL XCWAXDebugEnabled(void) {
    static BOOL enabled = NO;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        enabled = [NSProcessInfo.processInfo.environment[@"SIMDECK_AX_DEBUG"] boolValue];
    });
    return enabled;
}

static void XCWAXDebugLog(NSString *format, ...) NS_FORMAT_FUNCTION(1, 2);
static void XCWAXDebugLog(NSString *format, ...) {
    if (!XCWAXDebugEnabled()) {
        return;
    }
    va_list args;
    va_start(args, format);
    NSString *message = [[NSString alloc] initWithFormat:format arguments:args];
    va_end(args);
    fprintf(stderr, "[simdeck-ax] %s\n", message.UTF8String);
}

static NSError *XCWAXError(NSInteger code, NSString *description) {
    return [NSError errorWithDomain:XCWAccessibilityBridgeErrorDomain
                               code:code
                           userInfo:@{ NSLocalizedDescriptionKey: description }];
}

static NSString *XCWAXActiveDeveloperDirectory(void) {
    const char *developerDir = getenv("DEVELOPER_DIR");
    if (developerDir != NULL && developerDir[0] != '\0') {
        return [NSString stringWithUTF8String:developerDir];
    }

    FILE *pipe = popen("/usr/bin/xcode-select -p 2>/dev/null", "r");
    if (pipe != NULL) {
        char buffer[PATH_MAX] = {0};
        if (fgets(buffer, sizeof(buffer), pipe) != NULL) {
            NSString *selected = [[NSString stringWithUTF8String:buffer] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
            pclose(pipe);
            if (selected.length > 0) {
                return selected;
            }
        } else {
            pclose(pipe);
        }
    }

    return @"/Applications/Xcode.app/Contents/Developer";
}

static NSArray *XCWAXFlattenCoreSimulatorDevices(id devicesPayload) {
    if ([devicesPayload isKindOfClass:NSArray.class]) {
        return devicesPayload;
    }
    if ([devicesPayload isKindOfClass:NSSet.class]) {
        return [devicesPayload allObjects];
    }
    if ([devicesPayload isKindOfClass:NSDictionary.class]) {
        NSMutableArray *devices = [NSMutableArray array];
        for (id value in [(NSDictionary *)devicesPayload allValues]) {
            [devices addObjectsFromArray:XCWAXFlattenCoreSimulatorDevices(value)];
        }
        return devices;
    }
    return @[];
}

static NSArray *XCWAXDevicesForDeviceSet(id deviceSet) {
    SEL availableSelector = sel_registerName("availableDevices");
    if ([deviceSet respondsToSelector:availableSelector]) {
        NSArray *availableDevices = XCWAXFlattenCoreSimulatorDevices(((id(*)(id, SEL))objc_msgSend)(deviceSet, availableSelector));
        if (availableDevices.count > 0) {
            return availableDevices;
        }
    }

    SEL devicesSelector = sel_registerName("devices");
    if ([deviceSet respondsToSelector:devicesSelector]) {
        return XCWAXFlattenCoreSimulatorDevices(((id(*)(id, SEL))objc_msgSend)(deviceSet, devicesSelector));
    }
    return @[];
}

static BOOL XCWAXLoadPrivateFrameworks(NSError **error) {
    static dispatch_once_t onceToken;
    static NSError *frameworkError = nil;

    dispatch_once(&onceToken, ^{
        if (!dlopen(XCWCoreSimulatorPath.fileSystemRepresentation, RTLD_NOW | RTLD_GLOBAL)) {
            frameworkError = XCWAXError(1, [NSString stringWithFormat:@"Unable to load CoreSimulator from %@.", XCWCoreSimulatorPath]);
            return;
        }
        if (!dlopen(XCWAccessibilityPlatformTranslationPath.fileSystemRepresentation, RTLD_NOW | RTLD_GLOBAL)) {
            frameworkError = XCWAXError(2, [NSString stringWithFormat:@"Unable to load AccessibilityPlatformTranslation from %@.", XCWAccessibilityPlatformTranslationPath]);
        }
    });

    if (frameworkError != nil) {
        if (error != NULL) {
            *error = frameworkError;
        }
        return NO;
    }
    return YES;
}

static NSString *XCWAXUDIDString(id device) {
    id deviceUDID = ((id(*)(id, SEL))objc_msgSend)(device, sel_registerName("UDID"));
    if ([deviceUDID respondsToSelector:sel_registerName("UUIDString")]) {
        return ((id(*)(id, SEL))objc_msgSend)(deviceUDID, sel_registerName("UUIDString"));
    }
    return [deviceUDID description] ?: @"";
}

static id XCWAXDeviceForUDID(NSString *udid, NSError **error) {
    static dispatch_once_t cacheOnceToken;
    dispatch_once(&cacheOnceToken, ^{
        XCWAXDeviceCacheLock = [NSObject new];
        XCWAXCachedDevicesByUDID = [NSMutableDictionary dictionary];
    });

    @synchronized (XCWAXDeviceCacheLock) {
        id cachedDevice = XCWAXCachedDevicesByUDID[udid];
        if (cachedDevice != nil) {
            return cachedDevice;
        }
    }

    Class serviceContextClass = NSClassFromString(@"SimServiceContext");
    if (serviceContextClass == Nil) {
        if (error != NULL) {
            *error = XCWAXError(3, @"CoreSimulator did not expose SimServiceContext.");
        }
        return nil;
    }

    @synchronized (XCWAXDeviceCacheLock) {
        if (XCWAXCachedDeviceSet == nil) {
            NSString *developerDir = XCWAXActiveDeveloperDirectory();
            NSError *serviceError = nil;
            SEL sharedSelector = sel_registerName("sharedServiceContextForDeveloperDir:error:");
            if ([serviceContextClass respondsToSelector:sharedSelector]) {
                XCWAXCachedServiceContext = ((id(*)(id, SEL, id, NSError **))objc_msgSend)(
                    serviceContextClass,
                    sharedSelector,
                    developerDir,
                    &serviceError
                );
            }
            if (XCWAXCachedServiceContext == nil) {
                serviceError = nil;
                id contextAlloc = ((id(*)(id, SEL))objc_msgSend)(serviceContextClass, sel_registerName("alloc"));
                XCWAXCachedServiceContext = ((id(*)(id, SEL, id, long long, NSError **))objc_msgSend)(
                    contextAlloc,
                    sel_registerName("initWithDeveloperDir:connectionType:error:"),
                    developerDir,
                    0LL,
                    &serviceError
                );
            }
            if (XCWAXCachedServiceContext == nil) {
                if (error != NULL) {
                    *error = serviceError ?: XCWAXError(4, [NSString stringWithFormat:@"Unable to create a CoreSimulator service context for %@.", developerDir]);
                }
                return nil;
            }

            NSError *deviceSetError = nil;
            XCWAXCachedDeviceSet = ((id(*)(id, SEL, NSError **))objc_msgSend)(
                XCWAXCachedServiceContext,
                sel_registerName("defaultDeviceSetWithError:"),
                &deviceSetError
            );
            if (XCWAXCachedDeviceSet == nil) {
                XCWAXCachedServiceContext = nil;
                if (error != NULL) {
                    *error = deviceSetError ?: XCWAXError(5, @"Unable to access the default CoreSimulator device set.");
                }
                return nil;
            }
        }

        NSArray *devices = XCWAXDevicesForDeviceSet(XCWAXCachedDeviceSet);
        for (id candidate in devices) {
            NSString *candidateUDID = XCWAXUDIDString(candidate);
            if (candidateUDID.length > 0) {
                XCWAXCachedDevicesByUDID[candidateUDID] = candidate;
            }
        }

        id device = XCWAXCachedDevicesByUDID[udid];
        if (device != nil) {
            return device;
        }
    }

    if (error != NULL) {
        *error = XCWAXError(6, [NSString stringWithFormat:@"Unable to locate simulator %@ inside the CoreSimulator device set.", udid]);
    }
    return nil;
}

static long long XCWAXDeviceState(id device) {
    if (![device respondsToSelector:sel_registerName("state")]) {
        return -1;
    }
    return ((long long(*)(id, SEL))objc_msgSend)(device, sel_registerName("state"));
}

static NSString *XCWAXAccessibilityToken(void) {
    NSString *fallback = NSUUID.UUID.UUIDString;
    XCWAXDebugLog(@"using generated accessibility token %@", fallback);
    return fallback;
}

static NSArray<NSNumber *> *XCWAXCandidateDisplayIDs(void) {
    return @[@0, @1, @2];
}

@interface XCWAccessibilityTranslationDispatcher : NSObject

- (instancetype)initWithTranslator:(id)translator;
- (void)registerDevice:(id)device token:(NSString *)token;
- (void)unregisterToken:(NSString *)token;

@end

@implementation XCWAccessibilityTranslationDispatcher {
    id _translator;
    dispatch_queue_t _callbackQueue;
    NSMutableDictionary<NSString *, id> *_devicesByToken;
}

- (instancetype)initWithTranslator:(id)translator {
    self = [super init];
    if (self == nil) {
        return nil;
    }
    _translator = translator;
    _callbackQueue = dispatch_queue_create("com.simdeck.accessibility.callback", DISPATCH_QUEUE_SERIAL);
    _devicesByToken = [NSMutableDictionary dictionary];
    return self;
}

- (void)registerDevice:(id)device token:(NSString *)token {
    @synchronized (self) {
        _devicesByToken[token] = device;
    }
}

- (void)unregisterToken:(NSString *)token {
    @synchronized (self) {
        [_devicesByToken removeObjectForKey:token];
    }
}

- (XCWAXTranslationCallback)accessibilityTranslationDelegateBridgeCallbackWithToken:(NSString *)token {
    __weak typeof(self) weakSelf = self;
    return ^id(id request) {
        XCWAXDebugLog(@"callback token=%@ request=%@", token, request);
        __strong typeof(weakSelf) strongSelf = weakSelf;
        if (strongSelf == nil) {
            return nil;
        }

        __block id device = nil;
        @synchronized (strongSelf) {
            device = strongSelf->_devicesByToken[token];
        }
        if (device == nil || ![device respondsToSelector:sel_registerName("sendAccessibilityRequestAsync:completionQueue:completionHandler:")]) {
            Class responseClass = NSClassFromString(@"AXPTranslatorResponse");
            return [responseClass respondsToSelector:sel_registerName("emptyResponse")]
                ? ((id(*)(id, SEL))objc_msgSend)(responseClass, sel_registerName("emptyResponse"))
                : nil;
        }

        dispatch_group_t group = dispatch_group_create();
        dispatch_group_enter(group);
        __block id response = nil;
        void (^completion)(id) = ^(id innerResponse) {
            response = innerResponse;
            dispatch_group_leave(group);
        };

        ((void(*)(id, SEL, id, dispatch_queue_t, id))objc_msgSend)(
            device,
            sel_registerName("sendAccessibilityRequestAsync:completionQueue:completionHandler:"),
            request,
            strongSelf->_callbackQueue,
            completion
        );
        dispatch_group_wait(group, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(5 * NSEC_PER_SEC)));
        XCWAXDebugLog(@"callback token=%@ response=%@", token, response);
        return response;
    };
}

- (CGRect)accessibilityTranslationConvertPlatformFrameToSystem:(CGRect)rect withToken:(NSString *)token {
    (void)token;
    return rect;
}

- (id)accessibilityTranslationRootParentWithToken:(NSString *)token {
    (void)token;
    return nil;
}

@end

static id XCWAXObject(id object, const char *selectorName) {
    SEL selector = sel_registerName(selectorName);
    if (object == nil || ![object respondsToSelector:selector]) {
        return nil;
    }
    @try {
        return ((id(*)(id, SEL))objc_msgSend)(object, selector);
    } @catch (NSException *exception) {
        XCWAXDebugLog(@"selector %s threw %@", selectorName, exception);
        return nil;
    }
}

static id XCWAXTranslator(NSError **error) {
    Class translatorClass = NSClassFromString(@"AXPTranslator");
    if (translatorClass == Nil) {
        if (error != NULL) {
            *error = XCWAXError(11, @"AccessibilityPlatformTranslation did not expose AXPTranslator.");
        }
        return nil;
    }

    static dispatch_once_t onceToken;
    static NSError *translatorError = nil;
    dispatch_once(&onceToken, ^{
        XCWAXSharedTranslator = [translatorClass respondsToSelector:sel_registerName("sharedInstance")]
            ? ((id(*)(id, SEL))objc_msgSend)(translatorClass, sel_registerName("sharedInstance"))
            : nil;
        if (XCWAXSharedTranslator == nil) {
            translatorError = XCWAXError(8, @"AccessibilityPlatformTranslation did not expose AXPTranslator.sharedInstance.");
            return;
        }

        XCWAXSharedDispatcher = [[XCWAccessibilityTranslationDispatcher alloc] initWithTranslator:XCWAXSharedTranslator];
        if ([XCWAXSharedTranslator respondsToSelector:sel_registerName("setBridgeTokenDelegate:")]) {
            ((void(*)(id, SEL, id))objc_msgSend)(XCWAXSharedTranslator, sel_registerName("setBridgeTokenDelegate:"), XCWAXSharedDispatcher);
        } else {
            translatorError = XCWAXError(12, @"AXPTranslator did not expose setBridgeTokenDelegate:.");
        }
    });

    if (translatorError != nil) {
        if (error != NULL) {
            *error = translatorError;
        }
        return nil;
    }
    return XCWAXSharedTranslator;
}

static void XCWAXEnableTranslator(id translator) {
    id platformTranslator = XCWAXObject(translator, "platformTranslator");
    for (id candidate in @[translator ?: NSNull.null, platformTranslator ?: NSNull.null]) {
        if (candidate == NSNull.null) {
            continue;
        }
        @try {
            if ([candidate respondsToSelector:sel_registerName("setAccessibilityEnabled:")]) {
                ((void(*)(id, SEL, BOOL))objc_msgSend)(candidate, sel_registerName("setAccessibilityEnabled:"), YES);
            }
            if (candidate == platformTranslator && [candidate respondsToSelector:sel_registerName("enableAccessibility")]) {
                ((void(*)(id, SEL))objc_msgSend)(candidate, sel_registerName("enableAccessibility"));
            }
        } @catch (NSException *exception) {
            XCWAXDebugLog(@"enableAccessibility threw for %@: %@", candidate, exception);
        }
    }
}

static BOOL XCWAXBool(id object, const char *selectorName) {
    SEL selector = sel_registerName(selectorName);
    if (object == nil || ![object respondsToSelector:selector]) {
        return NO;
    }
    @try {
        return ((BOOL(*)(id, SEL))objc_msgSend)(object, selector);
    } @catch (NSException *exception) {
        XCWAXDebugLog(@"selector %s threw %@", selectorName, exception);
        return NO;
    }
}

static CGRect XCWAXFrame(id object) {
    SEL selector = sel_registerName("accessibilityFrame");
    if (object == nil || ![object respondsToSelector:selector]) {
        return CGRectZero;
    }
    @try {
        return ((CGRect(*)(id, SEL))objc_msgSend)(object, selector);
    } @catch (NSException *exception) {
        XCWAXDebugLog(@"accessibilityFrame threw %@", exception);
        return CGRectZero;
    }
}

static CGFloat XCWAXFrameArea(CGRect frame) {
    if (CGRectIsNull(frame) || frame.size.width <= 0 || frame.size.height <= 0) {
        return 0;
    }
    return frame.size.width * frame.size.height;
}

static NSUInteger XCWAXElementChildCount(id element) {
    id children = XCWAXObject(element, "accessibilityChildren");
    return [children isKindOfClass:NSArray.class] ? [(NSArray *)children count] : 0;
}

static NSString *XCWAXProcessPathForPID(pid_t pid) {
    if (pid <= 0) {
        return @"";
    }

    char path[PROC_PIDPATHINFO_MAXSIZE] = {0};
    int length = proc_pidpath(pid, path, sizeof(path));
    if (length <= 0) {
        return @"";
    }
    return [[NSString alloc] initWithBytes:path length:(NSUInteger)length encoding:NSUTF8StringEncoding] ?: @"";
}

static NSString *XCWAXProcessNameForPID(pid_t pid) {
    if (pid <= 0) {
        return @"";
    }

    char name[PROC_PIDPATHINFO_MAXSIZE] = {0};
    int length = proc_name(pid, name, sizeof(name));
    if (length <= 0) {
        return @"";
    }
    return [[NSString alloc] initWithBytes:name length:(NSUInteger)length encoding:NSUTF8StringEncoding] ?: @"";
}

static BOOL XCWAXPIDLooksLikeWidgetRenderer(pid_t pid) {
    NSString *processPath = XCWAXProcessPathForPID(pid);
    if ([processPath.lastPathComponent containsString:@"WidgetRenderer"]) {
        return YES;
    }
    return [XCWAXProcessNameForPID(pid) containsString:@"WidgetRenderer"];
}

static BOOL XCWAXProcessPathLooksLikeExtension(NSString *path) {
    NSString *lowercase = path.lowercaseString;
    return [lowercase containsString:@".appex/"] || [lowercase containsString:@"/plugins/"];
}

static BOOL XCWAXAXValueLooksEmpty(id value) {
    if (value == nil || value == NSNull.null) {
        return YES;
    }
    if (![value isKindOfClass:NSString.class]) {
        return NO;
    }
    NSString *trimmed = [(NSString *)value stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    return trimmed.length == 0;
}

static NSString *XCWAXRecoveredRootLabelForCandidate(NSDictionary *candidate) {
    id pathValue = candidate[@"processPath"];
    NSString *processPath = [pathValue isKindOfClass:NSString.class] ? (NSString *)pathValue : @"";
    NSString *lastPathComponent = processPath.lastPathComponent;
    if ([processPath containsString:@"WebContentExtension.appex"] || [lastPathComponent isEqualToString:@"com.apple.WebKit.WebContent"]) {
        return @"WebKit WebContent";
    }
    return nil;
}

static void XCWAXApplyRecoveredRootMetadata(NSMutableDictionary *root, NSDictionary *candidate) {
    NSString *label = XCWAXRecoveredRootLabelForCandidate(candidate);
    if (label.length == 0 || !XCWAXAXValueLooksEmpty(root[@"AXLabel"])) {
        return;
    }
    root[@"AXLabel"] = label;
}

static NSArray<NSString *> *XCWAXLaunchctlLines(NSString *output) {
    return [output componentsSeparatedByCharactersInSet:NSCharacterSet.newlineCharacterSet];
}

static NSString *XCWAXLaunchctlValue(NSString *output, NSString *key) {
    NSString *prefix = [key stringByAppendingString:@" = "];
    for (NSString *line in XCWAXLaunchctlLines(output)) {
        NSString *trimmed = [line stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
        if (![trimmed hasPrefix:prefix]) {
            continue;
        }
        NSString *value = [[trimmed substringFromIndex:prefix.length] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
        return value.length > 0 ? value : nil;
    }
    return nil;
}

static unsigned long long XCWAXLaunchctlUnsignedValue(NSString *output, NSString *key) {
    return [XCWAXLaunchctlValue(output, key) longLongValue];
}

static NSDictionary *XCWAXParseUIKitApplicationServiceLine(NSString *line) {
    NSString *trimmed = [line stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    if (![trimmed containsString:@"UIKitApplication:"]) {
        return nil;
    }

    NSArray<NSString *> *parts = [trimmed componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
    NSMutableArray<NSString *> *tokens = [NSMutableArray array];
    for (NSString *part in parts) {
        if (part.length > 0) {
            [tokens addObject:part];
        }
    }
    if (tokens.count < 3 || ![tokens[1] isEqualToString:@"-"] || ![tokens[2] hasPrefix:@"UIKitApplication:"]) {
        return nil;
    }

    pid_t pid = (pid_t)tokens[0].intValue;
    if (pid <= 0) {
        return nil;
    }
    return @{
        @"pid": @(pid),
        @"service": tokens[2],
    };
}

static NSUInteger XCWAXUIKitApplicationForegroundScore(NSString *details, unsigned long long *activeCount) {
    NSString *spawnRole = XCWAXLaunchctlValue(details, @"spawn role") ?: @"";
    if (activeCount != NULL) {
        *activeCount = XCWAXLaunchctlUnsignedValue(details, @"active count");
    }
    if ([spawnRole containsString:@"ui focal"]) {
        return 2;
    }
    if ([spawnRole containsString:@"ui"]) {
        return 1;
    }
    return 0;
}

static BOOL XCWAXUIKitServiceLooksDecisive(NSString *serviceName) {
    if (serviceName.length == 0) {
        return YES;
    }
    return [serviceName rangeOfString:@"ViewService"].location == NSNotFound &&
           [serviceName rangeOfString:@"WidgetRenderer"].location == NSNotFound &&
           [serviceName rangeOfString:@".appex"].location == NSNotFound;
}

static BOOL XCWAXUIKitApplicationCandidateIsBetter(NSDictionary *candidate, NSDictionary *current) {
    if (current == nil) {
        return YES;
    }
    NSUInteger candidateScore = [candidate[@"score"] unsignedIntegerValue];
    NSUInteger currentScore = [current[@"score"] unsignedIntegerValue];
    if (candidateScore != currentScore) {
        return candidateScore > currentScore;
    }
    unsigned long long candidateActiveCount = [candidate[@"activeCount"] unsignedLongLongValue];
    unsigned long long currentActiveCount = [current[@"activeCount"] unsignedLongLongValue];
    if (candidateActiveCount != currentActiveCount) {
        return candidateActiveCount > currentActiveCount;
    }
    return [candidate[@"pid"] intValue] < [current[@"pid"] intValue];
}

static pid_t XCWAXForegroundUIKitApplicationPID(NSString *udid) {
    NSError *error = nil;
    XCWProcessResult *result = [XCWProcessRunner runLaunchPath:@"/usr/bin/xcrun"
                                                     arguments:@[@"simctl", @"spawn", udid, @"launchctl", @"print", @"user/501"]
                                                     inputData:nil
                                                    timeoutSec:1
                                                         error:&error];
    if (result == nil || result.terminationStatus != 0) {
        XCWAXDebugLog(@"foreground UIKit application listing failed: %@", error ?: result.stderrString);
        return 0;
    }

    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:1.0];
    NSDictionary *best = nil;
    for (NSString *line in XCWAXLaunchctlLines(result.stdoutString)) {
        NSTimeInterval remaining = [deadline timeIntervalSinceNow];
        if (remaining <= 0) {
            break;
        }
        NSDictionary *service = XCWAXParseUIKitApplicationServiceLine(line);
        if (service == nil) {
            continue;
        }

        NSString *serviceName = service[@"service"];
        XCWProcessResult *detailsResult = [XCWProcessRunner runLaunchPath:@"/usr/bin/xcrun"
                                                                 arguments:@[@"simctl", @"spawn", udid, @"launchctl", @"print", [@"user/501/" stringByAppendingString:serviceName]]
                                                                 inputData:nil
                                                                timeoutSec:MAX(0.1, remaining)
                                                                     error:nil];
        if (detailsResult == nil || detailsResult.terminationStatus != 0) {
            continue;
        }

        NSString *details = detailsResult.stdoutString;
        NSString *state = XCWAXLaunchctlValue(details, @"state");
        if (state != nil && ![state isEqualToString:@"running"]) {
            continue;
        }
        pid_t pid = (pid_t)(XCWAXLaunchctlUnsignedValue(details, @"pid") ?: [service[@"pid"] intValue]);
        if (pid <= 0) {
            continue;
        }

        unsigned long long activeCount = 0;
        NSUInteger score = XCWAXUIKitApplicationForegroundScore(details, &activeCount);
        NSDictionary *candidate = @{
            @"pid": @(pid),
            @"score": @(score),
            @"activeCount": @(activeCount),
            @"service": serviceName,
        };
        if (XCWAXUIKitApplicationCandidateIsBetter(candidate, best)) {
            best = candidate;
        }
        if (score >= 2 && XCWAXUIKitServiceLooksDecisive(serviceName)) {
            break;
        }
    }

    pid_t pid = [best[@"pid"] intValue];
    XCWAXDebugLog(@"foreground UIKit application candidate=%@", best);
    return pid;
}

static id XCWAXJSONValue(id value) {
    if (value == nil) {
        return NSNull.null;
    }
    if ([NSJSONSerialization isValidJSONObject:@[value]]) {
        return value;
    }
    return [value description] ?: @"";
}

static NSString *XCWAXRoleType(NSString *role) {
    if ([role hasPrefix:@"AX"] && role.length > 2) {
        return [role substringFromIndex:2];
    }
    return role ?: @"";
}

static BOOL XCWAXRoleLooksInteractive(NSString *role) {
    NSString *lowercase = (role ?: @"").lowercaseString;
    for (NSString *needle in @[
        @"button",
        @"cell",
        @"checkbox",
        @"collection",
        @"combobox",
        @"control",
        @"link",
        @"menu",
        @"picker",
        @"radio",
        @"scroll",
        @"search",
        @"segmented",
        @"select",
        @"slider",
        @"stepper",
        @"switch",
        @"tab",
        @"table",
        @"textfield",
        @"text field",
        @"textinput",
        @"text input",
        @"toggle",
        @"webview",
    ]) {
        if ([lowercase containsString:needle]) {
            return YES;
        }
    }
    return NO;
}

static BOOL XCWAXElementLooksInteractive(id element, NSString *role) {
    if (XCWAXBool(element, "isAccessibilityHidden")) {
        return NO;
    }
    if (XCWAXBool(element, "isAccessibilityFocused")) {
        return YES;
    }
    return XCWAXRoleLooksInteractive(role) || XCWAXRoleLooksInteractive(XCWAXRoleType(role));
}

static pid_t XCWAXElementPID(id element) {
    id translation = XCWAXObject(element, "translation");
    SEL selector = sel_registerName("pid");
    if (translation == nil || ![translation respondsToSelector:selector]) {
        return 0;
    }
    return ((pid_t(*)(id, SEL))objc_msgSend)(translation, selector);
}

static NSDictionary *XCWAXDictionaryForElement(id element) {
    CGRect frame = XCWAXFrame(element);
    NSString *role = XCWAXObject(element, "accessibilityRole");
    NSMutableDictionary *values = [NSMutableDictionary dictionary];
    values[@"AXLabel"] = XCWAXJSONValue(XCWAXObject(element, "accessibilityLabel"));
    values[@"AXFrame"] = NSStringFromRect(frame);
    values[@"AXValue"] = XCWAXJSONValue(XCWAXObject(element, "accessibilityValue"));
    values[@"AXUniqueId"] = XCWAXJSONValue(XCWAXObject(element, "accessibilityIdentifier"));
    values[@"type"] = XCWAXJSONValue(XCWAXRoleType(role));
    values[@"role"] = XCWAXJSONValue(role);
    values[@"title"] = XCWAXJSONValue(XCWAXObject(element, "accessibilityTitle"));
    values[@"help"] = XCWAXJSONValue(XCWAXObject(element, "accessibilityHelp"));
    values[@"role_description"] = XCWAXJSONValue(XCWAXObject(element, "accessibilityRoleDescription"));
    values[@"subrole"] = XCWAXJSONValue(XCWAXObject(element, "accessibilitySubrole"));
    values[@"placeholder"] = XCWAXJSONValue(XCWAXObject(element, "accessibilityPlaceholderValue"));
    values[@"enabled"] = @(XCWAXBool(element, "accessibilityEnabled"));
    values[@"hidden"] = @(XCWAXBool(element, "isAccessibilityHidden"));
    values[@"focused"] = @(XCWAXBool(element, "isAccessibilityFocused"));
    values[@"pid"] = @(XCWAXElementPID(element));
    values[@"frame"] = @{
        @"x": @(frame.origin.x),
        @"y": @(frame.origin.y),
        @"width": @(frame.size.width),
        @"height": @(frame.size.height),
    };
    return values;
}

static NSMutableDictionary *XCWAXSerializeElementWithOptions(id element, NSString *token, NSHashTable *visited, NSUInteger depth, NSUInteger maxDepth, BOOL interactiveOnly) {
    if (element == nil || depth > maxDepth || [visited containsObject:element]) {
        return nil;
    }
    [visited addObject:element];

    id translation = XCWAXObject(element, "translation");
    if (translation != nil && [translation respondsToSelector:sel_registerName("setBridgeDelegateToken:")]) {
        ((void(*)(id, SEL, id))objc_msgSend)(translation, sel_registerName("setBridgeDelegateToken:"), token);
    }

    NSMutableArray *childrenValues = [NSMutableArray array];
    id children = depth < maxDepth ? XCWAXObject(element, "accessibilityChildren") : nil;
    if ([children isKindOfClass:NSArray.class]) {
        for (id child in (NSArray *)children) {
            NSMutableDictionary *childValues = XCWAXSerializeElementWithOptions(child, token, visited, depth + 1, maxDepth, interactiveOnly);
            if (childValues != nil) {
                [childrenValues addObject:childValues];
            }
        }
    }

    NSString *role = XCWAXObject(element, "accessibilityRole");
    if (interactiveOnly && childrenValues.count == 0 && !XCWAXElementLooksInteractive(element, role)) {
        return nil;
    }

    NSMutableDictionary *values = [XCWAXDictionaryForElement(element) mutableCopy];
    if (!interactiveOnly || childrenValues.count > 0) {
        values[@"children"] = childrenValues;
    }
    return values;
}

static NSMutableDictionary *XCWAXSerializeElement(id element, NSString *token, NSHashTable *visited, NSUInteger depth, NSUInteger maxDepth) {
    return XCWAXSerializeElementWithOptions(element, token, visited, depth, maxDepth, NO);
}

static NSArray<NSValue *> *XCWAXRootRecoveryHitTestPoints(void) {
    NSMutableArray<NSValue *> *points = [NSMutableArray array];
    NSArray<NSNumber *> *xValues = @[@80, @220, @360];
    NSArray<NSNumber *> *yValues = @[@120, @280, @480, @700, @840];
    for (NSNumber *yValue in yValues) {
        for (NSNumber *xValue in xValues) {
            [points addObject:[NSValue valueWithPoint:CGPointMake(xValue.doubleValue, yValue.doubleValue)]];
        }
    }
    return points;
}

static pid_t XCWAXTranslationPID(id translation) {
    SEL selector = sel_registerName("pid");
    if (translation == nil || ![translation respondsToSelector:selector]) {
        return 0;
    }
    @try {
        return ((pid_t(*)(id, SEL))objc_msgSend)(translation, selector);
    } @catch (NSException *exception) {
        XCWAXDebugLog(@"translation pid threw %@", exception);
        return 0;
    }
}

static void XCWAXSetBridgeDelegateTokenOnTranslation(id translation, NSString *token) {
    if (translation != nil && [translation respondsToSelector:sel_registerName("setBridgeDelegateToken:")]) {
        ((void(*)(id, SEL, id))objc_msgSend)(translation, sel_registerName("setBridgeDelegateToken:"), token);
    }
}

static id XCWAXApplicationTranslationForPID(id translator, pid_t pid, NSString *token) {
    if (pid <= 0 || translator == nil) {
        return nil;
    }

    Class requestClass = NSClassFromString(@"AXPTranslatorRequest");
    Class translationClass = NSClassFromString(@"AXPTranslationObject");
    SEL sendSelector = sel_registerName("sendTranslatorRequest:");
    if (requestClass != Nil && translationClass != Nil && [translator respondsToSelector:sendSelector]) {
        @try {
            // translationApplicationObjectForPid: builds this request without a
            // bridge token, which cannot route through the simulator delegate
            // after some private display lifecycle changes.
            id request = [requestClass new];
            id requestTranslation = [translationClass new];
            if ([requestTranslation respondsToSelector:sel_registerName("setPid:")]) {
                ((void(*)(id, SEL, pid_t))objc_msgSend)(requestTranslation, sel_registerName("setPid:"), pid);
            }
            XCWAXSetBridgeDelegateTokenOnTranslation(requestTranslation, token);

            if ([request respondsToSelector:sel_registerName("setRequestType:")]) {
                ((void(*)(id, SEL, NSUInteger))objc_msgSend)(request, sel_registerName("setRequestType:"), (NSUInteger)1);
            }
            if ([request respondsToSelector:sel_registerName("setParameters:")]) {
                ((void(*)(id, SEL, id))objc_msgSend)(request, sel_registerName("setParameters:"), @{ @"pid": @(pid) });
            }
            if ([request respondsToSelector:sel_registerName("setTranslation:")]) {
                ((void(*)(id, SEL, id))objc_msgSend)(request, sel_registerName("setTranslation:"), requestTranslation);
            }

            id response = ((id(*)(id, SEL, id))objc_msgSend)(translator, sendSelector, request);
            id translation = XCWAXObject(response, "translationResponse");
            XCWAXSetBridgeDelegateTokenOnTranslation(translation, token);
            if (translation != nil) {
                return translation;
            }
        } @catch (NSException *exception) {
            XCWAXDebugLog(@"tokenized application translation request for pid:%d threw %@", pid, exception);
        }
    }

    SEL selector = sel_registerName("translationApplicationObjectForPid:");
    if (![translator respondsToSelector:selector]) {
        return nil;
    }
    @try {
        return ((id(*)(id, SEL, pid_t))objc_msgSend)(translator, selector, pid);
    } @catch (NSException *exception) {
        XCWAXDebugLog(@"translationApplicationObjectForPid:%d threw %@", pid, exception);
        return nil;
    }
}

static id XCWAXMacPlatformElementFromTranslation(id translator, id translation) {
    SEL selector = sel_registerName("macPlatformElementFromTranslation:");
    if (translator == nil || translation == nil || ![translator respondsToSelector:selector]) {
        return nil;
    }
    @try {
        return ((id(*)(id, SEL, id))objc_msgSend)(translator, selector, translation);
    } @catch (NSException *exception) {
        XCWAXDebugLog(@"macPlatformElementFromTranslation threw %@", exception);
        return nil;
    }
}

static NSMutableDictionary *XCWAXRootRecoveryCandidateFromTranslation(id translator, id translation, NSNumber *displayID, NSString *token) {
    if (translation == nil) {
        return nil;
    }

    XCWAXSetBridgeDelegateTokenOnTranslation(translation, token);
    id element = XCWAXMacPlatformElementFromTranslation(translator, translation);
    if (element == nil) {
        return nil;
    }

    pid_t pid = XCWAXTranslationPID(translation);
    CGRect frame = XCWAXFrame(element);
    NSString *processPath = XCWAXProcessPathForPID(pid);
    return [@{
        @"translation": translation,
        @"displayID": displayID,
        @"pid": @(pid),
        @"area": @(XCWAXFrameArea(frame)),
        @"childCount": @(XCWAXElementChildCount(element)),
        @"hitCount": @0,
        @"processPath": processPath,
        @"isExtension": @(XCWAXProcessPathLooksLikeExtension(processPath)),
    } mutableCopy];
}

static NSMutableDictionary *XCWAXRootRecoveryCandidate(id translator, pid_t pid, NSNumber *displayID, NSString *token) {
    id applicationTranslation = XCWAXApplicationTranslationForPID(translator, pid, token);
    return XCWAXRootRecoveryCandidateFromTranslation(translator, applicationTranslation, displayID, token);
}

static NSNumber *XCWAXRootRecoveryCandidateKey(NSDictionary *candidate) {
    pid_t pid = [candidate[@"pid"] intValue];
    if (pid > 0) {
        return @(pid);
    }

    id translation = candidate[@"translation"];
    uintptr_t pointerValue = (uintptr_t)(__bridge const void *)(translation);
    return @(-((long long)(pointerValue & 0x7fffffffffffffffULL)));
}

static BOOL XCWAXRootRecoveryCandidateIsBetter(NSDictionary *candidate, NSDictionary *current) {
    if (current == nil) {
        return YES;
    }

    BOOL candidateExtension = [candidate[@"isExtension"] boolValue];
    BOOL currentExtension = [current[@"isExtension"] boolValue];
    if (candidateExtension != currentExtension) {
        return !candidateExtension;
    }

    CGFloat candidateArea = [candidate[@"area"] doubleValue];
    CGFloat currentArea = [current[@"area"] doubleValue];
    if (candidateArea > currentArea + 1.0) {
        return YES;
    }
    if (currentArea > candidateArea + 1.0) {
        return NO;
    }

    NSUInteger candidateHits = [candidate[@"hitCount"] unsignedIntegerValue];
    NSUInteger currentHits = [current[@"hitCount"] unsignedIntegerValue];
    if (candidateHits != currentHits) {
        return candidateHits > currentHits;
    }

    NSUInteger candidateChildren = [candidate[@"childCount"] unsignedIntegerValue];
    NSUInteger currentChildren = [current[@"childCount"] unsignedIntegerValue];
    if (candidateChildren != currentChildren) {
        return candidateChildren > currentChildren;
    }

    pid_t candidatePID = [candidate[@"pid"] intValue];
    pid_t currentPID = [current[@"pid"] intValue];
    if ((candidatePID > 0) != (currentPID > 0)) {
        return candidatePID > 0;
    }

    return candidatePID < currentPID;
}

static NSMutableDictionary *XCWAXStoreRootRecoveryCandidate(NSMutableDictionary<NSNumber *, NSMutableDictionary *> *candidatesByKey, NSMutableDictionary *candidate) {
    if (candidate == nil) {
        return nil;
    }

    NSNumber *key = XCWAXRootRecoveryCandidateKey(candidate);
    NSMutableDictionary *current = candidatesByKey[key];
    if (XCWAXRootRecoveryCandidateIsBetter(candidate, current)) {
        candidatesByKey[key] = candidate;
        return candidate;
    }
    return current;
}

static NSArray<NSDictionary *> *XCWAXSortedRootRecoveryCandidates(NSDictionary<NSNumber *, NSMutableDictionary *> *candidatesByPID) {
    return [candidatesByPID.allValues sortedArrayUsingComparator:^NSComparisonResult(NSDictionary *first, NSDictionary *second) {
        if (XCWAXRootRecoveryCandidateIsBetter(first, second)) {
            return NSOrderedAscending;
        }
        if (XCWAXRootRecoveryCandidateIsBetter(second, first)) {
            return NSOrderedDescending;
        }
        return NSOrderedSame;
    }];
}

static CGFloat XCWAXCGFloatField(NSDictionary *dictionary, NSString *key) {
    id value = dictionary[key];
    return [value respondsToSelector:@selector(doubleValue)] ? [value doubleValue] : NAN;
}

static BOOL XCWAXSerializedFrameIsValid(CGRect frame) {
    return !CGRectIsNull(frame) && isfinite(frame.origin.x) && isfinite(frame.origin.y) && isfinite(frame.size.width) && isfinite(frame.size.height) && frame.size.width > 0 && frame.size.height > 0;
}

static CGRect XCWAXSerializedNodeFrame(NSDictionary *node) {
    id frameValue = node[@"frame"];
    if (![frameValue isKindOfClass:NSDictionary.class]) {
        return CGRectNull;
    }

    NSDictionary *frame = (NSDictionary *)frameValue;
    CGRect rect = CGRectMake(
        XCWAXCGFloatField(frame, @"x"),
        XCWAXCGFloatField(frame, @"y"),
        XCWAXCGFloatField(frame, @"width"),
        XCWAXCGFloatField(frame, @"height")
    );
    return XCWAXSerializedFrameIsValid(rect) ? rect : CGRectNull;
}

static NSString *XCWAXSerializedNodeText(NSDictionary *node) {
    for (NSString *key in @[@"AXLabel", @"title", @"AXValue", @"AXUniqueId"]) {
        id value = node[key];
        if (![value isKindOfClass:NSString.class]) {
            continue;
        }
        NSString *trimmed = [(NSString *)value stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
        if (trimmed.length > 0) {
            return trimmed;
        }
    }
    return @"";
}

static pid_t XCWAXSerializedNodePID(NSDictionary *node) {
    id value = node[@"pid"];
    return [value respondsToSelector:@selector(intValue)] ? (pid_t)[value intValue] : 0;
}

static void XCWAXSetSerializedNodeFrame(NSMutableDictionary *node, CGRect frame) {
    if (!XCWAXSerializedFrameIsValid(frame)) {
        return;
    }
    node[@"AXFrame"] = NSStringFromRect(frame);
    node[@"frame"] = @{
        @"x": @(frame.origin.x),
        @"y": @(frame.origin.y),
        @"width": @(frame.size.width),
        @"height": @(frame.size.height),
    };
}

static void XCWAXCollectSerializedFrameAnchors(NSDictionary *node, NSMutableArray<NSDictionary *> *anchors) {
    NSString *label = XCWAXSerializedNodeText(node);
    CGRect frame = XCWAXSerializedNodeFrame(node);
    id roleValue = node[@"role"];
    BOOL isApplication = [roleValue isKindOfClass:NSString.class] && [(NSString *)roleValue isEqualToString:@"AXApplication"];
    if (!isApplication && label.length > 0 && XCWAXSerializedFrameIsValid(frame)) {
        [anchors addObject:@{
            @"label": label,
            @"frame": [NSValue valueWithRect:frame],
        }];
    }

    id children = node[@"children"];
    if (![children isKindOfClass:NSArray.class]) {
        return;
    }
    for (NSDictionary *child in (NSArray *)children) {
        if ([child isKindOfClass:NSDictionary.class]) {
            XCWAXCollectSerializedFrameAnchors(child, anchors);
        }
    }
}

static double XCWAXWidgetAnchorScore(CGRect localFrame, CGRect anchorFrame) {
    CGFloat widthRatio = anchorFrame.size.width / localFrame.size.width;
    CGFloat heightRatio = anchorFrame.size.height / localFrame.size.height;
    if (!isfinite(widthRatio) || !isfinite(heightRatio) || widthRatio <= 0 || heightRatio <= 0) {
        return DBL_MAX;
    }
    if (widthRatio < 0.35 || heightRatio < 0.35 || widthRatio > 3.0 || heightRatio > 3.0) {
        return DBL_MAX;
    }
    return fabs(log(widthRatio)) + fabs(log(heightRatio));
}

static BOOL XCWAXFrameContainsPoint(CGRect frame, CGPoint point) {
    return XCWAXSerializedFrameIsValid(frame) &&
        point.x >= frame.origin.x &&
        point.y >= frame.origin.y &&
        point.x <= frame.origin.x + frame.size.width &&
        point.y <= frame.origin.y + frame.size.height;
}

static NSUInteger XCWAXBestWidgetAnchorIndex(NSDictionary *child, NSArray<NSDictionary *> *anchors, NSSet<NSNumber *> *usedIndexes) {
    NSString *label = XCWAXSerializedNodeText(child);
    CGRect localFrame = XCWAXSerializedNodeFrame(child);
    if (label.length == 0 || !XCWAXSerializedFrameIsValid(localFrame)) {
        return NSNotFound;
    }

    NSUInteger bestIndex = NSNotFound;
    double bestScore = DBL_MAX;
    for (NSUInteger index = 0; index < anchors.count; index++) {
        if ([usedIndexes containsObject:@(index)]) {
            continue;
        }
        NSDictionary *anchor = anchors[index];
        if (![anchor[@"label"] isEqualToString:label]) {
            continue;
        }

        CGRect anchorFrame = [anchor[@"frame"] rectValue];
        double score = XCWAXWidgetAnchorScore(localFrame, anchorFrame);
        if (score < bestScore) {
            bestScore = score;
            bestIndex = index;
        }
    }
    return bestScore < 0.9 ? bestIndex : NSNotFound;
}

static NSUInteger XCWAXBestWidgetAnchorIndexContainingPoint(NSArray<NSDictionary *> *anchors, CGPoint point) {
    NSUInteger bestIndex = NSNotFound;
    CGFloat bestArea = CGFLOAT_MAX;
    for (NSUInteger index = 0; index < anchors.count; index++) {
        CGRect frame = [anchors[index][@"frame"] rectValue];
        if (!XCWAXFrameContainsPoint(frame, point)) {
            continue;
        }
        CGFloat area = XCWAXFrameArea(frame);
        if (area < bestArea) {
            bestArea = area;
            bestIndex = index;
        }
    }
    return bestIndex;
}

static CGRect XCWAXMapFrameFromLocalToScreen(CGRect frame, CGRect sourceFrame, CGRect targetFrame) {
    if (!XCWAXSerializedFrameIsValid(frame) || !XCWAXSerializedFrameIsValid(sourceFrame) || !XCWAXSerializedFrameIsValid(targetFrame)) {
        return CGRectNull;
    }

    CGFloat scaleX = targetFrame.size.width / sourceFrame.size.width;
    CGFloat scaleY = targetFrame.size.height / sourceFrame.size.height;
    return CGRectMake(
        targetFrame.origin.x + ((frame.origin.x - sourceFrame.origin.x) * scaleX),
        targetFrame.origin.y + ((frame.origin.y - sourceFrame.origin.y) * scaleY),
        frame.size.width * scaleX,
        frame.size.height * scaleY
    );
}

static void XCWAXMapSerializedNodeFramesFromLocalToScreen(NSMutableDictionary *node, CGRect sourceFrame, CGRect targetFrame) {
    CGRect frame = XCWAXSerializedNodeFrame(node);
    if (XCWAXSerializedFrameIsValid(frame)) {
        XCWAXSetSerializedNodeFrame(node, XCWAXMapFrameFromLocalToScreen(frame, sourceFrame, targetFrame));
    }

    id children = node[@"children"];
    if (![children isKindOfClass:NSArray.class]) {
        return;
    }
    for (NSMutableDictionary *child in (NSArray *)children) {
        if ([child isKindOfClass:NSMutableDictionary.class]) {
            XCWAXMapSerializedNodeFramesFromLocalToScreen(child, sourceFrame, targetFrame);
        }
    }
}

static CGRect XCWAXUnionSerializedNodeFrames(NSArray *nodes) {
    CGRect unionFrame = CGRectNull;
    for (NSDictionary *node in nodes) {
        if (![node isKindOfClass:NSDictionary.class]) {
            continue;
        }
        CGRect frame = XCWAXSerializedNodeFrame(node);
        if (!XCWAXSerializedFrameIsValid(frame)) {
            continue;
        }
        unionFrame = CGRectIsNull(unionFrame) ? frame : CGRectUnion(unionFrame, frame);
    }
    return unionFrame;
}

static BOOL XCWAXSerializedRootLooksLikeWidgetRenderer(NSDictionary *root, NSDictionary *candidate) {
    if ([candidate[@"isExtension"] boolValue]) {
        return YES;
    }

    NSString *label = XCWAXSerializedNodeText(root);
    if ([label containsString:@"WidgetRenderer"]) {
        return YES;
    }

    id processPathValue = candidate[@"processPath"];
    NSString *processPath = [processPathValue isKindOfClass:NSString.class] ? (NSString *)processPathValue : @"";
    if (processPath.length == 0) {
        pid_t pid = XCWAXSerializedNodePID(root);
        if (XCWAXPIDLooksLikeWidgetRenderer(pid)) {
            return YES;
        }
        processPath = XCWAXProcessPathForPID(pid);
    }
    return [processPath.lastPathComponent containsString:@"WidgetRenderer"] || XCWAXPIDLooksLikeWidgetRenderer(XCWAXSerializedNodePID(root));
}

static NSMutableDictionary *XCWAXSerializeTranslationRoot(id translator, id translation, NSString *token, NSUInteger maxDepth) {
    XCWAXSetBridgeDelegateTokenOnTranslation(translation, token);
    id element = XCWAXMacPlatformElementFromTranslation(translator, translation);
    if (element == nil) {
        return nil;
    }

    NSHashTable *visited = [NSHashTable hashTableWithOptions:NSPointerFunctionsObjectPointerPersonality];
    return XCWAXSerializeElement(element, token, visited, 0, MIN(maxDepth, XCWAXMaxDepth));
}

static CGRect XCWAXWidgetLocalSourceFrameForAnchor(NSDictionary *widgetRoot, NSString *anchorLabel, CGRect targetFrame) {
    id children = widgetRoot[@"children"];
    if (![children isKindOfClass:NSArray.class]) {
        CGRect rootFrame = XCWAXSerializedNodeFrame(widgetRoot);
        return XCWAXSerializedFrameIsValid(rootFrame) ? rootFrame : CGRectNull;
    }

    CGRect fallbackFrame = CGRectNull;
    double fallbackScore = DBL_MAX;
    for (NSDictionary *child in (NSArray *)children) {
        if (![child isKindOfClass:NSDictionary.class]) {
            continue;
        }

        CGRect frame = XCWAXSerializedNodeFrame(child);
        if (!XCWAXSerializedFrameIsValid(frame)) {
            continue;
        }

        NSString *label = XCWAXSerializedNodeText(child);
        if (anchorLabel.length > 0 && [label isEqualToString:anchorLabel]) {
            return frame;
        }

        double score = XCWAXWidgetAnchorScore(frame, targetFrame);
        if (score < fallbackScore) {
            fallbackScore = score;
            fallbackFrame = frame;
        }
    }

    return XCWAXSerializedFrameIsValid(fallbackFrame) ? fallbackFrame : XCWAXSerializedNodeFrame(widgetRoot);
}

static void XCWAXNormalizeWidgetRendererRootFrames(NSMutableArray<NSDictionary *> *serializedRootItems) {
    NSMutableArray<NSDictionary *> *anchors = [NSMutableArray array];
    for (NSDictionary *item in serializedRootItems) {
        NSDictionary *candidate = item[@"candidate"];
        NSDictionary *root = item[@"root"];
        if (XCWAXSerializedRootLooksLikeWidgetRenderer(root, candidate)) {
            continue;
        }
        if ([root isKindOfClass:NSDictionary.class]) {
            XCWAXCollectSerializedFrameAnchors(root, anchors);
        }
    }
    if (anchors.count == 0) {
        return;
    }

    for (NSDictionary *item in serializedRootItems) {
        NSDictionary *candidate = item[@"candidate"];
        NSMutableDictionary *root = item[@"root"];
        if (![root isKindOfClass:NSMutableDictionary.class] || !XCWAXSerializedRootLooksLikeWidgetRenderer(root, candidate)) {
            continue;
        }

        id children = root[@"children"];
        if (![children isKindOfClass:NSArray.class] || [(NSArray *)children count] == 0) {
            continue;
        }

        NSMutableSet<NSNumber *> *usedAnchorIndexes = [NSMutableSet set];
        NSUInteger mappedCount = 0;
        for (NSMutableDictionary *child in (NSArray *)children) {
            if (![child isKindOfClass:NSMutableDictionary.class]) {
                continue;
            }
            CGRect sourceFrame = XCWAXSerializedNodeFrame(child);
            NSUInteger anchorIndex = XCWAXBestWidgetAnchorIndex(child, anchors, usedAnchorIndexes);
            if (anchorIndex == NSNotFound || !XCWAXSerializedFrameIsValid(sourceFrame)) {
                continue;
            }

            [usedAnchorIndexes addObject:@(anchorIndex)];
            CGRect targetFrame = [anchors[anchorIndex][@"frame"] rectValue];
            XCWAXMapSerializedNodeFramesFromLocalToScreen(child, sourceFrame, targetFrame);
            mappedCount += 1;
            XCWAXDebugLog(@"normalized widget renderer child %@ from %@ to %@",
                          XCWAXSerializedNodeText(child),
                          NSStringFromRect(sourceFrame),
                          NSStringFromRect(targetFrame));
        }

        if (mappedCount == 0) {
            continue;
        }
        CGRect unionFrame = XCWAXUnionSerializedNodeFrames(children);
        if (XCWAXSerializedFrameIsValid(unionFrame)) {
            XCWAXSetSerializedNodeFrame(root, unionFrame);
            XCWAXDebugLog(@"normalized widget renderer root %@ to %@", XCWAXSerializedNodeText(root), NSStringFromRect(unionFrame));
        }
    }
}

static void XCWAXNormalizeWidgetRendererPointFrames(NSMutableArray<NSDictionary *> *serializedRootItems, id translator, NSString *udid, NSValue *pointValue, NSString *token) {
    if (pointValue == nil) {
        return;
    }

    CGPoint point = pointValue.pointValue;
    for (NSDictionary *item in serializedRootItems) {
        NSDictionary *candidate = item[@"candidate"];
        NSMutableDictionary *root = item[@"root"];
        if (![root isKindOfClass:NSMutableDictionary.class]) {
            continue;
        }

        pid_t widgetPID = [candidate[@"pid"] intValue];
        if (widgetPID <= 0) {
            widgetPID = XCWAXSerializedNodePID(root);
        }
        if (widgetPID <= 0 || !XCWAXSerializedRootLooksLikeWidgetRenderer(root, candidate)) {
            continue;
        }
        pid_t foregroundPID = XCWAXForegroundUIKitApplicationPID(udid);
        NSMutableDictionary<NSNumber *, NSMutableDictionary *> *anchorCandidatesByKey = [NSMutableDictionary dictionary];
        if (foregroundPID > 0 && foregroundPID != widgetPID) {
            XCWAXStoreRootRecoveryCandidate(anchorCandidatesByKey, XCWAXRootRecoveryCandidate(translator, foregroundPID, candidate[@"displayID"] ?: @0, token));
        }

        if (anchorCandidatesByKey.count == 0) {
            for (NSValue *recoveryPoint in XCWAXRootRecoveryHitTestPoints()) {
                for (NSNumber *displayID in XCWAXCandidateDisplayIDs()) {
                    CGPoint hitPoint = recoveryPoint.pointValue;
                    id hitTranslation = ((id(*)(id, SEL, CGPoint, uint32_t, id))objc_msgSend)(
                        translator,
                        sel_registerName("objectAtPoint:displayId:bridgeDelegateToken:"),
                        hitPoint,
                        displayID.unsignedIntValue,
                        token
                    );
                    pid_t pid = XCWAXTranslationPID(hitTranslation);
                    if (pid <= 0 || pid == widgetPID || anchorCandidatesByKey[@(pid)] != nil) {
                        continue;
                    }
                    XCWAXStoreRootRecoveryCandidate(anchorCandidatesByKey, XCWAXRootRecoveryCandidate(translator, pid, displayID, token));
                }
            }
        }

        NSMutableArray<NSDictionary *> *anchors = [NSMutableArray array];
        for (NSDictionary *anchorCandidate in XCWAXSortedRootRecoveryCandidates(anchorCandidatesByKey)) {
            NSMutableDictionary *anchorRoot = XCWAXSerializeTranslationRoot(translator, anchorCandidate[@"translation"], token, 4);
            if (anchorRoot == nil || XCWAXSerializedRootLooksLikeWidgetRenderer(anchorRoot, anchorCandidate)) {
                continue;
            }
            XCWAXCollectSerializedFrameAnchors(anchorRoot, anchors);
        }

        NSUInteger anchorIndex = XCWAXBestWidgetAnchorIndexContainingPoint(anchors, point);
        if (anchorIndex == NSNotFound) {
            continue;
        }

        CGRect targetFrame = [anchors[anchorIndex][@"frame"] rectValue];
        NSString *anchorLabel = anchors[anchorIndex][@"label"];
        NSMutableDictionary *widgetCandidate = XCWAXRootRecoveryCandidate(translator, widgetPID, candidate[@"displayID"] ?: @0, token);
        NSMutableDictionary *widgetRoot = XCWAXSerializeTranslationRoot(translator, widgetCandidate[@"translation"], token, 2);
        CGRect sourceFrame = XCWAXWidgetLocalSourceFrameForAnchor(widgetRoot ?: root, anchorLabel, targetFrame);
        if (!XCWAXSerializedFrameIsValid(sourceFrame)) {
            continue;
        }

        XCWAXMapSerializedNodeFramesFromLocalToScreen(root, sourceFrame, targetFrame);
        XCWAXDebugLog(@"normalized widget point root %@ using anchor %@ source=%@ target=%@",
                      XCWAXSerializedNodeText(root),
                      anchorLabel,
                      NSStringFromRect(sourceFrame),
                      NSStringFromRect(targetFrame));
    }
}

@implementation XCWAccessibilityBridge

+ (nullable NSDictionary *)accessibilitySnapshotForSimulatorUDID:(NSString *)udid
                                                         atPoint:(nullable NSValue *)pointValue
                                                        maxDepth:(NSUInteger)maxDepth
                                                 interactiveOnly:(BOOL)interactiveOnly
                                                           error:(NSError * _Nullable __autoreleasing *)error {
    if (![self.class loadAndValidate:error]) {
        return nil;
    }

    NSError *deviceError = nil;
    id device = XCWAXDeviceForUDID(udid, &deviceError);
    if (device == nil) {
        if (error != NULL) {
            *error = deviceError;
        }
        return nil;
    }

    if (XCWAXDeviceState(device) != 3) {
        if (error != NULL) {
            *error = XCWAXError(7, [NSString stringWithFormat:@"Cannot inspect accessibility for %@ because it is not booted.", udid]);
        }
        return nil;
    }

    NSError *translatorError = nil;
    id translator = XCWAXTranslator(&translatorError);
    if (translator == nil) {
        if (error != NULL) {
            *error = translatorError;
        }
        return nil;
    }
    XCWAXEnableTranslator(translator);
    XCWAXDebugLog(@"translator=%@ accessibilityEnabled=%@ supportsDelegateTokens=%@",
                  translator,
                  [translator respondsToSelector:sel_registerName("accessibilityEnabled")] ? @(((BOOL(*)(id, SEL))objc_msgSend)(translator, sel_registerName("accessibilityEnabled"))) : @"unknown",
                  [translator respondsToSelector:sel_registerName("supportsDelegateTokens")] ? @(((BOOL(*)(id, SEL))objc_msgSend)(translator, sel_registerName("supportsDelegateTokens"))) : @"unknown");

    NSString *token = XCWAXAccessibilityToken();
    [XCWAXSharedDispatcher registerDevice:device token:token];
    @try {
        id translation = nil;
        NSArray<NSDictionary *> *rootCandidates = nil;
        NSNumber *resolvedDisplayID = nil;
        for (NSNumber *displayID in XCWAXCandidateDisplayIDs()) {
            uint32_t display = displayID.unsignedIntValue;
            if (pointValue != nil) {
                CGPoint point = pointValue.pointValue;
                translation = ((id(*)(id, SEL, CGPoint, uint32_t, id))objc_msgSend)(
                    translator,
                    sel_registerName("objectAtPoint:displayId:bridgeDelegateToken:"),
                    point,
                    display,
                    token
                );
            } else {
                translation = ((id(*)(id, SEL, uint32_t, id))objc_msgSend)(
                    translator,
                    sel_registerName("frontmostApplicationWithDisplayId:bridgeDelegateToken:"),
                    display,
                    token
                );
            }
            XCWAXDebugLog(@"translation lookup display=%@ result=%@", displayID, translation);
            if (translation != nil) {
                resolvedDisplayID = displayID;
                break;
            }
        }
        // Shallow snapshots power fast agent describe loops. Keep the expensive
        // multi-root recovery pass for full trees, or when frontmost lookup fails.
        BOOL shouldRecoverRoots = pointValue == nil && (translation == nil || (!interactiveOnly && maxDepth > 2));
        if (shouldRecoverRoots) {
            NSMutableDictionary<NSNumber *, NSMutableDictionary *> *candidatesByKey = [NSMutableDictionary dictionary];
            if (translation != nil) {
                NSMutableDictionary *candidate = XCWAXRootRecoveryCandidateFromTranslation(translator, translation, resolvedDisplayID ?: @0, token);
                candidate = XCWAXStoreRootRecoveryCandidate(candidatesByKey, candidate);
                XCWAXDebugLog(@"root recovery frontmost candidate=%@", candidate);
            }

            pid_t foregroundPID = XCWAXForegroundUIKitApplicationPID(udid);
            if (foregroundPID > 0) {
                NSMutableDictionary *candidate = XCWAXRootRecoveryCandidate(translator, foregroundPID, @0, token);
                if (candidate != nil) {
                    candidate = XCWAXStoreRootRecoveryCandidate(candidatesByKey, candidate);
                    XCWAXDebugLog(@"root recovery foreground pid=%d candidate=%@", foregroundPID, candidate);
                }
            }
            for (NSValue *recoveryPoint in XCWAXRootRecoveryHitTestPoints()) {
                for (NSNumber *displayID in XCWAXCandidateDisplayIDs()) {
                    uint32_t display = displayID.unsignedIntValue;
                    CGPoint point = recoveryPoint.pointValue;
                    id hitTranslation = ((id(*)(id, SEL, CGPoint, uint32_t, id))objc_msgSend)(
                        translator,
                        sel_registerName("objectAtPoint:displayId:bridgeDelegateToken:"),
                        point,
                        display,
                        token
                    );
                    XCWAXDebugLog(@"root recovery hit-test point=%@ display=%@ result=%@", recoveryPoint, displayID, hitTranslation);
                    pid_t pid = XCWAXTranslationPID(hitTranslation);
                    if (pid <= 0) {
                        continue;
                    }
                    NSMutableDictionary *candidate = candidatesByKey[@(pid)];
                    if (candidate == nil) {
                        candidate = XCWAXRootRecoveryCandidate(translator, pid, displayID, token);
                        XCWAXDebugLog(@"root recovery pid=%d candidate=%@", pid, candidate);
                        if (candidate == nil) {
                            continue;
                        }
                        candidate = XCWAXStoreRootRecoveryCandidate(candidatesByKey, candidate);
                    }
                    candidate[@"hitCount"] = @([candidate[@"hitCount"] unsignedIntegerValue] + 1);
                }
            }

            NSArray<NSDictionary *> *candidates = XCWAXSortedRootRecoveryCandidates(candidatesByKey);
            if (candidates.count > 0) {
                rootCandidates = candidates;
                NSDictionary *primaryCandidate = candidates.firstObject;
                translation = primaryCandidate[@"translation"];
                resolvedDisplayID = primaryCandidate[@"displayID"];
                XCWAXDebugLog(@"root recovery selected primary pid=%@ area=%@ children=%@ hits=%@ extension=%@ path=%@ roots=%lu",
                              primaryCandidate[@"pid"],
                              primaryCandidate[@"area"],
                              primaryCandidate[@"childCount"],
                              primaryCandidate[@"hitCount"],
                              primaryCandidate[@"isExtension"],
                              primaryCandidate[@"processPath"],
                              (unsigned long)candidates.count);
            }
        }

        if (translation == nil) {
            XCWAXDebugLog(@"translation lookup returned nil point=%@", pointValue);
            if (error != NULL) {
                *error = XCWAXError(9, @"No application accessibility root returned for simulator. The simulator may be between lifecycle states or hidden by a fullscreen dialog.");
            }
            return nil;
        }
        XCWAXSetBridgeDelegateTokenOnTranslation(translation, token);
        XCWAXDebugLog(@"using accessibility display %@", resolvedDisplayID);

        NSMutableDictionary *singleRootCandidate = nil;
        if (rootCandidates == nil) {
            singleRootCandidate = XCWAXRootRecoveryCandidateFromTranslation(translator, translation, resolvedDisplayID ?: @0, token);
        }
        NSArray<NSDictionary *> *rootItems = rootCandidates ?: @[singleRootCandidate ?: @{ @"translation": translation }];
        NSMutableArray<NSDictionary *> *serializedRootItems = [NSMutableArray arrayWithCapacity:rootItems.count];
        for (NSDictionary *rootItem in rootItems) {
            id rootTranslation = rootItem[@"translation"];
            XCWAXSetBridgeDelegateTokenOnTranslation(rootTranslation, token);
            id element = XCWAXMacPlatformElementFromTranslation(translator, rootTranslation);
            if (element == nil) {
                XCWAXDebugLog(@"skipping accessibility root because translation could not become a platform element: %@", rootTranslation);
                continue;
            }

            NSHashTable *visited = [NSHashTable hashTableWithOptions:NSPointerFunctionsObjectPointerPersonality];
            NSMutableDictionary *root = XCWAXSerializeElementWithOptions(element, token, visited, 0, MIN(maxDepth, XCWAXMaxDepth), interactiveOnly);
            if (root != nil) {
                XCWAXApplyRecoveredRootMetadata(root, rootItem);
                [serializedRootItems addObject:@{
                    @"root": root,
                    @"candidate": rootItem,
                }];
            }
        }

        XCWAXNormalizeWidgetRendererRootFrames(serializedRootItems);
        XCWAXNormalizeWidgetRendererPointFrames(serializedRootItems, translator, udid, pointValue, token);

        NSMutableArray *roots = [NSMutableArray arrayWithCapacity:serializedRootItems.count];
        for (NSDictionary *item in serializedRootItems) {
            id root = item[@"root"];
            if (root != nil) {
                [roots addObject:root];
            }
        }

        if (roots.count == 0) {
            if (error != NULL) {
                *error = XCWAXError(10, @"Unable to create a macOS accessibility platform element from the simulator translation object.");
            }
            return nil;
        }
        return @{
            @"roots": roots,
            @"source": @"native-ax",
        };
    } @finally {
        [XCWAXSharedDispatcher unregisterToken:token];
    }
}

+ (BOOL)loadAndValidate:(NSError **)error {
    if (!XCWAXLoadPrivateFrameworks(error)) {
        return NO;
    }
    Class translatorClass = NSClassFromString(@"AXPTranslator");
    if (translatorClass == Nil) {
        if (error != NULL) {
            *error = XCWAXError(11, @"AccessibilityPlatformTranslation did not expose AXPTranslator.");
        }
        return NO;
    }
    return YES;
}

@end
