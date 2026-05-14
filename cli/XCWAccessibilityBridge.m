#import "XCWAccessibilityBridge.h"

#import <AppKit/AppKit.h>
#import <dlfcn.h>
#import <limits.h>
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

static NSMutableDictionary *XCWAXSerializeElement(id element, NSString *token, NSHashTable *visited, NSUInteger depth, NSUInteger maxDepth) {
    if (element == nil || depth > maxDepth || [visited containsObject:element]) {
        return nil;
    }
    [visited addObject:element];

    id translation = XCWAXObject(element, "translation");
    if (translation != nil && [translation respondsToSelector:sel_registerName("setBridgeDelegateToken:")]) {
        ((void(*)(id, SEL, id))objc_msgSend)(translation, sel_registerName("setBridgeDelegateToken:"), token);
    }

    NSMutableDictionary *values = [XCWAXDictionaryForElement(element) mutableCopy];
    NSMutableArray *childrenValues = [NSMutableArray array];
    id children = depth < maxDepth ? XCWAXObject(element, "accessibilityChildren") : nil;
    if ([children isKindOfClass:NSArray.class]) {
        for (id child in (NSArray *)children) {
            NSMutableDictionary *childValues = XCWAXSerializeElement(child, token, visited, depth + 1, maxDepth);
            if (childValues != nil) {
                [childrenValues addObject:childValues];
            }
        }
    }
    values[@"children"] = childrenValues;
    return values;
}

static NSArray<NSValue *> *XCWAXFallbackHitTestPoints(void) {
    NSMutableArray<NSValue *> *points = [NSMutableArray array];
    NSArray<NSNumber *> *xValues = @[@24, @100, @220, @340, @420];
    NSArray<NSNumber *> *yValues = @[@80, @150, @220, @300, @380, @460, @540, @620, @700, @780, @860, @930];
    for (NSNumber *yValue in yValues) {
        for (NSNumber *xValue in xValues) {
            [points addObject:[NSValue valueWithPoint:CGPointMake(xValue.doubleValue, yValue.doubleValue)]];
        }
    }
    return points;
}

static NSString *XCWAXElementIdentity(NSDictionary *element) {
    id identifier = element[@"AXUniqueId"];
    id label = element[@"AXLabel"];
    id role = element[@"role"];
    id frame = element[@"AXFrame"];
    return [@[role ?: @"", identifier ?: @"", label ?: @"", frame ?: @""] componentsJoinedByString:@"|"];
}

@implementation XCWAccessibilityBridge

+ (nullable NSDictionary *)accessibilitySnapshotForSimulatorUDID:(NSString *)udid
                                                         atPoint:(nullable NSValue *)pointValue
                                                        maxDepth:(NSUInteger)maxDepth
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
        if (translation == nil && pointValue == nil) {
            NSMutableArray *fallbackRoots = [NSMutableArray array];
            NSMutableSet<NSString *> *seenElements = [NSMutableSet set];
            for (NSValue *fallbackPoint in XCWAXFallbackHitTestPoints()) {
                for (NSNumber *displayID in XCWAXCandidateDisplayIDs()) {
                    uint32_t display = displayID.unsignedIntValue;
                    CGPoint point = fallbackPoint.pointValue;
                    id fallbackTranslation = ((id(*)(id, SEL, CGPoint, uint32_t, id))objc_msgSend)(
                        translator,
                        sel_registerName("objectAtPoint:displayId:bridgeDelegateToken:"),
                        point,
                        display,
                        token
                    );
                    XCWAXDebugLog(@"fallback translation lookup point=%@ display=%@ result=%@", fallbackPoint, displayID, fallbackTranslation);
                    if (fallbackTranslation == nil) {
                        continue;
                    }
                    if ([fallbackTranslation respondsToSelector:sel_registerName("setBridgeDelegateToken:")]) {
                        ((void(*)(id, SEL, id))objc_msgSend)(fallbackTranslation, sel_registerName("setBridgeDelegateToken:"), token);
                    }
                    id fallbackElement = ((id(*)(id, SEL, id))objc_msgSend)(
                        translator,
                        sel_registerName("macPlatformElementFromTranslation:"),
                        fallbackTranslation
                    );
                    NSHashTable *visited = [NSHashTable hashTableWithOptions:NSPointerFunctionsObjectPointerPersonality];
                    NSMutableDictionary *root = XCWAXSerializeElement(fallbackElement, token, visited, 0, MIN(maxDepth, XCWAXMaxDepth));
                    NSString *identity = root != nil ? XCWAXElementIdentity(root) : @"";
                    if (identity.length > 0 && ![seenElements containsObject:identity]) {
                        [seenElements addObject:identity];
                        [fallbackRoots addObject:root];
                    }
                }
            }
            if (fallbackRoots.count > 0) {
                NSArray *rootsWithChildren = [fallbackRoots filteredArrayUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(NSDictionary *root, NSDictionary *bindings) {
                    (void)bindings;
                    NSArray *children = [root[@"children"] isKindOfClass:NSArray.class] ? root[@"children"] : @[];
                    return children.count > 0;
                }]];
                NSArray *roots = rootsWithChildren.count > 0 ? rootsWithChildren : fallbackRoots;
                XCWAXDebugLog(@"frontmost lookup failed; returning %lu sampled fallback elements", (unsigned long)roots.count);
                return @{
                    @"roots": roots,
                    @"source": @"native-ax",
                };
            }
        }

        if (translation == nil) {
            XCWAXDebugLog(@"translation lookup returned nil point=%@", pointValue);
            if (error != NULL) {
                *error = XCWAXError(9, @"No translation object returned for simulator. The point may be invalid or hidden by a fullscreen dialog.");
            }
            return nil;
        }
        if ([translation respondsToSelector:sel_registerName("setBridgeDelegateToken:")]) {
            ((void(*)(id, SEL, id))objc_msgSend)(translation, sel_registerName("setBridgeDelegateToken:"), token);
        }
        XCWAXDebugLog(@"using accessibility display %@", resolvedDisplayID);

        id element = ((id(*)(id, SEL, id))objc_msgSend)(
            translator,
            sel_registerName("macPlatformElementFromTranslation:"),
            translation
        );
        if (element == nil) {
            if (error != NULL) {
                *error = XCWAXError(10, @"Unable to create a macOS accessibility platform element from the simulator translation object.");
            }
            return nil;
        }

        NSHashTable *visited = [NSHashTable hashTableWithOptions:NSPointerFunctionsObjectPointerPersonality];
        NSMutableDictionary *root = XCWAXSerializeElement(element, token, visited, 0, MIN(maxDepth, XCWAXMaxDepth));
        NSArray *roots = root != nil ? @[root] : @[];
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
