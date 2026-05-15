#import <AVFoundation/AVFoundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#import <QuartzCore/QuartzCore.h>
#import <UIKit/UIKit.h>

#import "SimDeckCameraShared.h"

#import <objc/message.h>
#import <objc/runtime.h>
#import <stdatomic.h>
#import <stdbool.h>
#import <stdint.h>
#import <stdio.h>
#import <stdlib.h>
#import <string.h>
#import <sys/mman.h>
#import <sys/stat.h>
#import <unistd.h>

static SimDeckCameraHeader *gHeader;
static uint8_t *gFrameBytes;
static size_t gFrameMapSize;
static dispatch_source_t gFrameTimer;
static dispatch_queue_t gFrameQueue;
static NSMutableArray<NSValue *> *gSessions;
static NSMutableArray<NSValue *> *gVideoOutputs;
static NSHashTable<CALayer *> *gPreviewLayers;
static NSMutableSet<NSString *> *gHookedVideoOutputClasses;

static char kSessionInputsKey;
static char kSessionOutputsKey;
static char kSessionRunningKey;
static char kInputFakeKey;
static char kInputDeviceKey;
static char kOutputDelegateKey;
static char kOutputQueueKey;
static char kOutputVideoSettingsKey;
static char kOutputDiscardsLateFramesKey;
static char kPreviewOverlayKey;
static char kPreviewHostKey;
static char kPickerOverlayViewKey;
static char kPickerCaptureControlKey;
static char kPickerCameraOverlayKey;
static char kPickerCaptureWindowKey;

static void StartFrameTimer(void);
static void InstallVideoOutputDelegateHook(Class cls);
static void Log(NSString *format, ...);
static void DebugLog(NSString *format, ...);
static BOOL OpenSharedCamera(void);
static void TrackPointer(NSMutableArray<NSValue *> *pointers, id object);
static void RegisterOutputLayer(CALayer *layer);
static void RegisterPreviewLayer(CALayer *layer);
static void SendPickerCapture(UIImagePickerController *picker);

static BOOL IsVideoMediaType(AVMediaType mediaType) {
    return mediaType == nil || [mediaType isEqualToString:AVMediaTypeVideo];
}

static void SimDeckSetSampleBufferDelegate(AVCaptureVideoDataOutput *output,
                                           SEL selector,
                                           id<AVCaptureVideoDataOutputSampleBufferDelegate> delegate,
                                           dispatch_queue_t sampleBufferCallbackQueue) {
    (void)selector;
    objc_setAssociatedObject(output, &kOutputDelegateKey, delegate, OBJC_ASSOCIATION_ASSIGN);
    objc_setAssociatedObject(output, &kOutputQueueKey, sampleBufferCallbackQueue, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    @synchronized(gVideoOutputs) {
        TrackPointer(gVideoOutputs, output);
    }
    StartFrameTimer();
}

static void TrackPointer(NSMutableArray<NSValue *> *pointers, id object) {
    if (!pointers || !object) return;
    void *pointer = (__bridge void *)object;
    for (NSValue *value in pointers) {
        if (value.pointerValue == pointer) return;
    }
    [pointers addObject:[NSValue valueWithPointer:pointer]];
}

static void RegisterOutputLayer(CALayer *layer) {
    if (!layer) return;
    @synchronized(gPreviewLayers) {
        [gPreviewLayers addObject:layer];
    }
    layer.contentsGravity = kCAGravityResizeAspectFill;
    layer.masksToBounds = YES;
    StartFrameTimer();
}

static void RegisterPreviewLayer(CALayer *layer) {
    if (!layer) return;
    CALayer *overlay = objc_getAssociatedObject(layer, &kPreviewOverlayKey);
    if (!overlay) {
        overlay = [CALayer layer];
        overlay.contentsGravity = kCAGravityResizeAspectFill;
        overlay.masksToBounds = YES;
        overlay.frame = layer.bounds;
        objc_setAssociatedObject(overlay, &kPreviewHostKey, layer, OBJC_ASSOCIATION_ASSIGN);
        objc_setAssociatedObject(layer, &kPreviewOverlayKey, overlay, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        [layer addSublayer:overlay];
        DebugLog(@"installed preview frame layer on %@", NSStringFromClass(object_getClass(layer)));
    }
    RegisterOutputLayer(overlay);
}

static void SimDeckSetVideoSettings(AVCaptureVideoDataOutput *output,
                                    SEL selector,
                                    NSDictionary *videoSettings) {
    (void)selector;
    objc_setAssociatedObject(output, &kOutputVideoSettingsKey, videoSettings, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    InstallVideoOutputDelegateHook(object_getClass(output));
    DebugLog(@"captured video settings on %@", NSStringFromClass(object_getClass(output)));
}

static id SimDeckVideoDataOutputAllocWithZone(Class cls, SEL selector, struct _NSZone *zone) {
    (void)selector;
    (void)zone;
    if (cls == AVCaptureVideoDataOutput.class && OpenSharedCamera()) {
        Class fakeClass = objc_getClass("SimDeckCameraVideoDataOutput");
        if (fakeClass) return class_createInstance(fakeClass, 0);
    }
    struct objc_super superInfo = {
        .receiver = cls,
        .super_class = class_getSuperclass(object_getClass(cls)),
    };
    return ((id (*)(struct objc_super *, SEL, struct _NSZone *))objc_msgSendSuper)(&superInfo, @selector(allocWithZone:), zone);
}

static void Log(NSString *format, ...) {
    va_list args;
    va_start(args, format);
    NSString *message = [[NSString alloc] initWithFormat:format arguments:args];
    va_end(args);
    fprintf(stderr, "[simdeck-camera] %s\n", message.UTF8String ?: "");
}

static BOOL DebugLoggingEnabled(void) {
    static BOOL enabled;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        const char *value = getenv("SIMDECK_CAMERA_DEBUG");
        enabled = value && value[0] != '\0' && strcmp(value, "0") != 0;
    });
    return enabled;
}

static void DebugLog(NSString *format, ...) {
    if (!DebugLoggingEnabled()) return;
    va_list args;
    va_start(args, format);
    NSString *message = [[NSString alloc] initWithFormat:format arguments:args];
    va_end(args);
    fprintf(stderr, "[simdeck-camera] %s\n", message.UTF8String ?: "");
}

static BOOL OpenSharedCamera(void) {
    if (gHeader) return YES;
    const char *name = getenv("SIMDECK_CAMERA_SHM_NAME");
    if (!name || name[0] == '\0') {
        name = getenv("SIMCAM_SHM_NAME");
    }
    if (!name || name[0] == '\0') {
        return NO;
    }
    int fd = shm_open(name, O_RDONLY, 0);
    if (fd < 0) {
        Log(@"unable to open shared memory %s", name);
        return NO;
    }
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size < (off_t)SIMDECK_CAMERA_HEADER_SIZE) {
        close(fd);
        return NO;
    }
    void *mapped = mmap(NULL, (size_t)st.st_size, PROT_READ, MAP_SHARED, fd, 0);
    close(fd);
    if (mapped == MAP_FAILED) {
        return NO;
    }
    SimDeckCameraHeader *header = (SimDeckCameraHeader *)mapped;
    if (header->magic != SIMDECK_CAMERA_MAGIC || header->version != SIMDECK_CAMERA_VERSION) {
        munmap(mapped, (size_t)st.st_size);
        return NO;
    }
    gHeader = header;
    gFrameMapSize = (size_t)st.st_size;
    gFrameBytes = ((uint8_t *)mapped) + header->headerSize;
    DebugLog(@"attached shared feed %ux%u", header->width, header->height);
    return YES;
}

static BOOL CopyCurrentFrame(NSMutableData **outData, uint32_t *outWidth, uint32_t *outHeight, uint32_t *outBytesPerRow, uint64_t *outTimestampNs) {
    if (!OpenSharedCamera()) return NO;
    uint32_t width = gHeader->width;
    uint32_t height = gHeader->height;
    uint32_t bytesPerRow = gHeader->bytesPerRow;
    if (width == 0 || height == 0 || bytesPerRow < width * 4) return NO;
    size_t length = (size_t)bytesPerRow * height;
    if ((size_t)gHeader->headerSize + length > gFrameMapSize) return NO;

    NSMutableData *copy = [NSMutableData dataWithLength:length];
    uint64_t before = 0;
    uint64_t after = 0;
    for (int attempt = 0; attempt < 4; attempt += 1) {
        before = gHeader->sequence;
        if ((before & 1u) != 0) {
            usleep(1000);
            continue;
        }
        memcpy(copy.mutableBytes, gFrameBytes, length);
        after = gHeader->sequence;
        if (before == after && (after & 1u) == 0) {
            *outData = copy;
            *outWidth = width;
            *outHeight = height;
            *outBytesPerRow = bytesPerRow;
            *outTimestampNs = gHeader->timestampNs;
            return YES;
        }
    }
    return NO;
}

static CMSampleBufferRef CreateSampleBuffer(NSData *frameData, uint32_t width, uint32_t height, uint32_t bytesPerRow, uint64_t timestampNs) {
    CVPixelBufferRef pixelBuffer = NULL;
    NSDictionary *attrs = @{
        (id)kCVPixelBufferIOSurfacePropertiesKey: @{},
        (id)kCVPixelBufferCGImageCompatibilityKey: @YES,
        (id)kCVPixelBufferCGBitmapContextCompatibilityKey: @YES,
    };
    CVReturn cv = CVPixelBufferCreate(kCFAllocatorDefault,
                                      width,
                                      height,
                                      kCVPixelFormatType_32BGRA,
                                      (__bridge CFDictionaryRef)attrs,
                                      &pixelBuffer);
    if (cv != kCVReturnSuccess || !pixelBuffer) return NULL;
    CVPixelBufferLockBaseAddress(pixelBuffer, 0);
    uint8_t *dest = CVPixelBufferGetBaseAddress(pixelBuffer);
    size_t destBytesPerRow = CVPixelBufferGetBytesPerRow(pixelBuffer);
    const uint8_t *source = frameData.bytes;
    for (uint32_t y = 0; y < height; y += 1) {
        memcpy(dest + (size_t)y * destBytesPerRow,
               source + (size_t)y * bytesPerRow,
               MIN(destBytesPerRow, bytesPerRow));
    }
    CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);

    CMVideoFormatDescriptionRef format = NULL;
    OSStatus status = CMVideoFormatDescriptionCreateForImageBuffer(kCFAllocatorDefault, pixelBuffer, &format);
    if (status != noErr || !format) {
        CVPixelBufferRelease(pixelBuffer);
        return NULL;
    }
    CMTime pts = CMTimeMake((int64_t)(timestampNs ?: (uint64_t)(CACurrentMediaTime() * 1000000000.0)), 1000000000);
    CMSampleTimingInfo timing = {
        .duration = CMTimeMake(1, 30),
        .presentationTimeStamp = pts,
        .decodeTimeStamp = kCMTimeInvalid,
    };
    CMSampleBufferRef sample = NULL;
    status = CMSampleBufferCreateReadyWithImageBuffer(kCFAllocatorDefault,
                                                      pixelBuffer,
                                                      format,
                                                      &timing,
                                                      &sample);
    CFRelease(format);
    CVPixelBufferRelease(pixelBuffer);
    return status == noErr ? sample : NULL;
}

static CGImageRef CreateImage(NSData *frameData, uint32_t width, uint32_t height, uint32_t bytesPerRow) {
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGDataProviderRef provider = CGDataProviderCreateWithCFData((__bridge CFDataRef)frameData);
    CGImageRef image = CGImageCreate(width,
                                     height,
                                     8,
                                     32,
                                     bytesPerRow,
                                     colorSpace,
                                     kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst,
                                     provider,
                                     NULL,
                                     false,
                                     kCGRenderingIntentDefault);
    CGDataProviderRelease(provider);
    CGColorSpaceRelease(colorSpace);
    return image;
}

static UIImage *CurrentFrameImage(void) {
    NSMutableData *data = nil;
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t bytesPerRow = 0;
    uint64_t timestampNs = 0;
    if (!CopyCurrentFrame(&data, &width, &height, &bytesPerRow, &timestampNs)) return nil;
    CGImageRef image = CreateImage(data, width, height, bytesPerRow);
    if (!image) return nil;
    UIImage *uiImage = [UIImage imageWithCGImage:image scale:UIScreen.mainScreen.scale orientation:UIImageOrientationUp];
    CGImageRelease(image);
    return uiImage;
}

static void DeliverFrame(void) {
    NSMutableData *data = nil;
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t bytesPerRow = 0;
    uint64_t timestampNs = 0;
    if (!CopyCurrentFrame(&data, &width, &height, &bytesPerRow, &timestampNs)) return;
    timestampNs = (uint64_t)(CACurrentMediaTime() * 1000000000.0);

    CMSampleBufferRef sample = CreateSampleBuffer(data, width, height, bytesPerRow, timestampNs);
    if (sample) {
        NSArray *outputs = nil;
        @synchronized(gVideoOutputs) {
            outputs = [gVideoOutputs copy];
        }
        for (NSValue *value in outputs) {
            AVCaptureVideoDataOutput *output = (__bridge AVCaptureVideoDataOutput *)value.pointerValue;
            id delegate = objc_getAssociatedObject(output, &kOutputDelegateKey);
            dispatch_queue_t queue = objc_getAssociatedObject(output, &kOutputQueueKey);
            if (!delegate || ![delegate respondsToSelector:@selector(captureOutput:didOutputSampleBuffer:fromConnection:)]) {
                continue;
            }
            CFRetain(sample);
            dispatch_async(queue ?: dispatch_get_main_queue(), ^{
                ((void (*)(id, SEL, AVCaptureOutput *, CMSampleBufferRef, AVCaptureConnection *))objc_msgSend)(
                    delegate,
                    @selector(captureOutput:didOutputSampleBuffer:fromConnection:),
                    output,
                    sample,
                    nil);
                CFRelease(sample);
            });
        }
        [outputs release];
        CFRelease(sample);
    }

    CGImageRef image = CreateImage(data, width, height, bytesPerRow);
    if (image) {
        NSArray<CALayer *> *layers = nil;
        @synchronized(gPreviewLayers) {
            layers = gPreviewLayers.allObjects;
        }
        dispatch_async(dispatch_get_main_queue(), ^{
            for (CALayer *layer in layers) {
                CALayer *host = objc_getAssociatedObject(layer, &kPreviewHostKey);
                if (host) {
                    layer.frame = host.bounds;
                }
                layer.contents = (__bridge id)image;
                layer.contentsGravity = kCAGravityResizeAspectFill;
                if (gHeader && gHeader->mirrorMode == SIMDECK_CAMERA_MIRROR_ON) {
                    layer.transform = CATransform3DMakeScale(-1, 1, 1);
                } else {
                    layer.transform = CATransform3DIdentity;
                }
            }
            CGImageRelease(image);
        });
    }
}

static void StartFrameTimer(void) {
    if (gFrameTimer) return;
    if (!gFrameQueue) {
        gFrameQueue = dispatch_queue_create("dev.nativescript.simdeck.camera.injector", DISPATCH_QUEUE_SERIAL);
    }
    gFrameTimer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, gFrameQueue);
    dispatch_source_set_timer(gFrameTimer,
                              dispatch_time(DISPATCH_TIME_NOW, 0),
                              (uint64_t)(NSEC_PER_SEC / 30),
                              (uint64_t)(NSEC_PER_MSEC * 4));
    dispatch_source_set_event_handler(gFrameTimer, ^{
        DeliverFrame();
    });
    dispatch_resume(gFrameTimer);
}

static void AddSessionOutput(AVCaptureSession *session, AVCaptureOutput *output) {
    NSMutableArray *outputs = objc_getAssociatedObject(session, &kSessionOutputsKey);
    if (!outputs) {
        outputs = [NSMutableArray array];
        objc_setAssociatedObject(session, &kSessionOutputsKey, outputs, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    if (![outputs containsObject:output]) [outputs addObject:output];
    if ([output isKindOfClass:AVCaptureVideoDataOutput.class]) {
        @synchronized(gVideoOutputs) {
            TrackPointer(gVideoOutputs, output);
        }
    }
}

static BOOL SimDeckIsFakeInput(id input) {
    return [objc_getAssociatedObject(input, &kInputFakeKey) boolValue];
}

static AVCaptureDeviceInput *SimDeckFakeInput(void) {
    static AVCaptureDeviceInput *input;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        input = (AVCaptureDeviceInput *)class_createInstance(AVCaptureDeviceInput.class, 0);
        objc_setAssociatedObject(input, &kInputFakeKey, @YES, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        Class deviceClass = objc_getClass("SimDeckCameraDevice");
        id device = ((id (*)(Class, SEL))objc_msgSend)(deviceClass, @selector(sharedDevice));
        objc_setAssociatedObject(input, &kInputDeviceKey, device, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    });
    return input;
}

static BOOL ClassIsSubclassOf(Class cls, Class parent) {
    for (Class current = cls; current; current = class_getSuperclass(current)) {
        if (current == parent) return YES;
    }
    return NO;
}

static void InstallVideoOutputDelegateHook(Class cls) {
    if (!cls) return;
    @synchronized(gHookedVideoOutputClasses) {
        NSString *name = NSStringFromClass(cls);
        if ([gHookedVideoOutputClasses containsObject:name]) return;
        [gHookedVideoOutputClasses addObject:name];
    }
    Method original = class_getInstanceMethod(cls, @selector(setSampleBufferDelegate:queue:));
    if (!original) {
        Log(@"missing video output delegate hook on %@", NSStringFromClass(cls));
        return;
    }
    class_replaceMethod(cls,
                        @selector(setSampleBufferDelegate:queue:),
                        (IMP)SimDeckSetSampleBufferDelegate,
                        method_getTypeEncoding(original));
    Method settings = class_getInstanceMethod(cls, @selector(setVideoSettings:));
    if (settings) {
        class_replaceMethod(cls,
                            @selector(setVideoSettings:),
                            (IMP)SimDeckSetVideoSettings,
                            method_getTypeEncoding(settings));
    }
    DebugLog(@"hooked video output delegate on %@", NSStringFromClass(cls));
}

static void InstallExistingVideoOutputDelegateHooks(void) {
    int count = objc_getClassList(NULL, 0);
    if (count <= 0) return;
    Class *classes = calloc((size_t)count, sizeof(Class));
    if (!classes) return;
    count = objc_getClassList(classes, count);
    for (int index = 0; index < count; index += 1) {
        Class cls = classes[index];
        if (ClassIsSubclassOf(cls, AVCaptureVideoDataOutput.class)) {
            InstallVideoOutputDelegateHook(cls);
        }
    }
    free(classes);
}

static void InstallVideoOutputAllocationHook(void) {
    Method alloc = class_getClassMethod(AVCaptureVideoDataOutput.class, @selector(allocWithZone:));
    Class meta = object_getClass(AVCaptureVideoDataOutput.class);
    if (!alloc || !meta) {
        Log(@"missing video output allocation hook");
        return;
    }
    class_replaceMethod(meta,
                        @selector(allocWithZone:),
                        (IMP)SimDeckVideoDataOutputAllocWithZone,
                        method_getTypeEncoding(alloc));
    DebugLog(@"hooked video output allocation");
}

@interface SimDeckCameraDevice : AVCaptureDevice
@end

@implementation SimDeckCameraDevice

+ (instancetype)sharedDevice {
    static SimDeckCameraDevice *device;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        device = NSAllocateObject(self, 0, nil);
    });
    return device;
}

- (NSString *)localizedName { return @"SimDeck Camera"; }
- (NSString *)uniqueID { return @"dev.nativescript.simdeck.camera"; }
- (AVCaptureDevicePosition)position { return AVCaptureDevicePositionBack; }
- (BOOL)hasMediaType:(AVMediaType)mediaType { return IsVideoMediaType(mediaType); }
- (BOOL)isConnected { return YES; }
- (BOOL)isSuspended { return NO; }
- (AVCaptureDeviceType)deviceType { return AVCaptureDeviceTypeBuiltInWideAngleCamera; }

@end

@interface SimDeckCameraInput : AVCaptureDeviceInput
@end

@implementation SimDeckCameraInput

- (AVCaptureDevice *)device {
    return (AVCaptureDevice *)[SimDeckCameraDevice sharedDevice];
}

@end

@interface SimDeckCameraVideoDataOutput : AVCaptureVideoDataOutput
@end

@implementation SimDeckCameraVideoDataOutput

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wobjc-designated-initializers"
- (instancetype)init {
    struct objc_super superInfo = { self, NSObject.class };
    return ((id (*)(struct objc_super *, SEL))objc_msgSendSuper)(&superInfo, @selector(init));
}
#pragma clang diagnostic pop

- (void)setVideoSettings:(NSDictionary *)videoSettings {
    objc_setAssociatedObject(self, &kOutputVideoSettingsKey, videoSettings, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
}

- (NSDictionary *)videoSettings {
    return objc_getAssociatedObject(self, &kOutputVideoSettingsKey);
}

- (void)setAlwaysDiscardsLateVideoFrames:(BOOL)alwaysDiscardsLateVideoFrames {
    objc_setAssociatedObject(self, &kOutputDiscardsLateFramesKey, @(alwaysDiscardsLateVideoFrames), OBJC_ASSOCIATION_RETAIN_NONATOMIC);
}

- (BOOL)alwaysDiscardsLateVideoFrames {
    NSNumber *value = objc_getAssociatedObject(self, &kOutputDiscardsLateFramesKey);
    return value ? value.boolValue : YES;
}

- (void)setSampleBufferDelegate:(id<AVCaptureVideoDataOutputSampleBufferDelegate>)delegate queue:(dispatch_queue_t)sampleBufferCallbackQueue {
    DebugLog(@"fake video output delegate set");
    SimDeckSetSampleBufferDelegate(self, _cmd, delegate, sampleBufferCallbackQueue);
}

@end

@interface SimDeckCameraPhoto : AVCapturePhoto
@property (nonatomic, strong) NSData *jpegData;
@end

@implementation SimDeckCameraPhoto

- (NSData *)fileDataRepresentation {
    return self.jpegData;
}

@end

@implementation AVCaptureDevice (SimDeckCamera)

+ (AVCaptureDevice *)sd_defaultDeviceWithMediaType:(AVMediaType)mediaType {
    AVCaptureDevice *device = [self sd_defaultDeviceWithMediaType:mediaType];
    if (device || !IsVideoMediaType(mediaType) || !OpenSharedCamera()) return device;
    return (AVCaptureDevice *)[SimDeckCameraDevice sharedDevice];
}

+ (AVCaptureDevice *)sd_defaultDeviceWithDeviceType:(AVCaptureDeviceType)deviceType
                                          mediaType:(AVMediaType)mediaType
                                           position:(AVCaptureDevicePosition)position {
    AVCaptureDevice *device = [self sd_defaultDeviceWithDeviceType:deviceType mediaType:mediaType position:position];
    if (device || !IsVideoMediaType(mediaType) || !OpenSharedCamera()) return device;
    return (AVCaptureDevice *)[SimDeckCameraDevice sharedDevice];
}

+ (NSArray<AVCaptureDevice *> *)sd_devicesWithMediaType:(AVMediaType)mediaType {
    NSArray *devices = [self sd_devicesWithMediaType:mediaType];
    if (devices.count > 0 || !IsVideoMediaType(mediaType)) return devices;
    return OpenSharedCamera() ? @[ (AVCaptureDevice *)[SimDeckCameraDevice sharedDevice] ] : @[];
}

+ (AVAuthorizationStatus)sd_authorizationStatusForMediaType:(AVMediaType)mediaType {
    if (IsVideoMediaType(mediaType)) return AVAuthorizationStatusAuthorized;
    return [self sd_authorizationStatusForMediaType:mediaType];
}

+ (void)sd_requestAccessForMediaType:(AVMediaType)mediaType completionHandler:(void (^)(BOOL granted))handler {
    if (IsVideoMediaType(mediaType)) {
        if (handler) dispatch_async(dispatch_get_main_queue(), ^{ handler(YES); });
        return;
    }
    [self sd_requestAccessForMediaType:mediaType completionHandler:handler];
}

@end

@implementation AVCaptureDeviceDiscoverySession (SimDeckCamera)

- (NSArray<AVCaptureDevice *> *)sd_devices {
    NSArray *devices = [self sd_devices];
    if (devices.count > 0) return devices;
    return OpenSharedCamera() ? @[ (AVCaptureDevice *)[SimDeckCameraDevice sharedDevice] ] : @[];
}

@end

@implementation AVCaptureDeviceInput (SimDeckCamera)

+ (instancetype)sd_deviceInputWithDevice:(AVCaptureDevice *)device error:(NSError **)outError {
    if ([device isKindOfClass:SimDeckCameraDevice.class]) {
        if (outError) *outError = nil;
        return (id)SimDeckFakeInput();
    }
    id input = [self sd_deviceInputWithDevice:device error:outError];
    if (!input && OpenSharedCamera() && [device hasMediaType:AVMediaTypeVideo]) {
        if (outError) *outError = nil;
        return (id)SimDeckFakeInput();
    }
    return input;
}

- (instancetype)sd_initWithDevice:(AVCaptureDevice *)device error:(NSError **)outError {
    if ([device isKindOfClass:SimDeckCameraDevice.class]) {
        if (outError) *outError = nil;
        return (id)SimDeckFakeInput();
    }
    return [self sd_initWithDevice:device error:outError];
}

- (AVCaptureDevice *)sd_device {
    AVCaptureDevice *device = objc_getAssociatedObject(self, &kInputDeviceKey);
    return device ?: [self sd_device];
}

- (NSArray *)sd_ports {
    if (SimDeckIsFakeInput(self)) return @[];
    return [self sd_ports];
}

@end

@implementation AVCaptureVideoDataOutput (SimDeckCamera)

+ (id)sd_allocWithZone:(struct _NSZone *)zone {
    if (self == AVCaptureVideoDataOutput.class && OpenSharedCamera()) {
        return NSAllocateObject(SimDeckCameraVideoDataOutput.class, 0, nil);
    }
    return [self sd_allocWithZone:zone];
}

- (instancetype)sd_init {
    id output = [self sd_init];
    InstallVideoOutputDelegateHook(object_getClass(output));
    return output;
}

- (void)sd_setSampleBufferDelegate:(id<AVCaptureVideoDataOutputSampleBufferDelegate>)delegate queue:(dispatch_queue_t)sampleBufferCallbackQueue {
    SimDeckSetSampleBufferDelegate(self, _cmd, delegate, sampleBufferCallbackQueue);
}

@end

@implementation AVCapturePhotoOutput (SimDeckCamera)

- (void)sd_capturePhotoWithSettings:(AVCapturePhotoSettings *)settings delegate:(id<AVCapturePhotoCaptureDelegate>)delegate {
    (void)settings;
    NSMutableData *data = nil;
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t bytesPerRow = 0;
    uint64_t ts = 0;
    if (!delegate || !CopyCurrentFrame(&data, &width, &height, &bytesPerRow, &ts)) {
        [self sd_capturePhotoWithSettings:settings delegate:delegate];
        return;
    }
    CGImageRef image = CreateImage(data, width, height, bytesPerRow);
    NSData *jpeg = image ? UIImageJPEGRepresentation([UIImage imageWithCGImage:image], 0.92) : nil;
    if (image) CGImageRelease(image);
    SimDeckCameraPhoto *photo = NSAllocateObject(SimDeckCameraPhoto.class, 0, nil);
    photo.jpegData = jpeg ?: [NSData data];
    dispatch_async(dispatch_get_main_queue(), ^{
        if ([delegate respondsToSelector:@selector(captureOutput:didFinishProcessingPhoto:error:)]) {
            [delegate captureOutput:self didFinishProcessingPhoto:photo error:nil];
        }
    });
}

@end

@implementation AVCaptureSession (SimDeckCamera)

- (BOOL)sd_canAddInput:(AVCaptureInput *)input {
    if (SimDeckIsFakeInput(input)) return YES;
    return [self sd_canAddInput:input];
}

- (void)sd_addInput:(AVCaptureInput *)input {
    if (SimDeckIsFakeInput(input)) {
        NSMutableArray *inputs = objc_getAssociatedObject(self, &kSessionInputsKey);
        if (!inputs) {
            inputs = [NSMutableArray array];
            objc_setAssociatedObject(self, &kSessionInputsKey, inputs, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        }
        if (![inputs containsObject:input]) [inputs addObject:input];
        return;
    }
    [self sd_addInput:input];
}

- (BOOL)sd_canAddOutput:(AVCaptureOutput *)output {
    if ([output isKindOfClass:AVCaptureVideoDataOutput.class] || [output isKindOfClass:AVCapturePhotoOutput.class]) return YES;
    return [self sd_canAddOutput:output];
}

- (void)sd_addOutput:(AVCaptureOutput *)output {
    if ([output isKindOfClass:AVCaptureVideoDataOutput.class] || [output isKindOfClass:AVCapturePhotoOutput.class]) {
        AddSessionOutput(self, output);
        return;
    }
    [self sd_addOutput:output];
}

- (NSArray<AVCaptureInput *> *)sd_inputs {
    NSArray *original = [self sd_inputs];
    NSMutableArray *inputs = objc_getAssociatedObject(self, &kSessionInputsKey);
    if (inputs.count == 0) return original;
    return [original arrayByAddingObjectsFromArray:inputs];
}

- (NSArray<AVCaptureOutput *> *)sd_outputs {
    NSArray *original = [self sd_outputs];
    NSMutableArray *outputs = objc_getAssociatedObject(self, &kSessionOutputsKey);
    if (outputs.count == 0) return original;
    return [original arrayByAddingObjectsFromArray:outputs];
}

- (void)sd_startRunning {
    objc_setAssociatedObject(self, &kSessionRunningKey, @YES, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    @synchronized(gSessions) {
        TrackPointer(gSessions, self);
    }
    StartFrameTimer();
}

- (void)sd_stopRunning {
    objc_setAssociatedObject(self, &kSessionRunningKey, @NO, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
}

- (BOOL)sd_isRunning {
    NSNumber *running = objc_getAssociatedObject(self, &kSessionRunningKey);
    if (running) return running.boolValue;
    return [self sd_isRunning];
}

@end

@interface SimDeckPickerOverlayView : UIView
@end

@implementation SimDeckPickerOverlayView

- (UIView *)hitTest:(CGPoint)point withEvent:(UIEvent *)event {
    UIView *hit = [super hitTest:point withEvent:event];
    return hit == self ? nil : hit;
}

@end

@interface SimDeckPickerOverlayWindow : UIWindow
@end

@implementation SimDeckPickerOverlayWindow

- (UIView *)hitTest:(CGPoint)point withEvent:(UIEvent *)event {
    UIView *hit = [super hitTest:point withEvent:event];
    return hit == self || hit == self.rootViewController.view ? nil : hit;
}

@end

@interface SimDeckPickerCaptureControl : UIControl
@property (nonatomic, assign) UIImagePickerController *picker;
- (instancetype)initWithPicker:(UIImagePickerController *)picker;
- (void)capture;
@end

@implementation SimDeckPickerCaptureControl

- (instancetype)initWithPicker:(UIImagePickerController *)picker {
    self = [super initWithFrame:CGRectZero];
    if (self) {
        self.picker = picker;
        self.backgroundColor = UIColor.clearColor;
        self.accessibilityLabel = @"SimDeck Capture";
        [self addTarget:self action:@selector(capture) forControlEvents:UIControlEventTouchUpInside];
    }
    return self;
}

- (void)capture {
    UIImagePickerController *picker = [self.picker retain];
    dispatch_async(dispatch_get_main_queue(), ^{
        SendPickerCapture(picker);
        [picker release];
    });
}

@end

static CGRect PickerCaptureFrame(UIView *view) {
    CGRect bounds = view.bounds;
    CGFloat width = CGRectGetWidth(bounds);
    CGFloat height = CGRectGetHeight(bounds);
    CGFloat size = MAX((CGFloat)104.0, MIN(width, height) * (CGFloat)0.26);
    if (width > height) {
        CGFloat centerX = width - MAX((CGFloat)120.0, height * (CGFloat)0.24);
        CGFloat centerY = height * (CGFloat)0.5;
        return CGRectMake(centerX - size * (CGFloat)0.5, centerY - size * (CGFloat)0.5, size, size);
    }
    CGFloat centerX = width * (CGFloat)0.5;
    CGFloat centerY = height - MAX((CGFloat)140.0, width * (CGFloat)0.35);
    return CGRectMake(centerX - size * (CGFloat)0.5, centerY - size * (CGFloat)0.5, size, size);
}

static void SendPickerCapture(UIImagePickerController *picker) {
    if (!picker) return;
    UIImage *image = CurrentFrameImage();
    if (!image) return;
    DebugLog(@"sending UIImagePicker simulated capture");
    NSDictionary *info = @{
        UIImagePickerControllerMediaType: @"public.image",
        UIImagePickerControllerOriginalImage: image,
        UIImagePickerControllerMediaMetadata: @{},
    };
    id<UIImagePickerControllerDelegate, UINavigationControllerDelegate> delegate = picker.delegate;
    if ([delegate respondsToSelector:@selector(imagePickerController:didFinishPickingMediaWithInfo:)]) {
        [delegate imagePickerController:picker didFinishPickingMediaWithInfo:info];
    } else {
        [picker dismissViewControllerAnimated:YES completion:nil];
    }
}

static CGRect PickerPreviewFrame(UIView *view) {
    CGRect bounds = view.bounds;
    CGFloat width = CGRectGetWidth(bounds);
    CGFloat height = CGRectGetHeight(bounds);
    if (width <= 0 || height <= 0) return bounds;
    if (width > height) {
        CGFloat previewWidth = MIN(width, height * 4.0 / 3.0);
        CGFloat x = (width - previewWidth) * 0.5;
        return CGRectMake(x, 0, previewWidth, height);
    }
    CGFloat previewHeight = MIN(height, width * 4.0 / 3.0);
    CGFloat bottomControls = MAX((CGFloat)150.0, width * (CGFloat)0.48);
    CGFloat y = height - previewHeight - bottomControls;
    if (y < 0) {
        y = (height - previewHeight) * 0.5;
    }
    y = MAX((CGFloat)0.0, MIN(y, height - previewHeight));
    return CGRectMake(0, y, width, previewHeight);
}

static void InstallPickerOverlay(UIImagePickerController *picker) {
    if (!picker || !OpenSharedCamera()) return;
    if (picker.sourceType != UIImagePickerControllerSourceTypeCamera) return;
    UIView *root = picker.view;
    if (!root) return;

    UIView *overlay = objc_getAssociatedObject(picker, &kPickerOverlayViewKey);
    if (!overlay) {
        overlay = [[[UIView alloc] initWithFrame:PickerPreviewFrame(root)] autorelease];
        overlay.userInteractionEnabled = NO;
        overlay.clipsToBounds = YES;
        overlay.backgroundColor = UIColor.clearColor;
        overlay.layer.contentsGravity = kCAGravityResizeAspectFill;
        overlay.layer.masksToBounds = YES;
        objc_setAssociatedObject(picker, &kPickerOverlayViewKey, overlay, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        [root addSubview:overlay];
        DebugLog(@"installed UIImagePicker camera preview overlay");
    }
    overlay.frame = PickerPreviewFrame(root);
    RegisterOutputLayer(overlay.layer);

    SimDeckPickerOverlayView *cameraOverlay = objc_getAssociatedObject(picker, &kPickerCameraOverlayKey);
    if (!cameraOverlay) {
        cameraOverlay = [[[SimDeckPickerOverlayView alloc] initWithFrame:root.bounds] autorelease];
        cameraOverlay.backgroundColor = UIColor.clearColor;
        cameraOverlay.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        objc_setAssociatedObject(picker, &kPickerCameraOverlayKey, cameraOverlay, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        picker.cameraOverlayView = cameraOverlay;
    }
    cameraOverlay.frame = root.bounds;

    UIWindow *hostWindow = root.window;
    if (!hostWindow) {
        for (UIScene *scene in UIApplication.sharedApplication.connectedScenes) {
            if (![scene isKindOfClass:UIWindowScene.class]) continue;
            for (UIWindow *window in ((UIWindowScene *)scene).windows) {
                if (window.isKeyWindow) {
                    hostWindow = window;
                    break;
                }
            }
            if (hostWindow) break;
        }
    }
    CGRect captureFrame = PickerCaptureFrame(root);
    SimDeckPickerOverlayWindow *captureWindow = objc_getAssociatedObject(picker, &kPickerCaptureWindowKey);
    if (!captureWindow) {
        captureWindow = [[[SimDeckPickerOverlayWindow alloc] initWithFrame:captureFrame] autorelease];
        captureWindow.windowLevel = UIWindowLevelAlert + 10.0;
        captureWindow.backgroundColor = UIColor.clearColor;
        UIViewController *rootController = [[[UIViewController alloc] init] autorelease];
        rootController.view = [[[SimDeckPickerOverlayView alloc] initWithFrame:captureWindow.bounds] autorelease];
        rootController.view.backgroundColor = UIColor.clearColor;
        rootController.view.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        captureWindow.rootViewController = rootController;
        captureWindow.hidden = NO;
        objc_setAssociatedObject(picker, &kPickerCaptureWindowKey, captureWindow, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    if (@available(iOS 13.0, *)) {
        if (hostWindow.windowScene && captureWindow.windowScene != hostWindow.windowScene) {
            captureWindow.windowScene = hostWindow.windowScene;
        }
    }
    captureWindow.frame = captureFrame;
    captureWindow.hidden = NO;
    UIView *captureRoot = captureWindow.rootViewController.view ?: cameraOverlay;
    captureRoot.frame = captureWindow.bounds;

    SimDeckPickerCaptureControl *capture = objc_getAssociatedObject(picker, &kPickerCaptureControlKey);
    if (!capture) {
        capture = [[[SimDeckPickerCaptureControl alloc] initWithPicker:picker] autorelease];
        objc_setAssociatedObject(picker, &kPickerCaptureControlKey, capture, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        [captureRoot addSubview:capture];
        DebugLog(@"installed UIImagePicker capture control");
    } else if (capture.superview != captureRoot) {
        [capture removeFromSuperview];
        [captureRoot addSubview:capture];
    }
    capture.frame = captureRoot.bounds;
    [captureRoot bringSubviewToFront:capture];
}

static void HidePickerOverlayWindow(UIImagePickerController *picker) {
    UIWindow *captureWindow = objc_getAssociatedObject(picker, &kPickerCaptureWindowKey);
    captureWindow.hidden = YES;
}

@implementation UIViewController (SimDeckCameraPicker)

- (void)sd_viewDidAppear:(BOOL)animated {
    [self sd_viewDidAppear:animated];
    if ([self isKindOfClass:UIImagePickerController.class]) {
        InstallPickerOverlay((UIImagePickerController *)self);
    }
}

- (void)sd_viewDidLayoutSubviews {
    [self sd_viewDidLayoutSubviews];
    if ([self isKindOfClass:UIImagePickerController.class]) {
        InstallPickerOverlay((UIImagePickerController *)self);
    }
}

- (void)sd_viewDidDisappear:(BOOL)animated {
    [self sd_viewDidDisappear:animated];
    if ([self isKindOfClass:UIImagePickerController.class]) {
        HidePickerOverlayWindow((UIImagePickerController *)self);
    }
}

@end

@implementation AVCaptureVideoPreviewLayer (SimDeckCamera)

+ (instancetype)sd_layerWithSession:(AVCaptureSession *)session {
    AVCaptureVideoPreviewLayer *layer = [self sd_layerWithSession:session];
    if (OpenSharedCamera()) RegisterPreviewLayer(layer);
    return layer;
}

- (instancetype)sd_initWithSession:(AVCaptureSession *)session {
    id layer = [self sd_initWithSession:session];
    if (layer && OpenSharedCamera()) {
        RegisterPreviewLayer(layer);
    }
    return layer;
}

- (void)sd_setSession:(AVCaptureSession *)session {
    [self sd_setSession:session];
    RegisterPreviewLayer(self);
}

@end

static void ExchangeInstance(Class cls, SEL original, SEL replacement) {
    Method a = class_getInstanceMethod(cls, original);
    Method b = class_getInstanceMethod(cls, replacement);
    if (!a || !b) {
        Log(@"missing instance method %@ on %@", NSStringFromSelector(original), NSStringFromClass(cls));
        return;
    }
    method_exchangeImplementations(a, b);
}

static void ExchangeClass(Class cls, SEL original, SEL replacement) {
    Method a = class_getClassMethod(cls, original);
    Method b = class_getClassMethod(cls, replacement);
    if (!a || !b) {
        Log(@"missing class method %@ on %@", NSStringFromSelector(original), NSStringFromClass(cls));
        return;
    }
    method_exchangeImplementations(a, b);
}

__attribute__((constructor))
static void SimDeckCameraInstall(void) {
    @autoreleasepool {
        gSessions = [[NSMutableArray alloc] init];
        gVideoOutputs = [[NSMutableArray alloc] init];
        gPreviewLayers = [[NSHashTable weakObjectsHashTable] retain];
        gHookedVideoOutputClasses = [[NSMutableSet alloc] init];
        OpenSharedCamera();

        ExchangeClass(AVCaptureDevice.class, @selector(defaultDeviceWithMediaType:), @selector(sd_defaultDeviceWithMediaType:));
        ExchangeClass(AVCaptureDevice.class, @selector(defaultDeviceWithDeviceType:mediaType:position:), @selector(sd_defaultDeviceWithDeviceType:mediaType:position:));
        ExchangeClass(AVCaptureDevice.class, @selector(devicesWithMediaType:), @selector(sd_devicesWithMediaType:));
        ExchangeClass(AVCaptureDevice.class, @selector(authorizationStatusForMediaType:), @selector(sd_authorizationStatusForMediaType:));
        ExchangeClass(AVCaptureDevice.class, @selector(requestAccessForMediaType:completionHandler:), @selector(sd_requestAccessForMediaType:completionHandler:));
        ExchangeInstance(AVCaptureDeviceDiscoverySession.class, @selector(devices), @selector(sd_devices));

        ExchangeClass(AVCaptureDeviceInput.class, @selector(deviceInputWithDevice:error:), @selector(sd_deviceInputWithDevice:error:));
        ExchangeInstance(AVCaptureDeviceInput.class, @selector(initWithDevice:error:), @selector(sd_initWithDevice:error:));
        ExchangeInstance(AVCaptureDeviceInput.class, @selector(device), @selector(sd_device));
        ExchangeInstance(AVCaptureDeviceInput.class, @selector(ports), @selector(sd_ports));

        InstallVideoOutputAllocationHook();
        InstallExistingVideoOutputDelegateHooks();
        ExchangeInstance(AVCapturePhotoOutput.class, @selector(capturePhotoWithSettings:delegate:), @selector(sd_capturePhotoWithSettings:delegate:));

        ExchangeInstance(AVCaptureSession.class, @selector(canAddInput:), @selector(sd_canAddInput:));
        ExchangeInstance(AVCaptureSession.class, @selector(addInput:), @selector(sd_addInput:));
        ExchangeInstance(AVCaptureSession.class, @selector(canAddOutput:), @selector(sd_canAddOutput:));
        ExchangeInstance(AVCaptureSession.class, @selector(addOutput:), @selector(sd_addOutput:));
        ExchangeInstance(AVCaptureSession.class, @selector(inputs), @selector(sd_inputs));
        ExchangeInstance(AVCaptureSession.class, @selector(outputs), @selector(sd_outputs));
        ExchangeInstance(AVCaptureSession.class, @selector(startRunning), @selector(sd_startRunning));
        ExchangeInstance(AVCaptureSession.class, @selector(stopRunning), @selector(sd_stopRunning));
        ExchangeInstance(AVCaptureSession.class, @selector(isRunning), @selector(sd_isRunning));

        ExchangeInstance(UIViewController.class, @selector(viewDidAppear:), @selector(sd_viewDidAppear:));
        ExchangeInstance(UIViewController.class, @selector(viewDidLayoutSubviews), @selector(sd_viewDidLayoutSubviews));
        ExchangeInstance(UIViewController.class, @selector(viewDidDisappear:), @selector(sd_viewDidDisappear:));
        ExchangeClass(AVCaptureVideoPreviewLayer.class, @selector(layerWithSession:), @selector(sd_layerWithSession:));
        ExchangeInstance(AVCaptureVideoPreviewLayer.class, @selector(initWithSession:), @selector(sd_initWithSession:));
        ExchangeInstance(AVCaptureVideoPreviewLayer.class, @selector(setSession:), @selector(sd_setSession:));
        DebugLog(@"installed");
    }
}
