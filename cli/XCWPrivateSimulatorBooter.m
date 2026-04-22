#import "XCWPrivateSimulatorBooter.h"

#import <dlfcn.h>
#import <objc/message.h>

static NSString * const XCWPrivateSimulatorBooterErrorDomain = @"XcodeCanvasWeb.PrivateSimulatorBooter";
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

    Class serviceContextClass = NSClassFromString(@"SimServiceContext");
    if (serviceContextClass == Nil) {
        if (error != NULL) {
            *error = XCWPrivateSimulatorBooterMakeError(
                XCWPrivateSimulatorBooterErrorCodeServiceContextFailed,
                @"CoreSimulator did not expose SimServiceContext."
            );
        }
        return NO;
    }

    NSError *serviceError = nil;
    id contextAlloc = ((id(*)(id, SEL))objc_msgSend)(serviceContextClass, sel_registerName("alloc"));
    id serviceContext = ((id(*)(id, SEL, id, long long, NSError **))objc_msgSend)(
        contextAlloc,
        sel_registerName("initWithDeveloperDir:connectionType:error:"),
        nil,
        0LL,
        &serviceError
    );
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
    NSArray *devices = ((id(*)(id, SEL))objc_msgSend)(deviceSet, sel_registerName("devices"));
    for (id candidate in devices) {
        id deviceUDID = ((id(*)(id, SEL))objc_msgSend)(candidate, sel_registerName("UDID"));
        NSString *candidateUDID = [deviceUDID respondsToSelector:sel_registerName("UUIDString")]
            ? ((id(*)(id, SEL))objc_msgSend)(deviceUDID, sel_registerName("UUIDString"))
            : [deviceUDID description];
        if ([candidateUDID isEqualToString:udid]) {
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
    BOOL booted = ((BOOL(*)(id, SEL, id, NSError **))objc_msgSend)(
        targetDevice,
        sel_registerName("bootWithOptions:error:"),
        @{},
        &bootError
    );

    if (!booted) {
        NSString *message = bootError.localizedDescription.lowercaseString;
        if ([message containsString:@"already booted"] || [message containsString:@"current state: booted"]) {
            return YES;
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
