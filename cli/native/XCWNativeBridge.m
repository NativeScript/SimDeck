#import "XCWNativeBridge.h"

#import "DFPrivateSimulatorDisplayBridge.h"
#import "XCWAccessibilityBridge.h"
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

static BOOL XCWPerformSimctlAction(char **errorMessage, BOOL (^action)(XCWSimctl *simctl, NSError **error)) {
    XCWSimctl *simctl = [[XCWSimctl alloc] init];
    NSError *error = nil;
    BOOL ok = action(simctl, &error);
    if (!ok) {
        XCWSetErrorMessage(errorMessage, error);
    }
    return ok;
}

static NSDictionary *XCWSimulatorRecordForUDID(const char *udid, char **errorMessage) {
    XCWSimctl *simctl = [[XCWSimctl alloc] init];
    NSError *error = nil;
    NSDictionary *simulator = [simctl simulatorWithUDID:XCWStringFromCString(udid) error:&error];
    if (simulator == nil) {
        XCWSetErrorMessage(errorMessage, error);
    }
    return simulator;
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
        return XCWPerformSimctlAction(error_message, ^BOOL(XCWSimctl *simctl, NSError **error) {
            return [simctl bootSimulatorWithUDID:XCWStringFromCString(udid) error:error];
        });
    }
}

bool xcw_native_shutdown_simulator(const char *udid, char **error_message) {
    @autoreleasepool {
        return XCWPerformSimctlAction(error_message, ^BOOL(XCWSimctl *simctl, NSError **error) {
            return [simctl shutdownSimulatorWithUDID:XCWStringFromCString(udid) error:error];
        });
    }
}

bool xcw_native_toggle_appearance(const char *udid, char **error_message) {
    @autoreleasepool {
        return XCWPerformSimctlAction(error_message, ^BOOL(XCWSimctl *simctl, NSError **error) {
            return [simctl toggleAppearanceForSimulatorUDID:XCWStringFromCString(udid) error:error];
        });
    }
}

bool xcw_native_open_url(const char *udid, const char *url, char **error_message) {
    @autoreleasepool {
        return XCWPerformSimctlAction(error_message, ^BOOL(XCWSimctl *simctl, NSError **error) {
            return [simctl openURL:XCWStringFromCString(url)
                     simulatorUDID:XCWStringFromCString(udid)
                             error:error];
        });
    }
}

bool xcw_native_launch_bundle(const char *udid, const char *bundle_id, char **error_message) {
    @autoreleasepool {
        return XCWPerformSimctlAction(error_message, ^BOOL(XCWSimctl *simctl, NSError **error) {
            return [simctl launchBundleID:XCWStringFromCString(bundle_id)
                            simulatorUDID:XCWStringFromCString(udid)
                                    error:error];
        });
    }
}

char *xcw_native_get_chrome_profile(const char *udid, char **error_message) {
    @autoreleasepool {
        NSDictionary *simulator = XCWSimulatorRecordForUDID(udid, error_message);
        if (simulator == nil) {
            return NULL;
        }

        NSError *profileError = nil;
        NSString *deviceName = simulator[@"deviceTypeName"] ?: simulator[@"name"] ?: @"";
        NSDictionary *profile = [XCWChromeRenderer profileForDeviceName:deviceName
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
        NSDictionary *simulator = XCWSimulatorRecordForUDID(udid, error_message);
        if (simulator == nil) {
            return (xcw_native_owned_bytes){0};
        }

        NSError *renderError = nil;
        NSString *deviceName = simulator[@"deviceTypeName"] ?: simulator[@"name"] ?: @"";
        NSData *pngData = [XCWChromeRenderer PNGDataForDeviceName:deviceName
                                                            error:&renderError];
        if (pngData == nil) {
            XCWSetErrorMessage(error_message, renderError);
            return (xcw_native_owned_bytes){0};
        }

        return XCWOwnedBytesFromData(pngData);
    }
}

xcw_native_owned_bytes xcw_native_render_screen_mask_png(const char *udid, char **error_message) {
    @autoreleasepool {
        NSDictionary *simulator = XCWSimulatorRecordForUDID(udid, error_message);
        if (simulator == nil) {
            return (xcw_native_owned_bytes){0};
        }

        NSError *renderError = nil;
        NSString *deviceName = simulator[@"deviceTypeName"] ?: simulator[@"name"] ?: @"";
        NSData *pngData = [XCWChromeRenderer screenMaskPNGDataForDeviceName:deviceName
                                                                      error:&renderError];
        if (pngData == nil) {
            XCWSetErrorMessage(error_message, renderError);
            return (xcw_native_owned_bytes){0};
        }

        return XCWOwnedBytesFromData(pngData);
    }
}

xcw_native_owned_bytes xcw_native_screenshot_png(const char *udid, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSData *png = [simctl screenshotPNGForSimulatorUDID:XCWStringFromCString(udid) error:&error];
        if (png == nil) {
            XCWSetErrorMessage(error_message, error);
            return (xcw_native_owned_bytes){0};
        }
        return XCWOwnedBytesFromData(png);
    }
}

char *xcw_native_recent_logs(const char *udid, double seconds, size_t limit, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSArray<NSDictionary *> *entries = [simctl recentLogEntriesForSimulatorUDID:XCWStringFromCString(udid)
                                                                            seconds:seconds
                                                                              limit:limit
                                                                              error:&error];
        if (entries == nil) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }

        return XCWJSONStringFromObject(@{ @"entries": entries }, error_message);
    }
}

char *xcw_native_accessibility_snapshot(const char *udid, bool has_point, double x, double y, size_t max_depth, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        NSValue *pointValue = has_point ? [NSValue valueWithPoint:NSMakePoint(x, y)] : nil;
        NSDictionary *snapshot = [XCWAccessibilityBridge accessibilitySnapshotForSimulatorUDID:XCWStringFromCString(udid)
                                                                                       atPoint:pointValue
                                                                                     maxDepth:max_depth
                                                                                         error:&error];
        if (snapshot == nil) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
        return XCWJSONStringFromObject(snapshot, error_message);
    }
}

static BOOL XCWTouchPhaseFromString(NSString *phase, DFPrivateSimulatorTouchPhase *outPhase, NSError **error) {
    NSString *phaseValue = phase.lowercaseString;
    if ([phaseValue isEqualToString:@"began"]) {
        *outPhase = DFPrivateSimulatorTouchPhaseBegan;
        return YES;
    }
    if ([phaseValue isEqualToString:@"moved"]) {
        *outPhase = DFPrivateSimulatorTouchPhaseMoved;
        return YES;
    }
    if ([phaseValue isEqualToString:@"ended"]) {
        *outPhase = DFPrivateSimulatorTouchPhaseEnded;
        return YES;
    }
    if ([phaseValue isEqualToString:@"cancelled"]) {
        *outPhase = DFPrivateSimulatorTouchPhaseCancelled;
        return YES;
    }
    if (error != NULL) {
        *error = [NSError errorWithDomain:@"SimDeck.NativeBridge"
                                     code:1
                                 userInfo:@{ NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Unsupported touch phase `%@`.", phase ?: @""] }];
    }
    return NO;
}

static DFPrivateSimulatorDisplayBridge *XCWInputBridgeForUDID(const char *udid, char **errorMessage) {
    NSError *error = nil;
    DFPrivateSimulatorDisplayBridge *bridge = [[DFPrivateSimulatorDisplayBridge alloc] initWithUDID:XCWStringFromCString(udid)
                                                                                      attachDisplay:NO
                                                                                              error:&error];
    if (bridge == nil) {
        XCWSetErrorMessage(errorMessage, error);
    }
    return bridge;
}

bool xcw_native_send_touch(const char *udid, double x, double y, const char *phase, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *phaseError = nil;
        DFPrivateSimulatorTouchPhase touchPhase = DFPrivateSimulatorTouchPhaseMoved;
        if (!XCWTouchPhaseFromString(XCWStringFromCString(phase), &touchPhase, &phaseError)) {
            XCWSetErrorMessage(error_message, phaseError);
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge sendTouchAtNormalizedX:x normalizedY:y phase:touchPhase error:&error];
        [bridge disconnect];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

void *xcw_native_input_create(const char *udid, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return NULL;
        }
        return (__bridge_retained void *)bridge;
    }
}

void xcw_native_input_destroy(void *handle) {
    @autoreleasepool {
        if (handle == NULL) {
            return;
        }
        DFPrivateSimulatorDisplayBridge *bridge = CFBridgingRelease(handle);
        [bridge disconnect];
    }
}

bool xcw_native_input_display_size(void *handle, double *width, double *height) {
    @autoreleasepool {
        if (handle == NULL) {
            return false;
        }
        CGSize size = [(__bridge DFPrivateSimulatorDisplayBridge *)handle displaySize];
        if (width != NULL) {
            *width = size.width;
        }
        if (height != NULL) {
            *height = size.height;
        }
        return size.width > 0.0 && size.height > 0.0;
    }
}

bool xcw_native_input_send_touch(void *handle, double x, double y, const char *phase, char **error_message) {
    @autoreleasepool {
        if (handle == NULL) {
            XCWSetErrorMessage(error_message, [NSError errorWithDomain:@"SimDeck.NativeInput"
                                                                   code:1
                                                               userInfo:@{NSLocalizedDescriptionKey: @"Native input handle is null."}]);
            return false;
        }
        NSError *phaseError = nil;
        DFPrivateSimulatorTouchPhase touchPhase = DFPrivateSimulatorTouchPhaseMoved;
        if (!XCWTouchPhaseFromString(XCWStringFromCString(phase), &touchPhase, &phaseError)) {
            XCWSetErrorMessage(error_message, phaseError);
            return false;
        }
        NSError *error = nil;
        BOOL ok = [(__bridge DFPrivateSimulatorDisplayBridge *)handle sendTouchAtNormalizedX:x
                                                                                normalizedY:y
                                                                                      phase:touchPhase
                                                                                      error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_input_send_multitouch(void *handle, double x1, double y1, double x2, double y2, const char *phase, char **error_message) {
    @autoreleasepool {
        if (handle == NULL) {
            XCWSetErrorMessage(error_message, [NSError errorWithDomain:@"SimDeck.NativeInput"
                                                                   code:1
                                                               userInfo:@{NSLocalizedDescriptionKey: @"Native input handle is null."}]);
            return false;
        }
        NSError *phaseError = nil;
        DFPrivateSimulatorTouchPhase touchPhase = DFPrivateSimulatorTouchPhaseMoved;
        if (!XCWTouchPhaseFromString(XCWStringFromCString(phase), &touchPhase, &phaseError)) {
            XCWSetErrorMessage(error_message, phaseError);
            return false;
        }
        NSError *error = nil;
        BOOL ok = [(__bridge DFPrivateSimulatorDisplayBridge *)handle sendMultiTouchAtNormalizedX1:x1
                                                                                       normalizedY1:y1
                                                                                       normalizedX2:x2
                                                                                       normalizedY2:y2
                                                                                             phase:touchPhase
                                                                                             error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_input_send_key(void *handle, uint16_t key_code, uint32_t modifiers, char **error_message) {
    @autoreleasepool {
        if (handle == NULL) {
            XCWSetErrorMessage(error_message, [NSError errorWithDomain:@"SimDeck.NativeInput"
                                                                   code:1
                                                               userInfo:@{NSLocalizedDescriptionKey: @"Native input handle is null."}]);
            return false;
        }
        NSError *error = nil;
        BOOL ok = [(__bridge DFPrivateSimulatorDisplayBridge *)handle sendKeyCode:key_code
                                                                        modifiers:modifiers
                                                                            error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_input_send_key_event(void *handle, uint16_t key_code, bool down, char **error_message) {
    @autoreleasepool {
        if (handle == NULL) {
            XCWSetErrorMessage(error_message, [NSError errorWithDomain:@"SimDeck.NativeInput"
                                                                   code:1
                                                               userInfo:@{NSLocalizedDescriptionKey: @"Native input handle is null."}]);
            return false;
        }
        NSError *error = nil;
        BOOL ok = [(__bridge DFPrivateSimulatorDisplayBridge *)handle sendKeyCode:key_code
                                                                             down:down
                                                                            error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_send_key(const char *udid, uint16_t key_code, uint32_t modifiers, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge sendKeyCode:key_code modifiers:modifiers error:&error];
        [bridge disconnect];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_send_key_event(const char *udid, uint16_t key_code, bool down, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge sendKeyCode:key_code down:down error:&error];
        [bridge disconnect];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_press_home(const char *udid, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge pressHomeButton:&error];
        [bridge disconnect];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_press_button(const char *udid, const char *button_name, uint32_t duration_ms, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge pressHardwareButtonNamed:XCWStringFromCString(button_name)
                                        durationMs:(NSUInteger)duration_ms
                                             error:&error];
        [bridge disconnect];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_rotate_right(const char *udid, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge rotateRight:&error];
        [bridge disconnect];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_rotate_left(const char *udid, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge rotateLeft:&error];
        [bridge disconnect];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_erase_simulator(const char *udid, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        BOOL ok = [simctl eraseSimulatorWithUDID:XCWStringFromCString(udid) error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_install_app(const char *udid, const char *app_path, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        BOOL ok = [simctl installAppAtPath:XCWStringFromCString(app_path)
                             simulatorUDID:XCWStringFromCString(udid)
                                      error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_uninstall_app(const char *udid, const char *bundle_id, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        BOOL ok = [simctl uninstallBundleID:XCWStringFromCString(bundle_id)
                              simulatorUDID:XCWStringFromCString(udid)
                                       error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_set_pasteboard_text(const char *udid, const char *text, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        BOOL ok = [simctl setPasteboardText:XCWStringFromCString(text)
                              simulatorUDID:XCWStringFromCString(udid)
                                       error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

char *xcw_native_get_pasteboard_text(const char *udid, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSString *text = [simctl pasteboardTextForSimulatorUDID:XCWStringFromCString(udid) error:&error];
        if (text == nil) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
        return XCWCopyCString(text);
    }
}

void *xcw_native_session_create(const char *udid, char **error_message) {
    @autoreleasepool {
        @try {
            NSError *error = nil;
            XCWNativeSession *session = [[XCWNativeSession alloc] initWithUDID:XCWStringFromCString(udid)
                                                                         error:&error];
            if (session == nil) {
                XCWSetErrorMessage(error_message, error);
                return NULL;
            }
            return (__bridge_retained void *)session;
        } @catch (NSException *exception) {
            NSString *reason = exception.reason ?: exception.name ?: @"unknown Objective-C exception";
            NSError *error = [NSError errorWithDomain:@"SimDeck.NativeBridge"
                                                 code:91
                                             userInfo:@{ NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Native simulator session creation threw: %@", reason] }];
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
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

bool xcw_native_session_send_multitouch(void *handle, double x1, double y1, double x2, double y2, const char *phase, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) sendMultiTouchAtX1:x1
                                                                      y1:y1
                                                                      x2:x2
                                                                      y2:y2
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

bool xcw_native_session_rotate_left(void *handle, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) rotateLeft:&error];
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
