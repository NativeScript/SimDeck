#import "XCWAccessibilityBridge.h"

#import <AppKit/AppKit.h>
#import <dlfcn.h>
#import <objc/message.h>
#import <objc/runtime.h>

static NSString * const XCWAccessibilityBridgeErrorDomain = @"XcodeCanvasWeb.AccessibilityBridge";
static NSString * const XCWCoreSimulatorPath = @"/Library/Developer/PrivateFrameworks/CoreSimulator.framework/CoreSimulator";
static NSString * const XCWAccessibilityPlatformTranslationPath = @"/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework/AccessibilityPlatformTranslation";
static const NSUInteger XCWAXMaxDepth = 80;
static NSObject *XCWAXDeviceCacheLock = nil;
static id XCWAXCachedServiceContext = nil;
static id XCWAXCachedDeviceSet = nil;
static NSMutableDictionary<NSString *, id> *XCWAXCachedDevicesByUDID = nil;

typedef id _Nullable (^XCWAXTranslationCallback)(id request);

static NSError *XCWAXError(NSInteger code, NSString *description) {
    return [NSError errorWithDomain:XCWAccessibilityBridgeErrorDomain
                               code:code
                           userInfo:@{ NSLocalizedDescriptionKey: description }];
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
            NSError *serviceError = nil;
            id contextAlloc = ((id(*)(id, SEL))objc_msgSend)(serviceContextClass, sel_registerName("alloc"));
            XCWAXCachedServiceContext = ((id(*)(id, SEL, id, long long, NSError **))objc_msgSend)(
                contextAlloc,
                sel_registerName("initWithDeveloperDir:connectionType:error:"),
                nil,
                0LL,
                &serviceError
            );
            if (XCWAXCachedServiceContext == nil) {
                if (error != NULL) {
                    *error = serviceError ?: XCWAXError(4, @"Unable to create a CoreSimulator service context.");
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

        NSArray *devices = ((id(*)(id, SEL))objc_msgSend)(XCWAXCachedDeviceSet, sel_registerName("devices"));
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
    _callbackQueue = dispatch_queue_create("com.xcodecanvasweb.accessibility.callback", DISPATCH_QUEUE_SERIAL);
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
    return ((id(*)(id, SEL))objc_msgSend)(object, selector);
}

static BOOL XCWAXBool(id object, const char *selectorName) {
    SEL selector = sel_registerName(selectorName);
    if (object == nil || ![object respondsToSelector:selector]) {
        return NO;
    }
    return ((BOOL(*)(id, SEL))objc_msgSend)(object, selector);
}

static CGRect XCWAXFrame(id object) {
    SEL selector = sel_registerName("accessibilityFrame");
    if (object == nil || ![object respondsToSelector:selector]) {
        return CGRectZero;
    }
    return ((CGRect(*)(id, SEL))objc_msgSend)(object, selector);
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

static NSMutableDictionary *XCWAXSerializeElement(id element, NSString *token, NSHashTable *visited, NSUInteger depth) {
    if (element == nil || depth > XCWAXMaxDepth || [visited containsObject:element]) {
        return nil;
    }
    [visited addObject:element];

    id translation = XCWAXObject(element, "translation");
    if (translation != nil && [translation respondsToSelector:sel_registerName("setBridgeDelegateToken:")]) {
        ((void(*)(id, SEL, id))objc_msgSend)(translation, sel_registerName("setBridgeDelegateToken:"), token);
    }

    NSMutableDictionary *values = [XCWAXDictionaryForElement(element) mutableCopy];
    NSMutableArray *childrenValues = [NSMutableArray array];
    id children = XCWAXObject(element, "accessibilityChildren");
    if ([children isKindOfClass:NSArray.class]) {
        for (id child in (NSArray *)children) {
            NSMutableDictionary *childValues = XCWAXSerializeElement(child, token, visited, depth + 1);
            if (childValues != nil) {
                [childrenValues addObject:childValues];
            }
        }
    }
    values[@"children"] = childrenValues;
    return values;
}

@implementation XCWAccessibilityBridge

+ (nullable NSDictionary *)accessibilitySnapshotForSimulatorUDID:(NSString *)udid
                                                         atPoint:(nullable NSValue *)pointValue
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

    Class translatorClass = NSClassFromString(@"AXPTranslator");
    id translator = [translatorClass respondsToSelector:sel_registerName("sharedInstance")]
        ? ((id(*)(id, SEL))objc_msgSend)(translatorClass, sel_registerName("sharedInstance"))
        : nil;
    if (translator == nil) {
        if (error != NULL) {
            *error = XCWAXError(8, @"AccessibilityPlatformTranslation did not expose AXPTranslator.sharedInstance.");
        }
        return nil;
    }

    XCWAccessibilityTranslationDispatcher *dispatcher = [[XCWAccessibilityTranslationDispatcher alloc] initWithTranslator:translator];
    if ([translator respondsToSelector:sel_registerName("setBridgeTokenDelegate:")]) {
        ((void(*)(id, SEL, id))objc_msgSend)(translator, sel_registerName("setBridgeTokenDelegate:"), dispatcher);
    }

    NSString *token = NSUUID.UUID.UUIDString;
    [dispatcher registerDevice:device token:token];
    @try {
        id translation = nil;
        if (pointValue != nil) {
            CGPoint point = pointValue.pointValue;
            translation = ((id(*)(id, SEL, CGPoint, int, id))objc_msgSend)(
                translator,
                sel_registerName("objectAtPoint:displayId:bridgeDelegateToken:"),
                point,
                0,
                token
            );
        } else {
            translation = ((id(*)(id, SEL, int, id))objc_msgSend)(
                translator,
                sel_registerName("frontmostApplicationWithDisplayId:bridgeDelegateToken:"),
                0,
                token
            );
        }

        if (translation == nil) {
            if (error != NULL) {
                *error = XCWAXError(9, @"No translation object returned for simulator. The point may be invalid or hidden by a fullscreen dialog.");
            }
            return nil;
        }
        if ([translation respondsToSelector:sel_registerName("setBridgeDelegateToken:")]) {
            ((void(*)(id, SEL, id))objc_msgSend)(translation, sel_registerName("setBridgeDelegateToken:"), token);
        }

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
        NSMutableDictionary *root = XCWAXSerializeElement(element, token, visited, 0);
        NSArray *roots = root != nil ? @[root] : @[];
        return @{
            @"roots": roots,
            @"source": @"native-ax",
        };
    } @finally {
        [dispatcher unregisterToken:token];
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
