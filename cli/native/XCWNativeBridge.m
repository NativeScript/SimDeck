#import "XCWNativeBridge.h"

#import "XCWChromeRenderer.h"
#import "XCWNativeSession.h"
#import "XCWSimctl.h"

#import <AppKit/AppKit.h>
#import <CoreFoundation/CoreFoundation.h>
#include <stdlib.h>
#include <string.h>

static NSString *XCWStringFromCString(const char *value) {
    if (value == NULL) {
        return @"";
    }
    return [NSString stringWithUTF8String:value] ?: @"";
}

static char *XCWCopyCString(NSString *string) {
    NSData *data = [[string ?: @"" dataUsingEncoding:NSUTF8StringEncoding] copy];
    char *buffer = calloc(data.length + 1, sizeof(char));
    if (buffer == NULL) {
        return NULL;
    }
    memcpy(buffer, data.bytes, data.length);
    buffer[data.length] = '\0';
    return buffer;
}

static void XCWSetErrorMessage(char **errorMessage, NSError *error) {
    if (errorMessage == NULL) {
        return;
    }
    *errorMessage = XCWCopyCString(error.localizedDescription ?: @"Unknown native error.");
}

static char *XCWJSONStringFromObject(id object, char **errorMessage) {
    NSError *jsonError = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:0 error:&jsonError];
    if (data == nil) {
        XCWSetErrorMessage(errorMessage, jsonError);
        return NULL;
    }

    NSString *string = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] ?: @"{}";
    return XCWCopyCString(string);
}

static xcw_native_owned_bytes XCWOwnedBytesFromData(NSData *data) {
    xcw_native_owned_bytes bytes = {0};
    if (data.length == 0) {
        return bytes;
    }

    bytes.data = malloc(data.length);
    if (bytes.data == NULL) {
        return (xcw_native_owned_bytes){0};
    }
    memcpy(bytes.data, data.bytes, data.length);
    bytes.length = data.length;
    return bytes;
}

static XCWNativeSession *XCWNativeSessionFromHandle(void *handle) {
    return (__bridge XCWNativeSession *)handle;
}

void xcw_native_initialize_app(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
    }
}

void xcw_native_run_main_loop_slice(double duration_seconds) {
    @autoreleasepool {
        if (duration_seconds <= 0) {
            duration_seconds = 0.01;
        }
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:duration_seconds];
        [[NSRunLoop mainRunLoop] runUntilDate:deadline];
    }
}

char *xcw_native_list_simulators(char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSArray<NSDictionary *> *simulators = [simctl listSimulatorsWithError:&error];
        if (simulators == nil) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
        return XCWJSONStringFromObject(@{ @"simulators": simulators }, error_message);
    }
}

bool xcw_native_boot_simulator(const char *udid, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        BOOL ok = [simctl bootSimulatorWithUDID:XCWStringFromCString(udid) error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_shutdown_simulator(const char *udid, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        BOOL ok = [simctl shutdownSimulatorWithUDID:XCWStringFromCString(udid) error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_open_url(const char *udid, const char *url, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        BOOL ok = [simctl openURL:XCWStringFromCString(url)
                    simulatorUDID:XCWStringFromCString(udid)
                            error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_launch_bundle(const char *udid, const char *bundle_id, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        BOOL ok = [simctl launchBundleID:XCWStringFromCString(bundle_id)
                           simulatorUDID:XCWStringFromCString(udid)
                                   error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

char *xcw_native_get_chrome_profile(const char *udid, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *lookupError = nil;
        NSDictionary *simulator = [simctl simulatorWithUDID:XCWStringFromCString(udid) error:&lookupError];
        if (simulator == nil) {
            XCWSetErrorMessage(error_message, lookupError);
            return NULL;
        }

        NSError *profileError = nil;
        NSDictionary *profile = [XCWChromeRenderer profileForDeviceName:simulator[@"name"] ?: @""
                                                                  error:&profileError];
        if (profile == nil) {
            XCWSetErrorMessage(error_message, profileError);
            return NULL;
        }

        return XCWJSONStringFromObject(profile, error_message);
    }
}

xcw_native_owned_bytes xcw_native_render_chrome_png(const char *udid, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *lookupError = nil;
        NSDictionary *simulator = [simctl simulatorWithUDID:XCWStringFromCString(udid) error:&lookupError];
        if (simulator == nil) {
            XCWSetErrorMessage(error_message, lookupError);
            return (xcw_native_owned_bytes){0};
        }

        NSError *renderError = nil;
        NSData *pngData = [XCWChromeRenderer PNGDataForDeviceName:simulator[@"name"] ?: @""
                                                            error:&renderError];
        if (pngData == nil) {
            XCWSetErrorMessage(error_message, renderError);
            return (xcw_native_owned_bytes){0};
        }

        return XCWOwnedBytesFromData(pngData);
    }
}

void *xcw_native_session_create(const char *udid, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        XCWNativeSession *session = [[XCWNativeSession alloc] initWithUDID:XCWStringFromCString(udid)
                                                                     error:&error];
        if (session == nil) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
        return (__bridge_retained void *)session;
    }
}

void xcw_native_session_destroy(void *handle) {
    @autoreleasepool {
        if (handle == NULL) {
            return;
        }
        XCWNativeSession *session = CFBridgingRelease(handle);
        [session disconnect];
    }
}

bool xcw_native_session_start(void *handle, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) start:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

void xcw_native_session_request_refresh(void *handle) {
    @autoreleasepool {
        [XCWNativeSessionFromHandle(handle) requestRefresh];
    }
}

bool xcw_native_session_send_touch(void *handle, double x, double y, const char *phase, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) sendTouchAtX:x
                                                                 y:y
                                                             phase:XCWStringFromCString(phase)
                                                             error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_session_send_key(void *handle, uint16_t key_code, uint32_t modifiers, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) sendKeyCode:key_code
                                                        modifiers:modifiers
                                                            error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_session_press_home(void *handle, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) pressHome:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_session_rotate_right(void *handle, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) rotateRight:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

void xcw_native_session_set_frame_callback(void *handle, xcw_native_frame_callback callback, void *user_data) {
    @autoreleasepool {
        [XCWNativeSessionFromHandle(handle) setFrameCallback:callback userData:user_data];
    }
}

void xcw_native_free_string(char *value) {
    if (value != NULL) {
        free(value);
    }
}

void xcw_native_free_bytes(xcw_native_owned_bytes bytes) {
    if (bytes.data != NULL) {
        free(bytes.data);
    }
}

void xcw_native_release_shared_bytes(xcw_native_shared_bytes bytes) {
    if (bytes.owner != NULL) {
        CFRelease(bytes.owner);
    }
}
