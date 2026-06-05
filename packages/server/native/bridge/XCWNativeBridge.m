#import "XCWNativeBridge.h"

#import "DFPrivateSimulatorDisplayBridge.h"
#import "XCWAccessibilityBridge.h"
#import "XCWChromeRenderer.h"
#import "XCWH264Encoder.h"
#import "XCWNativeSession.h"
#import "XCWSimctl.h"

#import <AppKit/AppKit.h>
#import <CoreFoundation/CoreFoundation.h>
#import <CoreVideo/CoreVideo.h>
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

@interface XCWNativeAccessibilityThreadRunner : NSObject
@end

@implementation XCWNativeAccessibilityThreadRunner

+ (void)run {
    @autoreleasepool {
        NSThread.currentThread.name = @"com.simdeck.native-accessibility";
        NSRunLoop *runLoop = NSRunLoop.currentRunLoop;
        [runLoop addPort:NSMachPort.port forMode:NSDefaultRunLoopMode];
        while (!NSThread.currentThread.cancelled) {
            @autoreleasepool {
                [runLoop runMode:NSDefaultRunLoopMode beforeDate:NSDate.distantFuture];
            }
        }
    }
}

@end

@interface XCWNativeAccessibilitySnapshotRequest : NSObject

@property (nonatomic, copy) NSString *udid;
@property (nonatomic, assign) BOOL hasPoint;
@property (nonatomic, assign) double x;
@property (nonatomic, assign) double y;
@property (nonatomic, assign) NSUInteger maxDepth;
@property (nonatomic, assign) BOOL interactiveOnly;
@property (nonatomic, assign) char *result;
@property (nonatomic, assign) char *serializationError;
@property (nonatomic, strong) NSError *snapshotError;

- (void)performSnapshot;

@end

@implementation XCWNativeAccessibilitySnapshotRequest

- (void)performSnapshot {
    @autoreleasepool {
        NSError *error = nil;
        NSValue *pointValue = self.hasPoint ? [NSValue valueWithPoint:NSMakePoint(self.x, self.y)] : nil;
        NSDictionary *snapshot = [XCWAccessibilityBridge accessibilitySnapshotForSimulatorUDID:self.udid
                                                                                       atPoint:pointValue
                                                                                     maxDepth:self.maxDepth
                                                                               interactiveOnly:self.interactiveOnly
                                                                                         error:&error];
        if (snapshot == nil) {
            self.snapshotError = error;
            return;
        }
        self.result = XCWJSONStringFromObject(snapshot, &_serializationError);
    }
}

@end

static NSThread *XCWNativeAccessibilityThread(void) {
    static NSThread *thread = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        thread = [[NSThread alloc] initWithTarget:XCWNativeAccessibilityThreadRunner.class
                                        selector:@selector(run)
                                          object:nil];
        thread.name = @"com.simdeck.native-accessibility";
        [thread start];
    });
    return thread;
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

static xcw_native_shared_bytes XCWSharedBytesFromData(NSData *data) {
    if (data.length == 0) {
        return (xcw_native_shared_bytes){0};
    }

    CFTypeRef owner = CFRetain((__bridge CFTypeRef)data);
    return (xcw_native_shared_bytes){
        .data = data.bytes,
        .length = data.length,
        .owner = (const void *)owner,
    };
}

static XCWNativeSession *XCWNativeSessionFromHandle(void *handle) {
    return (__bridge XCWNativeSession *)handle;
}

@interface XCWNativeH264Encoder : NSObject

- (instancetype)initWithFrameCallback:(xcw_native_frame_callback)callback
                             userData:(void *)userData;
- (BOOL)encodeRGBA:(const uint8_t *)rgba
            length:(size_t)length
             width:(uint32_t)width
            height:(uint32_t)height
             error:(NSError * _Nullable __autoreleasing *)error;
- (BOOL)encodeBGRA:(const uint8_t *)bgra
            length:(size_t)length
             width:(uint32_t)width
            height:(uint32_t)height
             error:(NSError * _Nullable __autoreleasing *)error;
- (void)requestKeyFrame;
- (void)invalidate;

@end

@implementation XCWNativeH264Encoder {
    XCWH264Encoder *_encoder;
    xcw_native_frame_callback _callback;
    void *_callbackUserData;
    uint64_t _frameSequence;
}

- (instancetype)initWithFrameCallback:(xcw_native_frame_callback)callback
                             userData:(void *)userData {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _callback = callback;
    _callbackUserData = userData;
    __weak typeof(self) weakSelf = self;
    @synchronized (XCWNativeH264Encoder.class) {
        const char *previousCodec = getenv("SIMDECK_VIDEO_CODEC");
        char *previousCodecCopy = previousCodec != NULL ? strdup(previousCodec) : NULL;
        const char *previousRealtimeStream = getenv("SIMDECK_REALTIME_STREAM");
        char *previousRealtimeStreamCopy = previousRealtimeStream != NULL ? strdup(previousRealtimeStream) : NULL;
        const char *androidCodec = getenv("SIMDECK_ANDROID_VIDEO_CODEC");
        if (androidCodec == NULL || strlen(androidCodec) == 0) {
            androidCodec = (previousCodec != NULL && strlen(previousCodec) > 0) ? previousCodec : "auto";
        }
        setenv("SIMDECK_VIDEO_CODEC", androidCodec, 1);
        setenv("SIMDECK_REALTIME_STREAM", "1", 1);
        _encoder = [[XCWH264Encoder alloc] initWithOutputHandler:^(NSData *sampleData,
                                                                   uint64_t timestampUs,
                                                                   BOOL isKeyFrame,
                                                                   NSString * _Nullable codec,
                                                                   NSData * _Nullable decoderConfig,
                                                                   CGSize dimensions) {
            __strong typeof(weakSelf) strongSelf = weakSelf;
            if (strongSelf == nil || strongSelf->_callback == NULL || sampleData.length == 0) {
                return;
            }
            strongSelf->_frameSequence += 1;
            xcw_native_frame frame = {
                .frame_sequence = strongSelf->_frameSequence,
                .timestamp_us = timestampUs,
                .is_keyframe = isKeyFrame,
                .width = (uint32_t)llround(dimensions.width),
                .height = (uint32_t)llround(dimensions.height),
                .codec = codec.UTF8String,
                .description = XCWSharedBytesFromData(decoderConfig),
                .data = XCWSharedBytesFromData(sampleData),
            };
            strongSelf->_callback(&frame, strongSelf->_callbackUserData);
        }];
        if (previousCodecCopy != NULL) {
            setenv("SIMDECK_VIDEO_CODEC", previousCodecCopy, 1);
            free(previousCodecCopy);
        } else {
            unsetenv("SIMDECK_VIDEO_CODEC");
        }
        if (previousRealtimeStreamCopy != NULL) {
            setenv("SIMDECK_REALTIME_STREAM", previousRealtimeStreamCopy, 1);
            free(previousRealtimeStreamCopy);
        } else {
            unsetenv("SIMDECK_REALTIME_STREAM");
        }
    }
    return self;
}

- (void)dealloc {
    [self invalidate];
}

- (BOOL)encodeRGBA:(const uint8_t *)rgba
            length:(size_t)length
             width:(uint32_t)width
            height:(uint32_t)height
             error:(NSError * _Nullable __autoreleasing *)error {
    if (rgba == NULL || width == 0 || height == 0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"SimDeck.NativeH264Encoder"
                                         code:1
                                     userInfo:@{ NSLocalizedDescriptionKey: @"RGBA frame input was empty." }];
        }
        return NO;
    }
    size_t expectedLength = (size_t)width * (size_t)height * 4;
    if (length < expectedLength) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"SimDeck.NativeH264Encoder"
                                         code:2
                                     userInfo:@{ NSLocalizedDescriptionKey: @"RGBA frame input was truncated." }];
        }
        return NO;
    }

    NSDictionary *attributes = @{
        (__bridge NSString *)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
        (__bridge NSString *)kCVPixelBufferWidthKey: @(width),
        (__bridge NSString *)kCVPixelBufferHeightKey: @(height),
        (__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey: @{},
    };
    CVPixelBufferRef pixelBuffer = NULL;
    CVReturn createStatus = CVPixelBufferCreate(kCFAllocatorDefault,
                                                (size_t)width,
                                                (size_t)height,
                                                kCVPixelFormatType_32BGRA,
                                                (__bridge CFDictionaryRef)attributes,
                                                &pixelBuffer);
    if (createStatus != kCVReturnSuccess || pixelBuffer == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"SimDeck.NativeH264Encoder"
                                         code:createStatus
                                     userInfo:@{ NSLocalizedDescriptionKey: @"Unable to allocate a VideoToolbox pixel buffer." }];
        }
        return NO;
    }

    CVReturn lockStatus = CVPixelBufferLockBaseAddress(pixelBuffer, 0);
    if (lockStatus != kCVReturnSuccess) {
        CVPixelBufferRelease(pixelBuffer);
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"SimDeck.NativeH264Encoder"
                                         code:lockStatus
                                     userInfo:@{ NSLocalizedDescriptionKey: @"Unable to lock a VideoToolbox pixel buffer." }];
        }
        return NO;
    }

    uint8_t *dst = CVPixelBufferGetBaseAddress(pixelBuffer);
    size_t dstRowBytes = CVPixelBufferGetBytesPerRow(pixelBuffer);
    size_t srcRowBytes = (size_t)width * 4;
    for (uint32_t y = 0; y < height; y += 1) {
        const uint8_t *srcRow = rgba + ((size_t)y * srcRowBytes);
        uint8_t *dstRow = dst + ((size_t)y * dstRowBytes);
        for (uint32_t x = 0; x < width; x += 1) {
            const uint8_t *src = srcRow + ((size_t)x * 4);
            uint8_t *pixel = dstRow + ((size_t)x * 4);
            pixel[0] = src[2];
            pixel[1] = src[1];
            pixel[2] = src[0];
            pixel[3] = src[3];
        }
    }
    CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);
    [_encoder encodePixelBuffer:pixelBuffer];
    CVPixelBufferRelease(pixelBuffer);
    return YES;
}

- (BOOL)encodeBGRA:(const uint8_t *)bgra
            length:(size_t)length
             width:(uint32_t)width
            height:(uint32_t)height
             error:(NSError * _Nullable __autoreleasing *)error {
    if (bgra == NULL || width == 0 || height == 0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"SimDeck.NativeH264Encoder"
                                         code:1
                                     userInfo:@{ NSLocalizedDescriptionKey: @"BGRA frame input was empty." }];
        }
        return NO;
    }
    size_t expectedLength = (size_t)width * (size_t)height * 4;
    if (length < expectedLength) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"SimDeck.NativeH264Encoder"
                                         code:2
                                     userInfo:@{ NSLocalizedDescriptionKey: @"BGRA frame input was truncated." }];
        }
        return NO;
    }

    NSDictionary *attributes = @{
        (__bridge NSString *)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
        (__bridge NSString *)kCVPixelBufferWidthKey: @(width),
        (__bridge NSString *)kCVPixelBufferHeightKey: @(height),
        (__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey: @{},
    };
    CVPixelBufferRef pixelBuffer = NULL;
    CVReturn createStatus = CVPixelBufferCreate(kCFAllocatorDefault,
                                                (size_t)width,
                                                (size_t)height,
                                                kCVPixelFormatType_32BGRA,
                                                (__bridge CFDictionaryRef)attributes,
                                                &pixelBuffer);
    if (createStatus != kCVReturnSuccess || pixelBuffer == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"SimDeck.NativeH264Encoder"
                                         code:createStatus
                                     userInfo:@{ NSLocalizedDescriptionKey: @"Unable to allocate a VideoToolbox pixel buffer." }];
        }
        return NO;
    }

    CVReturn lockStatus = CVPixelBufferLockBaseAddress(pixelBuffer, 0);
    if (lockStatus != kCVReturnSuccess) {
        CVPixelBufferRelease(pixelBuffer);
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"SimDeck.NativeH264Encoder"
                                         code:lockStatus
                                     userInfo:@{ NSLocalizedDescriptionKey: @"Unable to lock a VideoToolbox pixel buffer." }];
        }
        return NO;
    }

    uint8_t *dst = CVPixelBufferGetBaseAddress(pixelBuffer);
    size_t dstRowBytes = CVPixelBufferGetBytesPerRow(pixelBuffer);
    size_t srcRowBytes = (size_t)width * 4;
    for (uint32_t y = 0; y < height; y += 1) {
        memcpy(dst + ((size_t)y * dstRowBytes),
               bgra + ((size_t)y * srcRowBytes),
               srcRowBytes);
    }
    CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);
    [_encoder encodePixelBuffer:pixelBuffer];
    CVPixelBufferRelease(pixelBuffer);
    return YES;
}

- (void)requestKeyFrame {
    [_encoder requestKeyFrame];
}

- (void)invalidate {
    [_encoder invalidate];
}

@end

static XCWNativeH264Encoder *XCWNativeH264EncoderFromHandle(void *handle) {
    return (__bridge XCWNativeH264Encoder *)handle;
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

char *xcw_native_simulator_creation_options(char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSDictionary *options = [simctl simulatorCreationOptionsWithError:&error];
        if (options == nil) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
        return XCWJSONStringFromObject(options, error_message);
    }
}

char *xcw_native_create_simulator(const char *name,
                                  const char *device_type_identifier,
                                  const char *runtime_identifier,
                                  const char *paired_watch_name,
                                  const char *paired_watch_device_type_identifier,
                                  const char *paired_watch_runtime_identifier,
                                  char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSDictionary *result = [simctl createSimulatorWithName:XCWStringFromCString(name)
                                          deviceTypeIdentifier:XCWStringFromCString(device_type_identifier)
                                             runtimeIdentifier:runtime_identifier == NULL ? nil : XCWStringFromCString(runtime_identifier)
                                               pairedWatchName:paired_watch_name == NULL ? nil : XCWStringFromCString(paired_watch_name)
                               pairedWatchDeviceTypeIdentifier:paired_watch_device_type_identifier == NULL ? nil : XCWStringFromCString(paired_watch_device_type_identifier)
                                  pairedWatchRuntimeIdentifier:paired_watch_runtime_identifier == NULL ? nil : XCWStringFromCString(paired_watch_runtime_identifier)
                                                         error:&error];
        if (result == nil) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
        return XCWJSONStringFromObject(result, error_message);
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

xcw_native_owned_bytes xcw_native_render_chrome_png(const char *udid, bool include_buttons, char **error_message) {
    @autoreleasepool {
        NSDictionary *simulator = XCWSimulatorRecordForUDID(udid, error_message);
        if (simulator == nil) {
            return (xcw_native_owned_bytes){0};
        }

        NSError *renderError = nil;
        NSString *deviceName = simulator[@"deviceTypeName"] ?: simulator[@"name"] ?: @"";
        NSData *pngData = [XCWChromeRenderer PNGDataForDeviceName:deviceName
                                                   includeButtons:include_buttons
                                                            error:&renderError];
        if (pngData == nil) {
            XCWSetErrorMessage(error_message, renderError);
            return (xcw_native_owned_bytes){0};
        }

        return XCWOwnedBytesFromData(pngData);
    }
}

xcw_native_owned_bytes xcw_native_render_chrome_button_png(const char *udid, const char *button_name, bool pressed, char **error_message) {
    @autoreleasepool {
        NSDictionary *simulator = XCWSimulatorRecordForUDID(udid, error_message);
        if (simulator == nil) {
            return (xcw_native_owned_bytes){0};
        }

        NSError *renderError = nil;
        NSString *deviceName = simulator[@"deviceTypeName"] ?: simulator[@"name"] ?: @"";
        NSData *pngData = [XCWChromeRenderer buttonPNGDataForDeviceName:deviceName
                                                              buttonName:XCWStringFromCString(button_name)
                                                                 pressed:pressed
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

xcw_native_owned_bytes xcw_native_screenshot_png(const char *udid, bool include_bezel, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSData *png = [simctl screenshotPNGForSimulatorUDID:XCWStringFromCString(udid)
                                               includeBezel:include_bezel
                                                      error:&error];
        if (png == nil) {
            XCWSetErrorMessage(error_message, error);
            return (xcw_native_owned_bytes){0};
        }
        return XCWOwnedBytesFromData(png);
    }
}

xcw_native_owned_bytes xcw_native_screen_recording_mp4(const char *udid, double duration_seconds, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSData *mp4 = [simctl screenRecordingMP4ForSimulatorUDID:XCWStringFromCString(udid)
                                                 durationSeconds:duration_seconds
                                                           error:&error];
        if (mp4 == nil) {
            XCWSetErrorMessage(error_message, error);
            return (xcw_native_owned_bytes){0};
        }
        return XCWOwnedBytesFromData(mp4);
    }
}

char *xcw_native_start_screen_recording(const char *udid, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSString *recordingID = [simctl startScreenRecordingForSimulatorUDID:XCWStringFromCString(udid)
                                                                       error:&error];
        if (recordingID == nil) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
        return XCWCopyCString(recordingID);
    }
}

xcw_native_owned_bytes xcw_native_stop_screen_recording(const char *recording_id, char **error_message) {
    @autoreleasepool {
        XCWSimctl *simctl = [[XCWSimctl alloc] init];
        NSError *error = nil;
        NSData *mp4 = [simctl stopScreenRecordingWithID:XCWStringFromCString(recording_id)
                                                  error:&error];
        if (mp4 == nil) {
            XCWSetErrorMessage(error_message, error);
            return (xcw_native_owned_bytes){0};
        }
        return XCWOwnedBytesFromData(mp4);
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

char *xcw_native_accessibility_snapshot(const char *udid, bool has_point, double x, double y, size_t max_depth, bool interactive_only, char **error_message) {
    @autoreleasepool {
        XCWNativeAccessibilitySnapshotRequest *request = [XCWNativeAccessibilitySnapshotRequest new];
        request.udid = XCWStringFromCString(udid);
        request.hasPoint = has_point;
        request.x = x;
        request.y = y;
        request.maxDepth = max_depth;
        request.interactiveOnly = interactive_only;

        NSThread *accessibilityThread = XCWNativeAccessibilityThread();
        if (NSThread.currentThread == accessibilityThread) {
            [request performSnapshot];
        } else {
            [request performSelector:@selector(performSnapshot)
                             onThread:accessibilityThread
                           withObject:nil
                        waitUntilDone:YES
                                modes:@[NSDefaultRunLoopMode]];
        }

        if (request.result != NULL) {
            return request.result;
        }
        if (request.serializationError != NULL) {
            if (error_message != NULL) {
                *error_message = request.serializationError;
            } else {
                free(request.serializationError);
            }
            return NULL;
        }
        XCWSetErrorMessage(error_message, request.snapshotError);
        return NULL;
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
        return nil;
    }

    NSDictionary *simulator = XCWSimulatorRecordForUDID(udid, NULL);
    NSString *deviceName = simulator[@"deviceTypeName"] ?: simulator[@"name"] ?: @"";
    NSError *displaySizeError = nil;
    CGSize displaySize = [XCWChromeRenderer displayPixelSizeForDeviceName:deviceName
                                                                    error:&displaySizeError];
    if (displaySize.width > 0.0 && displaySize.height > 0.0) {
        [bridge updateInputDisplaySize:displaySize];
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

bool xcw_native_input_send_edge_touch(void *handle, double x, double y, const char *phase, uint32_t edge, char **error_message) {
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
        BOOL ok = [(__bridge DFPrivateSimulatorDisplayBridge *)handle sendEdgeTouchAtNormalizedX:x
                                                                                    normalizedY:y
                                                                                          phase:touchPhase
                                                                                           edge:(DFPrivateSimulatorTouchEdge)edge
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

bool xcw_native_open_app_switcher(const char *udid, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge openAppSwitcher:&error];
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

bool xcw_native_send_button(const char *udid, const char *button_name, bool pressed, bool has_usage, uint32_t usage_page, uint32_t usage, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge sendHardwareButtonNamed:XCWStringFromCString(button_name)
                                          pressed:pressed
                                        usagePage:has_usage ? @(usage_page) : nil
                                            usage:has_usage ? @(usage) : nil
                                            error:&error];
        [bridge disconnect];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_rotate_crown(const char *udid, double delta, char **error_message) {
    @autoreleasepool {
        DFPrivateSimulatorDisplayBridge *bridge = XCWInputBridgeForUDID(udid, error_message);
        if (bridge == nil) {
            return false;
        }
        NSError *error = nil;
        BOOL ok = [bridge rotateDigitalCrownByDelta:delta error:&error];
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

void xcw_native_session_request_keyframe(void *handle) {
    @autoreleasepool {
        [XCWNativeSessionFromHandle(handle) requestKeyFrame];
    }
}

void xcw_native_session_reconfigure_video_encoder(void *handle) {
    @autoreleasepool {
        [XCWNativeSessionFromHandle(handle) reconfigureVideoEncoder];
    }
}

void xcw_native_session_set_client_foreground(void *handle, bool foreground) {
    @autoreleasepool {
        [XCWNativeSessionFromHandle(handle) setClientForeground:foreground];
    }
}

char *xcw_native_session_video_encoder_stats(void *handle, char **error_message) {
    @autoreleasepool {
        return XCWJSONStringFromObject([XCWNativeSessionFromHandle(handle) videoEncoderStats] ?: @{}, error_message);
    }
}

int32_t xcw_native_session_rotation_quarter_turns(void *handle) {
    @autoreleasepool {
        NSInteger turns = [XCWNativeSessionFromHandle(handle) rotationQuarterTurns];
        NSInteger normalized = ((turns % 4) + 4) % 4;
        return (int32_t)normalized;
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

bool xcw_native_session_send_edge_touch(void *handle, double x, double y, const char *phase, uint32_t edge, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) sendEdgeTouchAtX:x
                                                                     y:y
                                                                 phase:XCWStringFromCString(phase)
                                                                  edge:(NSInteger)edge
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

bool xcw_native_session_press_button(void *handle, const char *button_name, uint32_t duration_ms, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) pressHardwareButtonNamed:XCWStringFromCString(button_name)
                                                                    durationMs:(NSUInteger)duration_ms
                                                                         error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_session_send_button(void *handle, const char *button_name, bool pressed, bool has_usage, uint32_t usage_page, uint32_t usage, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) sendHardwareButtonNamed:XCWStringFromCString(button_name)
                                                                      pressed:pressed
                                                                    usagePage:has_usage ? @(usage_page) : nil
                                                                        usage:has_usage ? @(usage) : nil
                                                                        error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_session_rotate_crown(void *handle, double delta, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) rotateDigitalCrownByDelta:delta error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_session_open_app_switcher(void *handle, char **error_message) {
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeSessionFromHandle(handle) openAppSwitcher:&error];
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

void *xcw_native_h264_encoder_create(xcw_native_frame_callback callback, void *user_data, char **error_message) {
    @autoreleasepool {
        XCWNativeH264Encoder *encoder = [[XCWNativeH264Encoder alloc] initWithFrameCallback:callback
                                                                                  userData:user_data];
        if (encoder == nil) {
            if (error_message != NULL) {
                *error_message = XCWCopyCString(@"Unable to create the native H.264 encoder.");
            }
            return NULL;
        }
        return (__bridge_retained void *)encoder;
    }
}

void xcw_native_h264_encoder_destroy(void *handle) {
    if (handle == NULL) {
        return;
    }
    @autoreleasepool {
        XCWNativeH264Encoder *encoder = CFBridgingRelease(handle);
        [encoder invalidate];
    }
}

bool xcw_native_h264_encoder_encode_rgba(void *handle,
                                         const uint8_t *rgba,
                                         size_t length,
                                         uint32_t width,
                                         uint32_t height,
                                         uint64_t timestamp_us,
                                         char **error_message) {
    (void)timestamp_us;
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeH264EncoderFromHandle(handle) encodeRGBA:rgba
                                                              length:length
                                                               width:width
                                                              height:height
                                                               error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

bool xcw_native_h264_encoder_encode_bgra(void *handle,
                                         const uint8_t *bgra,
                                         size_t length,
                                         uint32_t width,
                                         uint32_t height,
                                         uint64_t timestamp_us,
                                         char **error_message) {
    (void)timestamp_us;
    @autoreleasepool {
        NSError *error = nil;
        BOOL ok = [XCWNativeH264EncoderFromHandle(handle) encodeBGRA:bgra
                                                              length:length
                                                               width:width
                                                              height:height
                                                               error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

void xcw_native_h264_encoder_request_keyframe(void *handle) {
    @autoreleasepool {
        [XCWNativeH264EncoderFromHandle(handle) requestKeyFrame];
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
