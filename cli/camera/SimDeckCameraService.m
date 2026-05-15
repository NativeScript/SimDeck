#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreImage/CoreImage.h>
#import <Foundation/Foundation.h>

#import "SimDeckCameraShared.h"

#import <CoreVideo/CoreVideo.h>
#import <dispatch/dispatch.h>
#import <fcntl.h>
#import <signal.h>
#import <stdatomic.h>
#import <stdbool.h>
#import <stdint.h>
#import <stdio.h>
#import <stdlib.h>
#import <string.h>
#import <sys/mman.h>
#import <sys/stat.h>
#import <unistd.h>

static uint32_t gWidth = 1280;
static uint32_t gHeight = 720;
static char *gShmName = NULL;
static SimDeckCameraHeader *gHeader = NULL;
static uint8_t *gPixels = NULL;
static size_t gMappedSize = 0;
static dispatch_queue_t gWriteQueue;
static dispatch_source_t gPlaceholderTimer;
static AVCaptureSession *gWebcamSession;
static id gWebcamDelegate;
static atomic_uint gSourceGeneration;
static atomic_ullong gPublishedFrames;
static atomic_ullong gDroppedFrames;
static NSString *gSourceName = nil;
static NSString *gSourceArgument = nil;
static uint32_t gSourceKind = SIMDECK_CAMERA_SOURCE_PLACEHOLDER;
static OSType gLastPixelFormat = 0;
static BOOL gServiceStarted = NO;
static NSString *gActiveUDID = nil;

static NSObject *CameraLock(void) {
    static NSObject *lock;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        lock = [NSObject new];
    });
    return lock;
}

static void RunOnMainSync(dispatch_block_t block) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_sync(dispatch_get_main_queue(), block);
    }
}

static void RunMainRunLoopUntil(BOOL (^isFinished)(void), NSTimeInterval timeout) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:timeout];
    while (!isFinished() && [deadline timeIntervalSinceNow] > 0) {
        [[NSRunLoop mainRunLoop] runMode:NSDefaultRunLoopMode
                              beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.1]];
    }
}

static uint64_t NowNs(void) {
    return (uint64_t)([[NSDate date] timeIntervalSince1970] * 1000000000.0);
}

static NSString *StringFromCString(const char *value) {
    return value ? [NSString stringWithUTF8String:value] ?: @"" : @"";
}

static NSString *FourCCString(OSType value) {
    char chars[5] = {
        (char)((value >> 24) & 0xff),
        (char)((value >> 16) & 0xff),
        (char)((value >> 8) & 0xff),
        (char)(value & 0xff),
        '\0',
    };
    for (NSUInteger index = 0; index < 4; index += 1) {
        if (chars[index] < 32 || chars[index] > 126) {
            return [NSString stringWithFormat:@"0x%08x", value];
        }
    }
    return [NSString stringWithUTF8String:chars] ?: [NSString stringWithFormat:@"0x%08x", value];
}

static NSString *AuthorizationStatusName(AVAuthorizationStatus status) {
    switch (status) {
        case AVAuthorizationStatusAuthorized: return @"authorized";
        case AVAuthorizationStatusDenied: return @"denied";
        case AVAuthorizationStatusRestricted: return @"restricted";
        case AVAuthorizationStatusNotDetermined: return @"not-determined";
    }
    return @"unknown";
}

static BOOL EnsureCameraAccess(NSString **error) {
    AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo];
    if (status == AVAuthorizationStatusAuthorized) {
        return YES;
    }
    if (status == AVAuthorizationStatusNotDetermined) {
        __block BOOL granted = NO;
        __block BOOL finished = NO;
        void (^requestAccess)(void) = ^{
            [NSApplication sharedApplication];
            [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
            [NSApp activateIgnoringOtherApps:YES];
            [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL didGrant) {
                granted = didGrant;
                finished = YES;
            }];
        };
        if ([NSThread isMainThread]) {
            requestAccess();
            RunMainRunLoopUntil(^BOOL{
                return finished;
            }, 60.0);
        } else {
            dispatch_async(dispatch_get_main_queue(), requestAccess);
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:60.0];
            while (!finished && [deadline timeIntervalSinceNow] > 0) {
                usleep(50 * 1000);
            }
        }
        if (granted) {
            return YES;
        }
        status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo];
    }
    if (error) {
        *error = [NSString stringWithFormat:@"Mac camera permission is %@ for SimDeck.", AuthorizationStatusName(status)];
    }
    return NO;
}

static uint32_t SourceKindForName(NSString *name) {
    NSString *lower = name.lowercaseString;
    if ([lower isEqualToString:@"image"]) return SIMDECK_CAMERA_SOURCE_IMAGE;
    if ([lower isEqualToString:@"video"]) return SIMDECK_CAMERA_SOURCE_VIDEO;
    if ([lower isEqualToString:@"webcam"]) return SIMDECK_CAMERA_SOURCE_WEBCAM;
    return SIMDECK_CAMERA_SOURCE_PLACEHOLDER;
}

static NSString *SourceNameForKind(uint32_t kind) {
    switch (kind) {
        case SIMDECK_CAMERA_SOURCE_IMAGE: return @"image";
        case SIMDECK_CAMERA_SOURCE_VIDEO: return @"video";
        case SIMDECK_CAMERA_SOURCE_WEBCAM: return @"webcam";
        default: return @"placeholder";
    }
}

static NSString *MirrorName(uint32_t mode) {
    switch (mode) {
        case SIMDECK_CAMERA_MIRROR_ON: return @"on";
        case SIMDECK_CAMERA_MIRROR_OFF: return @"off";
        default: return @"auto";
    }
}

static uint32_t MirrorModeForName(NSString *name) {
    NSString *lower = name.lowercaseString;
    if ([lower isEqualToString:@"on"]) return SIMDECK_CAMERA_MIRROR_ON;
    if ([lower isEqualToString:@"off"]) return SIMDECK_CAMERA_MIRROR_OFF;
    return SIMDECK_CAMERA_MIRROR_AUTO;
}

static void SetSourceMetadata(uint32_t sourceKind, NSString *argument) {
    if (!gHeader) return;
    gHeader->sourceKind = sourceKind;
    memset(gHeader->sourceLabel, 0, sizeof(gHeader->sourceLabel));
    NSString *label = argument.length > 0 ? argument : SourceNameForKind(sourceKind);
    NSData *labelData = [label dataUsingEncoding:NSUTF8StringEncoding];
    if (labelData.length > 0) {
        memcpy(gHeader->sourceLabel,
               labelData.bytes,
               MIN(labelData.length, sizeof(gHeader->sourceLabel) - 1));
    }
}

static void SetSourceState(uint32_t sourceKind, NSString *name, NSString *argument) {
    gSourceKind = sourceKind;
    gSourceName = [name copy];
    gSourceArgument = [argument copy];
}

static void PublishBGRA(const uint8_t *source,
                        uint32_t sourceWidth,
                        uint32_t sourceHeight,
                        size_t sourceBytesPerRow,
                        uint32_t sourceKind,
                        NSString *label) {
    if (!gHeader || !gPixels || !source || sourceWidth == 0 || sourceHeight == 0) return;
    dispatch_sync(gWriteQueue, ^{
        gHeader->sequence += 1;
        for (uint32_t y = 0; y < gHeight; y += 1) {
            uint32_t sy = (uint32_t)(((uint64_t)y * sourceHeight) / MAX(gHeight, 1));
            const uint8_t *sourceRow = source + ((size_t)sy * sourceBytesPerRow);
            uint8_t *destRow = gPixels + ((size_t)y * gHeader->bytesPerRow);
            for (uint32_t x = 0; x < gWidth; x += 1) {
                uint32_t sx = (uint32_t)(((uint64_t)x * sourceWidth) / MAX(gWidth, 1));
                const uint8_t *pixel = sourceRow + ((size_t)sx * 4);
                uint8_t *out = destRow + ((size_t)x * 4);
                out[0] = pixel[0];
                out[1] = pixel[1];
                out[2] = pixel[2];
                out[3] = 0xff;
            }
        }
        gHeader->timestampNs = NowNs();
        gHeader->sourceKind = sourceKind;
        SetSourceMetadata(sourceKind, label);
        gHeader->sequence += 1;
        atomic_fetch_add(&gPublishedFrames, 1);
    });
}

static void PublishPixelBuffer(CVPixelBufferRef pixelBuffer, uint32_t sourceKind, NSString *label) {
    if (!pixelBuffer) return;
    OSType format = CVPixelBufferGetPixelFormatType(pixelBuffer);
    gLastPixelFormat = format;
    if (format == kCVPixelFormatType_32BGRA) {
        CVPixelBufferLockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
        PublishBGRA((const uint8_t *)CVPixelBufferGetBaseAddress(pixelBuffer),
                    (uint32_t)CVPixelBufferGetWidth(pixelBuffer),
                    (uint32_t)CVPixelBufferGetHeight(pixelBuffer),
                    CVPixelBufferGetBytesPerRow(pixelBuffer),
                    sourceKind,
                    label);
        CVPixelBufferUnlockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
        return;
    }

    CIImage *image = [CIImage imageWithCVPixelBuffer:pixelBuffer];
    if (!image) {
        atomic_fetch_add(&gDroppedFrames, 1);
        return;
    }
    static CIContext *context;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        context = [CIContext contextWithOptions:@{ kCIContextWorkingColorSpace: [NSNull null] }];
    });
    size_t width = CVPixelBufferGetWidth(pixelBuffer);
    size_t height = CVPixelBufferGetHeight(pixelBuffer);
    size_t bytesPerRow = width * 4;
    NSMutableData *data = [NSMutableData dataWithLength:bytesPerRow * height];
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    [context render:image
           toBitmap:data.mutableBytes
           rowBytes:bytesPerRow
             bounds:CGRectMake(0, 0, width, height)
             format:kCIFormatBGRA8
         colorSpace:colorSpace];
    CGColorSpaceRelease(colorSpace);
    PublishBGRA(data.bytes,
                (uint32_t)width,
                (uint32_t)height,
                bytesPerRow,
                sourceKind,
                label);
}

static void DrawPlaceholderFrame(uint32_t frameIndex) {
    if (!gHeader || !gPixels) return;
    dispatch_sync(gWriteQueue, ^{
        gHeader->sequence += 1;
        for (uint32_t y = 0; y < gHeight; y += 1) {
            uint8_t *row = gPixels + ((size_t)y * gHeader->bytesPerRow);
            for (uint32_t x = 0; x < gWidth; x += 1) {
                uint8_t *p = row + ((size_t)x * 4);
                uint8_t stripe = (uint8_t)(((x / 80) + (frameIndex / 6)) % 2 ? 56 : 24);
                p[0] = (uint8_t)((x + frameIndex * 7) % 256);
                p[1] = (uint8_t)((y + frameIndex * 3) % 256);
                p[2] = (uint8_t)(180 + stripe);
                p[3] = 0xff;
            }
        }
        gHeader->timestampNs = NowNs();
        SetSourceMetadata(SIMDECK_CAMERA_SOURCE_PLACEHOLDER, @"placeholder");
        gHeader->sequence += 1;
        atomic_fetch_add(&gPublishedFrames, 1);
    });
}

static BOOL PublishImageAtPath(NSString *path, NSString **error) {
    NSImage *image = [[NSImage alloc] initWithContentsOfFile:path];
    CGImageRef cgImage = [image CGImageForProposedRect:NULL context:nil hints:nil];
    if (!cgImage) {
        if (error) *error = [NSString stringWithFormat:@"Unable to decode image at %@", path];
        return NO;
    }
    size_t sourceWidth = CGImageGetWidth(cgImage);
    size_t sourceHeight = CGImageGetHeight(cgImage);
    size_t bytesPerRow = sourceWidth * 4;
    NSMutableData *data = [NSMutableData dataWithLength:bytesPerRow * sourceHeight];
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(data.mutableBytes,
                                                 sourceWidth,
                                                 sourceHeight,
                                                 8,
                                                 bytesPerRow,
                                                 colorSpace,
                                                 kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst);
    CGColorSpaceRelease(colorSpace);
    if (!context) {
        if (error) *error = @"Unable to allocate image conversion buffer.";
        return NO;
    }
    CGContextDrawImage(context, CGRectMake(0, 0, sourceWidth, sourceHeight), cgImage);
    CGContextRelease(context);
    PublishBGRA(data.bytes,
                (uint32_t)sourceWidth,
                (uint32_t)sourceHeight,
                bytesPerRow,
                SIMDECK_CAMERA_SOURCE_IMAGE,
                path);
    return YES;
}

@interface SimDeckCameraWebcamWriter : NSObject <AVCaptureVideoDataOutputSampleBufferDelegate>
@property (nonatomic, copy) NSString *label;
@end

@implementation SimDeckCameraWebcamWriter

- (void)captureOutput:(AVCaptureOutput *)output
 didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
       fromConnection:(AVCaptureConnection *)connection {
    (void)output;
    (void)connection;
    PublishPixelBuffer(CMSampleBufferGetImageBuffer(sampleBuffer),
                       SIMDECK_CAMERA_SOURCE_WEBCAM,
                       self.label);
}

@end

static NSArray<AVCaptureDevice *> *CameraDevices(void) {
    AVCaptureDeviceDiscoverySession *session = [AVCaptureDeviceDiscoverySession
        discoverySessionWithDeviceTypes:@[
            AVCaptureDeviceTypeBuiltInWideAngleCamera,
            AVCaptureDeviceTypeExternal,
            AVCaptureDeviceTypeContinuityCamera,
        ]
                              mediaType:AVMediaTypeVideo
                               position:AVCaptureDevicePositionUnspecified];
    return session.devices ?: @[];
}

static AVCaptureDevice *PickCamera(NSString *wanted) {
    NSArray<AVCaptureDevice *> *devices = CameraDevices();
    if (wanted.length == 0) {
        for (AVCaptureDevice *device in devices) {
            if (device.position == AVCaptureDevicePositionFront) return device;
        }
        return devices.firstObject;
    }
    NSString *needle = wanted.lowercaseString;
    for (AVCaptureDevice *device in devices) {
        if ([device.uniqueID.lowercaseString isEqualToString:needle]) return device;
    }
    for (AVCaptureDevice *device in devices) {
        if ([device.localizedName.lowercaseString containsString:needle]) return device;
    }
    return nil;
}

static void StopCurrentSource(void) {
    atomic_fetch_add(&gSourceGeneration, 1);
    if (gPlaceholderTimer) {
        dispatch_source_cancel(gPlaceholderTimer);
        gPlaceholderTimer = nil;
    }
    if (gWebcamSession) {
        [gWebcamSession stopRunning];
        gWebcamSession = nil;
    }
    gWebcamDelegate = nil;
}

static BOOL StartPlaceholder(NSString **error) {
    (void)error;
    StopCurrentSource();
    SetSourceState(SIMDECK_CAMERA_SOURCE_PLACEHOLDER, @"placeholder", nil);
    __block uint32_t frame = 0;
    DrawPlaceholderFrame(frame);
    dispatch_queue_t queue = dispatch_queue_create("dev.nativescript.simdeck.camera.placeholder", DISPATCH_QUEUE_SERIAL);
    gPlaceholderTimer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, queue);
    dispatch_source_set_timer(gPlaceholderTimer,
                              dispatch_time(DISPATCH_TIME_NOW, 0),
                              (uint64_t)(NSEC_PER_SEC / 30),
                              (uint64_t)(NSEC_PER_MSEC * 5));
    dispatch_source_set_event_handler(gPlaceholderTimer, ^{
        DrawPlaceholderFrame(frame++);
    });
    dispatch_resume(gPlaceholderTimer);
    return YES;
}

static BOOL StartImage(NSString *path, NSString **error) {
    StopCurrentSource();
    if (!PublishImageAtPath(path, error)) {
        return NO;
    }
    SetSourceState(SIMDECK_CAMERA_SOURCE_IMAGE, @"image", path);
    return YES;
}

static BOOL StartVideo(NSString *path, NSString **error) {
    NSURL *url = nil;
    NSURLComponents *components = [NSURLComponents componentsWithString:path ?: @""];
    if (components.scheme.length > 0) {
        url = components.URL;
    } else if ([[NSFileManager defaultManager] fileExistsAtPath:path]) {
        url = [NSURL fileURLWithPath:path];
    }
    if (!url) {
        if (error) *error = [NSString stringWithFormat:@"Video file does not exist: %@", path];
        return NO;
    }
    StopCurrentSource();
    SetSourceState(SIMDECK_CAMERA_SOURCE_VIDEO, @"video", path);
    unsigned generation = atomic_load(&gSourceGeneration);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        while (atomic_load(&gSourceGeneration) == generation) {
            @autoreleasepool {
                AVAsset *asset = [AVAsset assetWithURL:url];
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
                NSArray<AVAssetTrack *> *tracks = [asset tracksWithMediaType:AVMediaTypeVideo];
#pragma clang diagnostic pop
                AVAssetTrack *track = tracks.firstObject;
                if (!track) {
                    usleep(300 * 1000);
                    continue;
                }
                NSError *readerError = nil;
                AVAssetReader *reader = [[AVAssetReader alloc] initWithAsset:asset error:&readerError];
                if (!reader) {
                    fprintf(stderr, "simdeck-camera: video reader failed: %s\n", readerError.localizedDescription.UTF8String);
                    usleep(300 * 1000);
                    continue;
                }
                NSDictionary *settings = @{
                    (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
                };
                AVAssetReaderTrackOutput *output = [[AVAssetReaderTrackOutput alloc] initWithTrack:track outputSettings:settings];
                output.alwaysCopiesSampleData = NO;
                if (![reader canAddOutput:output]) {
                    usleep(300 * 1000);
                    continue;
                }
                [reader addOutput:output];
                if (![reader startReading]) {
                    usleep(300 * 1000);
                    continue;
                }
                while (atomic_load(&gSourceGeneration) == generation && reader.status == AVAssetReaderStatusReading) {
                    CMSampleBufferRef sample = [output copyNextSampleBuffer];
                    if (!sample) break;
                    PublishPixelBuffer(CMSampleBufferGetImageBuffer(sample), SIMDECK_CAMERA_SOURCE_VIDEO, path);
                    CFRelease(sample);
                    usleep(33333);
                }
            }
        }
    });
    return YES;
}

static BOOL StartWebcam(NSString *requestedDevice, NSString **error) {
    StopCurrentSource();
    if (!EnsureCameraAccess(error)) {
        return NO;
    }
    AVCaptureDevice *device = PickCamera(requestedDevice);
    if (!device) {
        if (error) *error = requestedDevice.length > 0
            ? [NSString stringWithFormat:@"No matching Mac camera: %@", requestedDevice]
            : @"No Mac camera is available.";
        return NO;
    }
    NSError *inputError = nil;
    AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:&inputError];
    if (!input) {
        if (error) *error = inputError.localizedDescription ?: @"Unable to open Mac camera.";
        return NO;
    }
    AVCaptureSession *session = [[AVCaptureSession alloc] init];
    session.sessionPreset = AVCaptureSessionPreset1280x720;
    if (![session canAddInput:input]) {
        if (error) *error = @"Unable to attach Mac camera input.";
        return NO;
    }
    [session addInput:input];
    AVCaptureVideoDataOutput *output = [[AVCaptureVideoDataOutput alloc] init];
    output.videoSettings = @{ (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA) };
    output.alwaysDiscardsLateVideoFrames = YES;
    SimDeckCameraWebcamWriter *writer = [[SimDeckCameraWebcamWriter alloc] init];
    writer.label = device.localizedName ?: @"webcam";
    [output setSampleBufferDelegate:writer queue:dispatch_queue_create("dev.nativescript.simdeck.camera.webcam", DISPATCH_QUEUE_SERIAL)];
    if (![session canAddOutput:output]) {
        if (error) *error = @"Unable to attach Mac camera output.";
        return NO;
    }
    [session addOutput:output];
    gWebcamSession = session;
    gWebcamDelegate = writer;
    [session startRunning];
    if (!session.isRunning) {
        if (error) *error = @"Mac camera session did not start.";
        StopCurrentSource();
        return NO;
    }
    uint64_t startFrames = atomic_load(&gPublishedFrames);
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:4.0];
    while (atomic_load(&gPublishedFrames) == startFrames && [deadline timeIntervalSinceNow] > 0) {
        [[NSRunLoop mainRunLoop] runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        usleep(20 * 1000);
    }
    if (atomic_load(&gPublishedFrames) == startFrames) {
        if (error) {
            *error = [NSString stringWithFormat:@"Mac camera session started but delivered no frames. Authorization=%@ pixelFormat=%@",
                      AuthorizationStatusName([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]),
                      gLastPixelFormat ? FourCCString(gLastPixelFormat) : @"none"];
        }
        StopCurrentSource();
        return NO;
    }
    SetSourceState(SIMDECK_CAMERA_SOURCE_WEBCAM, @"webcam", device.uniqueID ?: device.localizedName);
    return YES;
}

static BOOL SwitchSource(NSString *source, NSString *argument, NSString **error) {
    uint32_t kind = SourceKindForName(source);
    switch (kind) {
        case SIMDECK_CAMERA_SOURCE_IMAGE:
            return StartImage(argument ?: @"", error);
        case SIMDECK_CAMERA_SOURCE_VIDEO:
            return StartVideo(argument ?: @"", error);
        case SIMDECK_CAMERA_SOURCE_WEBCAM:
            return StartWebcam(argument ?: @"", error);
        default:
            return StartPlaceholder(error);
    }
}

static NSDictionary *StatusPayload(BOOL ok, NSString *error) {
    NSMutableDictionary *payload = [@{
        @"ok": @(ok),
        @"alive": @YES,
        @"source": gSourceName ?: SourceNameForKind(gSourceKind),
        @"mirror": gHeader ? MirrorName(gHeader->mirrorMode) : @"auto",
        @"width": @(gWidth),
        @"height": @(gHeight),
        @"processId": @((int)getpid()),
        @"sequence": gHeader ? @(gHeader->sequence) : @0,
        @"frames": @(atomic_load(&gPublishedFrames)),
        @"droppedFrames": @(atomic_load(&gDroppedFrames)),
    } mutableCopy];
    if (gSourceArgument.length > 0) payload[@"arg"] = gSourceArgument;
    if (gHeader) payload[@"sourceLabel"] = StringFromCString(gHeader->sourceLabel);
    if (gLastPixelFormat != 0) payload[@"pixelFormat"] = FourCCString(gLastPixelFormat);
    payload[@"cameraAuthorization"] = AuthorizationStatusName([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]);
    if (gSourceKind == SIMDECK_CAMERA_SOURCE_WEBCAM) {
        payload[@"webcamRunning"] = @(gWebcamSession.isRunning);
    }
    if (error.length > 0) payload[@"error"] = error;
    return payload;
}

static int OpenSharedMemory(void) {
    if (!gShmName) return -1;
    shm_unlink(gShmName);
    gMappedSize = (size_t)SimDeckCameraBufferSize(gWidth, gHeight);
    int fd = shm_open(gShmName, O_CREAT | O_RDWR, 0644);
    if (fd < 0) {
        perror("shm_open");
        return -1;
    }
    if (ftruncate(fd, (off_t)gMappedSize) != 0) {
        perror("ftruncate");
        close(fd);
        return -1;
    }
    void *mapped = mmap(NULL, gMappedSize, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (mapped == MAP_FAILED) {
        perror("mmap");
        return -1;
    }
    gHeader = (SimDeckCameraHeader *)mapped;
    memset(gHeader, 0, SIMDECK_CAMERA_HEADER_SIZE);
    gHeader->magic = SIMDECK_CAMERA_MAGIC;
    gHeader->version = SIMDECK_CAMERA_VERSION;
    gHeader->headerSize = SIMDECK_CAMERA_HEADER_SIZE;
    gHeader->width = gWidth;
    gHeader->height = gHeight;
    gHeader->bytesPerRow = gWidth * 4;
    gHeader->pixelFormat = kCVPixelFormatType_32BGRA;
    gHeader->mirrorMode = SIMDECK_CAMERA_MIRROR_AUTO;
    gPixels = ((uint8_t *)mapped) + SIMDECK_CAMERA_HEADER_SIZE;
    return 0;
}

static NSDictionary *WebcamsPayload(void) {
    NSMutableArray *items = [NSMutableArray array];
    for (AVCaptureDevice *device in CameraDevices()) {
        [items addObject:@{
            @"id": device.uniqueID ?: device.localizedName ?: @"",
            @"name": device.localizedName ?: device.uniqueID ?: @"Camera",
            @"position": device.position == AVCaptureDevicePositionFront ? @"front" :
                device.position == AVCaptureDevicePositionBack ? @"back" : @"unspecified",
        }];
    }
    return @{ @"webcams": items };
}

static void Cleanup(void) {
    StopCurrentSource();
    if (gHeader) {
        munmap(gHeader, gMappedSize);
        gHeader = NULL;
    }
    if (gShmName) {
        shm_unlink(gShmName);
        free(gShmName);
        gShmName = NULL;
    }
    gPixels = NULL;
    gMappedSize = 0;
    gSourceName = nil;
    gSourceArgument = nil;
    gSourceKind = SIMDECK_CAMERA_SOURCE_PLACEHOLDER;
    gActiveUDID = nil;
    gServiceStarted = NO;
}

static void SignalHandler(int signalNumber) {
    (void)signalNumber;
    Cleanup();
    _exit(0);
}

static char *CopyCString(NSString *value) {
    const char *utf8 = value.UTF8String ?: "";
    char *copy = strdup(utf8);
    return copy ?: strdup("");
}

static void SetNativeError(char **errorMessage, NSString *message) {
    if (errorMessage) {
        *errorMessage = CopyCString(message ?: @"Unknown camera error.");
    }
}

static char *JSONCString(NSDictionary *payload) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:payload ?: @{} options:0 error:nil];
    if (!data) {
        return CopyCString(@"{}");
    }
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] ?: @"{}";
    return CopyCString(json);
}

char *simdeck_camera_list_webcams_json(char **errorMessage) {
    (void)errorMessage;
    __block char *result = NULL;
    RunOnMainSync(^{
        @autoreleasepool {
            result = JSONCString(WebcamsPayload());
        }
    });
    return result;
}

bool simdeck_camera_start(const char *udid,
                          const char *shmName,
                          const char *source,
                          const char *sourceArgument,
                          const char *mirror,
                          char **errorMessage) {
    __block BOOL ok = NO;
    __block NSString *nativeError = nil;
    RunOnMainSync(^{
        @autoreleasepool {
            @synchronized (CameraLock()) {
                Cleanup();
                if (!shmName || shmName[0] != '/') {
                    nativeError = @"Camera shared memory name must start with `/`.";
                    return;
                }
                gActiveUDID = [StringFromCString(udid) copy];
                gShmName = strdup(shmName);
                gWriteQueue = dispatch_queue_create("dev.nativescript.simdeck.camera.writer", DISPATCH_QUEUE_SERIAL);
                atomic_store(&gPublishedFrames, 0);
                atomic_store(&gDroppedFrames, 0);
                gLastPixelFormat = 0;
                [NSApplication sharedApplication];
                [NSApp finishLaunching];
                signal(SIGINT, SignalHandler);
                signal(SIGTERM, SignalHandler);
                if (OpenSharedMemory() != 0) {
                    nativeError = @"Unable to open camera shared memory.";
                    Cleanup();
                    return;
                }
                if (gHeader) {
                    gHeader->mirrorMode = MirrorModeForName(StringFromCString(mirror));
                }
                if (!SwitchSource(StringFromCString(source), StringFromCString(sourceArgument), &nativeError)) {
                    Cleanup();
                    return;
                }
                gServiceStarted = YES;
                ok = YES;
            }
        }
    });
    if (!ok) {
        SetNativeError(errorMessage, nativeError);
    }
    return ok;
}

char *simdeck_camera_status(const char *udid, char **errorMessage) {
    (void)errorMessage;
    __block char *result = NULL;
    RunOnMainSync(^{
        @autoreleasepool {
            @synchronized (CameraLock()) {
                NSString *requestedUDID = StringFromCString(udid);
                if (!gServiceStarted || (requestedUDID.length > 0 && gActiveUDID.length > 0 && ![requestedUDID isEqualToString:gActiveUDID])) {
                    result = JSONCString(@{
                        @"ok": @YES,
                        @"alive": @NO,
                        @"cameraAuthorization": AuthorizationStatusName([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]),
                    });
                    return;
                }
                result = JSONCString(StatusPayload(YES, nil));
            }
        }
    });
    return result;
}

char *simdeck_camera_switch(const char *udid,
                            const char *source,
                            const char *sourceArgument,
                            const char *mirror,
                            char **errorMessage) {
    __block char *result = NULL;
    __block NSString *nativeError = nil;
    RunOnMainSync(^{
        @autoreleasepool {
            @synchronized (CameraLock()) {
                NSString *requestedUDID = StringFromCString(udid);
                if (!gServiceStarted || (requestedUDID.length > 0 && gActiveUDID.length > 0 && ![requestedUDID isEqualToString:gActiveUDID])) {
                    nativeError = @"Camera simulation is not running for this simulator.";
                    return;
                }
                if (mirror && mirror[0] && gHeader) {
                    gHeader->mirrorMode = MirrorModeForName(StringFromCString(mirror));
                }
                if (source && source[0] && !SwitchSource(StringFromCString(source), StringFromCString(sourceArgument), &nativeError)) {
                    return;
                }
                result = JSONCString(StatusPayload(YES, nil));
            }
        }
    });
    if (!result) {
        SetNativeError(errorMessage, nativeError);
    }
    return result;
}

bool simdeck_camera_stop(const char *udid, char **errorMessage) {
    (void)errorMessage;
    __block BOOL stopped = NO;
    RunOnMainSync(^{
        @autoreleasepool {
            @synchronized (CameraLock()) {
                NSString *requestedUDID = StringFromCString(udid);
                if (requestedUDID.length == 0 || gActiveUDID.length == 0 || [requestedUDID isEqualToString:gActiveUDID]) {
                    Cleanup();
                }
                stopped = YES;
            }
        }
    });
    return stopped;
}
