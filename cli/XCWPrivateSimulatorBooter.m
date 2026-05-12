#import "XCWPrivateSimulatorBooter.h"

#import <dlfcn.h>
#import <limits.h>
#import <objc/message.h>
#import <objc/runtime.h>

static NSString * const XCWPrivateSimulatorBooterErrorDomain = @"SimDeck.PrivateSimulatorBooter";
static NSString * const XCWCoreSimulatorPath = @"/Library/Developer/PrivateFrameworks/CoreSimulator.framework/CoreSimulator";

typedef NS_ENUM(NSInteger, XCWPrivateSimulatorBooterErrorCode) {
    XCWPrivateSimulatorBooterErrorCodeFrameworkLoadFailed = 1,
    XCWPrivateSimulatorBooterErrorCodeServiceContextFailed = 2,
    XCWPrivateSimulatorBooterErrorCodeDeviceLookupFailed = 3,
    XCWPrivateSimulatorBooterErrorCodeBootFailed = 4,
};

static NSError *XCWPrivateSimulatorBooterMakeError(XCWPrivateSimulatorBooterErrorCode code, NSString *description) {
    return [NSError errorWithDomain:XCWPrivateSimulatorBooterErrorDomain
                               code:code
                           userInfo:@{
        NSLocalizedDescriptionKey: description,
    }];
}

static NSString *XCWActiveDeveloperDirectory(void) {
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

static id XCWCreateCoreSimulatorServiceContext(NSError **error) {
    Class serviceContextClass = NSClassFromString(@"SimServiceContext");
    if (serviceContextClass == Nil) {
        if (error != NULL) {
            *error = XCWPrivateSimulatorBooterMakeError(
                XCWPrivateSimulatorBooterErrorCodeServiceContextFailed,
                @"CoreSimulator did not expose SimServiceContext."
            );
        }
        return nil;
    }

    NSString *developerDir = XCWActiveDeveloperDirectory();
    NSError *serviceError = nil;
    SEL sharedSelector = sel_registerName("sharedServiceContextForDeveloperDir:error:");
    if ([serviceContextClass respondsToSelector:sharedSelector]) {
        id context = ((id(*)(id, SEL, id, NSError **))objc_msgSend)(
            serviceContextClass,
            sharedSelector,
            developerDir,
            &serviceError
        );
        if (context != nil) {
            return context;
        }
    }

    serviceError = nil;
    SEL initSelector = sel_registerName("initWithDeveloperDir:connectionType:error:");
    id contextAlloc = ((id(*)(id, SEL))objc_msgSend)(serviceContextClass, sel_registerName("alloc"));
    if (![contextAlloc respondsToSelector:initSelector]) {
        if (error != NULL) {
            *error = XCWPrivateSimulatorBooterMakeError(
                XCWPrivateSimulatorBooterErrorCodeServiceContextFailed,
                @"CoreSimulator did not expose a supported SimServiceContext initializer."
            );
        }
        return nil;
    }
    id context = ((id(*)(id, SEL, id, long long, NSError **))objc_msgSend)(
        contextAlloc,
        initSelector,
        developerDir,
        0LL,
        &serviceError
    );
    if (context == nil && error != NULL) {
        *error = serviceError ?: XCWPrivateSimulatorBooterMakeError(
            XCWPrivateSimulatorBooterErrorCodeServiceContextFailed,
            [NSString stringWithFormat:@"Unable to create a CoreSimulator service context for %@.", developerDir]
        );
    }
    return context;
}

static NSArray *XCWFlattenCoreSimulatorDevices(id devicesPayload) {
    if ([devicesPayload isKindOfClass:[NSArray class]]) {
        return devicesPayload;
    }
    if ([devicesPayload isKindOfClass:[NSSet class]]) {
        return [devicesPayload allObjects];
    }
    if ([devicesPayload isKindOfClass:[NSDictionary class]]) {
        NSMutableArray *devices = [NSMutableArray array];
        for (id value in [(NSDictionary *)devicesPayload allValues]) {
            [devices addObjectsFromArray:XCWFlattenCoreSimulatorDevices(value)];
        }
        return devices;
    }
    return @[];
}

static NSArray *XCWDevicesForDeviceSet(id deviceSet) {
    SEL availableSelector = sel_registerName("availableDevices");
    if ([deviceSet respondsToSelector:availableSelector]) {
        NSArray *availableDevices = XCWFlattenCoreSimulatorDevices(((id(*)(id, SEL))objc_msgSend)(deviceSet, availableSelector));
        if (availableDevices.count > 0) {
            return availableDevices;
        }
    }

    SEL devicesSelector = sel_registerName("devices");
    if ([deviceSet respondsToSelector:devicesSelector]) {
        return XCWFlattenCoreSimulatorDevices(((id(*)(id, SEL))objc_msgSend)(deviceSet, devicesSelector));
    }
    return @[];
}

static NSString *XCWUDIDForDevice(id device) {
    id deviceUDID = ((id(*)(id, SEL))objc_msgSend)(device, sel_registerName("UDID"));
    if ([deviceUDID respondsToSelector:sel_registerName("UUIDString")]) {
        return ((id(*)(id, SEL))objc_msgSend)(deviceUDID, sel_registerName("UUIDString"));
    }
    return [deviceUDID description];
}

static BOOL XCWPrivateBootErrorMeansAlreadyBooted(NSError *error) {
    NSString *message = error.localizedDescription.lowercaseString ?: @"";
    return [message containsString:@"already booted"] || [message containsString:@"current state: booted"];
}

@implementation XCWPrivateSimulatorBooter

+ (BOOL)bootDeviceWithUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    static dispatch_once_t onceToken;
    static NSError *frameworkError = nil;

    dispatch_once(&onceToken, ^{
        if (!dlopen(XCWCoreSimulatorPath.fileSystemRepresentation, RTLD_NOW | RTLD_GLOBAL)) {
            frameworkError = XCWPrivateSimulatorBooterMakeError(
                XCWPrivateSimulatorBooterErrorCodeFrameworkLoadFailed,
                [NSString stringWithFormat:@"Unable to load CoreSimulator from %@.", XCWCoreSimulatorPath]
            );
        }
    });

    if (frameworkError != nil) {
        if (error != NULL) {
            *error = frameworkError;
        }
        return NO;
    }

    NSError *serviceError = nil;
    id serviceContext = XCWCreateCoreSimulatorServiceContext(&serviceError);
    if (serviceContext == nil) {
        if (error != NULL) {
            *error = serviceError ?: XCWPrivateSimulatorBooterMakeError(
                XCWPrivateSimulatorBooterErrorCodeServiceContextFailed,
                @"Unable to create a CoreSimulator service context."
            );
        }
        return NO;
    }

    NSError *deviceSetError = nil;
    id deviceSet = ((id(*)(id, SEL, NSError **))objc_msgSend)(serviceContext, sel_registerName("defaultDeviceSetWithError:"), &deviceSetError);
    if (deviceSet == nil) {
        if (error != NULL) {
            *error = deviceSetError ?: XCWPrivateSimulatorBooterMakeError(
                XCWPrivateSimulatorBooterErrorCodeServiceContextFailed,
                @"Unable to access the default CoreSimulator device set."
            );
        }
        return NO;
    }

    id targetDevice = nil;
    NSArray *devices = XCWDevicesForDeviceSet(deviceSet);
    for (id candidate in devices) {
        if ([XCWUDIDForDevice(candidate) isEqualToString:udid]) {
            targetDevice = candidate;
            break;
        }
    }

    if (targetDevice == nil) {
        if (error != NULL) {
            *error = XCWPrivateSimulatorBooterMakeError(
                XCWPrivateSimulatorBooterErrorCodeDeviceLookupFailed,
                [NSString stringWithFormat:@"Unable to locate simulator %@ inside the CoreSimulator device set.", udid]
            );
        }
        return NO;
    }

    NSError *bootError = nil;
    BOOL booted = NO;
    SEL bootWithOptionsSelector = sel_registerName("bootWithOptions:error:");
    if ([targetDevice respondsToSelector:bootWithOptionsSelector]) {
        booted = ((BOOL(*)(id, SEL, id, NSError **))objc_msgSend)(
            targetDevice,
            bootWithOptionsSelector,
            @{ @"persist": @YES },
            &bootError
        );
    } else {
        bootError = XCWPrivateSimulatorBooterMakeError(
            XCWPrivateSimulatorBooterErrorCodeBootFailed,
            @"CoreSimulator device did not expose bootWithOptions:error:."
        );
    }

    if (!booted) {
        if (XCWPrivateBootErrorMeansAlreadyBooted(bootError)) {
            return YES;
        }

        SEL bootWithErrorSelector = sel_registerName("bootWithError:");
        if ([targetDevice respondsToSelector:bootWithErrorSelector]) {
            NSError *legacyBootError = nil;
            booted = ((BOOL(*)(id, SEL, NSError **))objc_msgSend)(
                targetDevice,
                bootWithErrorSelector,
                &legacyBootError
            );
            if (booted || XCWPrivateBootErrorMeansAlreadyBooted(legacyBootError)) {
                return YES;
            }
            if (legacyBootError != nil) {
                bootError = legacyBootError;
            }
        }

        if (error != NULL) {
            *error = bootError ?: XCWPrivateSimulatorBooterMakeError(
                XCWPrivateSimulatorBooterErrorCodeBootFailed,
                [NSString stringWithFormat:@"Private CoreSimulator boot failed for %@.", udid]
            );
        }
        return NO;
    }

    return YES;
}

@end
