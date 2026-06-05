#import "XCWNativeBridge.h"

#import "DFPrivateSimulatorDisplayBridge.h"
#import "XCWAccessibilityBridge.h"
#import "XCWChromeRenderer.h"
#import "XCWH264Encoder.h"
#import "XCWNativeSession.h"
#import "XCWSimctl.h"

#import <AppKit/AppKit.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreAudio/AudioHardwareTapping.h>
#import <CoreAudio/CATapDescription.h>
#import <CoreFoundation/CoreFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#include <math.h>
#include <stddef.h>
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
        const char *androidCodec = getenv("SIMDECK_ANDROID_VIDEO_CODEC");
        if (androidCodec == NULL || strlen(androidCodec) == 0) {
            androidCodec = "software";
        }
        setenv("SIMDECK_VIDEO_CODEC", androidCodec, 1);
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

- (void)requestKeyFrame {
    [_encoder requestKeyFrame];
}

- (void)invalidate {
    [_encoder invalidate];
}

@end

static NSString *XCWAudioDictionaryKey(const char *key) {
    return [NSString stringWithUTF8String:key] ?: @"";
}

static NSString *XCWAudioOSStatusString(OSStatus status) {
    UInt32 code = CFSwapInt32HostToBig((UInt32)status);
    char text[5] = {0};
    memcpy(text, &code, 4);
    BOOL printable = YES;
    for (NSUInteger index = 0; index < 4; index++) {
        if (text[index] < 32 || text[index] > 126) {
            printable = NO;
            break;
        }
    }
    if (printable) {
        return [NSString stringWithFormat:@"%d ('%s')", (int)status, text];
    }
    return [NSString stringWithFormat:@"%d", (int)status];
}

static NSError *XCWAudioCaptureError(NSInteger code, NSString *description) {
    return [NSError errorWithDomain:@"SimDeck.AudioCapture"
                               code:code
                           userInfo:@{ NSLocalizedDescriptionKey: description ?: @"Audio capture failed." }];
}

static NSError *XCWAudioCaptureStatusError(NSInteger code, NSString *operation, OSStatus status) {
    return XCWAudioCaptureError(code, [NSString stringWithFormat:@"%@ failed with OSStatus %@.", operation, XCWAudioOSStatusString(status)]);
}

static const uint32_t XCWOpusSampleRate = 48000;
static const uint16_t XCWOpusChannels = 2;
static const UInt32 XCWOpusFramesPerPacket = 960;
static const UInt32 XCWOpusBitRate = 96000;
static const UInt32 XCWOpusFallbackMaxPacketBytes = 1500;
static const NSUInteger XCWAudioProcessStableRefreshes = 3;
static const OSStatus XCWAudioConverterNoDataStatus = -1;

static int16_t XCWClampPCM16(double value) {
    if (!isfinite(value)) {
        return 0;
    }
    if (value <= -1.0) {
        return INT16_MIN;
    }
    if (value >= 1.0) {
        return INT16_MAX;
    }
    return (int16_t)lrint(value * 32767.0);
}

static int16_t XCWReadPCM16Sample(const AudioBufferList *bufferList,
                                  const AudioStreamBasicDescription *asbd,
                                  NSUInteger frame,
                                  NSUInteger channel) {
    if (bufferList == NULL || asbd == NULL || bufferList->mNumberBuffers == 0) {
        return 0;
    }

    const UInt32 bitsPerChannel = asbd->mBitsPerChannel;
    const NSUInteger bytesPerSample = MAX((NSUInteger)bitsPerChannel / 8, 1);
    const BOOL nonInterleaved = (asbd->mFormatFlags & kAudioFormatFlagIsNonInterleaved) != 0;
    const BOOL isFloat = (asbd->mFormatFlags & kAudioFormatFlagIsFloat) != 0;
    const BOOL isSigned = (asbd->mFormatFlags & kAudioFormatFlagIsSignedInteger) != 0;
    const BOOL isBigEndian = (asbd->mFormatFlags & kAudioFormatFlagIsBigEndian) != 0;
    const NSUInteger sourceChannels = MAX((NSUInteger)asbd->mChannelsPerFrame, 1);
    const NSUInteger bufferIndex = nonInterleaved
        ? MIN(channel, (NSUInteger)bufferList->mNumberBuffers - 1)
        : 0;
    const NSUInteger channelInBuffer = nonInterleaved ? 0 : MIN(channel, sourceChannels - 1);
    const AudioBuffer buffer = bufferList->mBuffers[bufferIndex];
    if (buffer.mData == NULL || buffer.mDataByteSize == 0) {
        return 0;
    }

    const NSUInteger fallbackBytesPerFrame = bytesPerSample * (nonInterleaved ? 1 : sourceChannels);
    const NSUInteger bytesPerFrame = MAX((NSUInteger)asbd->mBytesPerFrame, fallbackBytesPerFrame);
    const NSUInteger offset = frame * bytesPerFrame + channelInBuffer * bytesPerSample;
    if (offset + bytesPerSample > buffer.mDataByteSize) {
        return 0;
    }

    const uint8_t *sample = (const uint8_t *)buffer.mData + offset;
    if (isFloat && bytesPerSample == sizeof(float)) {
        float value = 0.0f;
        memcpy(&value, sample, sizeof(value));
        return XCWClampPCM16((double)value);
    }
    if (isFloat && bytesPerSample == sizeof(double)) {
        double value = 0.0;
        memcpy(&value, sample, sizeof(value));
        return XCWClampPCM16(value);
    }
    if (bytesPerSample == sizeof(int16_t)) {
        uint16_t raw = 0;
        memcpy(&raw, sample, sizeof(raw));
        if (isBigEndian) {
            raw = CFSwapInt16BigToHost(raw);
        }
        return (int16_t)raw;
    }
    if (bytesPerSample == sizeof(int32_t)) {
        uint32_t raw = 0;
        memcpy(&raw, sample, sizeof(raw));
        if (isBigEndian) {
            raw = CFSwapInt32BigToHost(raw);
        }
        return (int16_t)(((int32_t)raw) >> 16);
    }
    if (bytesPerSample == sizeof(uint8_t)) {
        if (isSigned) {
            return (int16_t)(((int8_t)sample[0]) << 8);
        }
        return (int16_t)(((int)sample[0] - 128) << 8);
    }

    return 0;
}

static NSUInteger XCWAudioFrameCount(const AudioBufferList *bufferList,
                                     const AudioStreamBasicDescription *asbd) {
    if (bufferList == NULL || asbd == NULL || bufferList->mNumberBuffers == 0) {
        return 0;
    }
    const AudioBuffer buffer = bufferList->mBuffers[0];
    if (buffer.mData == NULL || buffer.mDataByteSize == 0) {
        return 0;
    }
    const NSUInteger bytesPerSample = MAX((NSUInteger)asbd->mBitsPerChannel / 8, 1);
    const BOOL nonInterleaved = (asbd->mFormatFlags & kAudioFormatFlagIsNonInterleaved) != 0;
    const NSUInteger sourceChannels = MAX((NSUInteger)asbd->mChannelsPerFrame, 1);
    const NSUInteger fallbackBytesPerFrame = bytesPerSample * (nonInterleaved ? 1 : sourceChannels);
    const NSUInteger bytesPerFrame = MAX((NSUInteger)asbd->mBytesPerFrame, fallbackBytesPerFrame);
    if (bytesPerFrame == 0) {
        return 0;
    }
    return (NSUInteger)buffer.mDataByteSize / bytesPerFrame;
}

static NSData *XCWPCM16InterleavedDataFromAudioBufferList(const AudioBufferList *bufferList,
                                                          const AudioStreamBasicDescription *asbd,
                                                          uint32_t *sampleRate,
                                                          uint16_t *channels) {
    if (bufferList == NULL || asbd == NULL || asbd->mFormatID != kAudioFormatLinearPCM) {
        return nil;
    }
    const NSUInteger frameCount = XCWAudioFrameCount(bufferList, asbd);
    const NSUInteger sourceChannels = MAX((NSUInteger)asbd->mChannelsPerFrame, 1);
    const NSUInteger outputChannels = MIN(sourceChannels, (NSUInteger)2);
    if (frameCount == 0 || outputChannels == 0) {
        return nil;
    }

    NSMutableData *output = [NSMutableData dataWithLength:frameCount * outputChannels * sizeof(int16_t)];
    int16_t *outputSamples = (int16_t *)output.mutableBytes;
    for (NSUInteger frame = 0; frame < frameCount; frame++) {
        for (NSUInteger channel = 0; channel < outputChannels; channel++) {
            outputSamples[frame * outputChannels + channel] = XCWReadPCM16Sample(bufferList, asbd, frame, channel);
        }
    }

    if (sampleRate != NULL) {
        *sampleRate = (uint32_t)llround(asbd->mSampleRate > 0 ? asbd->mSampleRate : 48000.0);
    }
    if (channels != NULL) {
        *channels = (uint16_t)outputChannels;
    }
    return output;
}

static uint64_t XCWAudioTimestampUS(const AudioTimeStamp *timeStamp) {
    if (timeStamp != NULL && (timeStamp->mFlags & kAudioTimeStampHostTimeValid) != 0 && timeStamp->mHostTime != 0) {
        return AudioConvertHostTimeToNanos(timeStamp->mHostTime) / 1000;
    }
    return (uint64_t)llround([[NSDate date] timeIntervalSince1970] * 1000000.0);
}

static AudioObjectID XCWAudioProcessObjectIDForPID(pid_t pid) {
    if (pid <= 0) {
        return kAudioObjectUnknown;
    }
    AudioObjectPropertyAddress address = {
        .mSelector = kAudioHardwarePropertyTranslatePIDToProcessObject,
        .mScope = kAudioObjectPropertyScopeGlobal,
        .mElement = kAudioObjectPropertyElementMain,
    };
    AudioObjectID processObjectID = kAudioObjectUnknown;
    UInt32 size = sizeof(processObjectID);
    OSStatus status = AudioObjectGetPropertyData(kAudioObjectSystemObject,
                                                 &address,
                                                 sizeof(pid),
                                                 &pid,
                                                 &size,
                                                 &processObjectID);
    if (status != noErr) {
        return kAudioObjectUnknown;
    }
    return processObjectID;
}

static NSArray<NSNumber *> *XCWAudioProcessObjectIDsForProcessIDs(const int32_t *processIDs,
                                                                  size_t processCount) {
    NSMutableSet<NSNumber *> *seen = [NSMutableSet set];
    NSMutableArray<NSNumber *> *objects = [NSMutableArray array];
    for (size_t index = 0; index < processCount; index++) {
        pid_t pid = (pid_t)processIDs[index];
        if (pid <= 0) {
            continue;
        }
        AudioObjectID objectID = XCWAudioProcessObjectIDForPID(pid);
        if (objectID == kAudioObjectUnknown) {
            continue;
        }
        NSNumber *boxed = @(objectID);
        if ([seen containsObject:boxed]) {
            continue;
        }
        [seen addObject:boxed];
        [objects addObject:boxed];
    }
    [objects sortUsingSelector:@selector(compare:)];
    return objects;
}

static CATapDescription *XCWAudioTapDescription(NSArray<NSNumber *> *processObjectIDs) API_AVAILABLE(macos(14.2)) {
    CATapDescription *description = [[CATapDescription alloc] initStereoMixdownOfProcesses:processObjectIDs];
    description.name = @"SimDeck Simulator Audio";
    description.privateTap = YES;
    description.muteBehavior = CATapMutedWhenTapped;
    description.mixdown = YES;
    description.mono = NO;
    description.exclusive = NO;
    return description;
}

static NSString *XCWAudioTapUID(AudioObjectID tapID, NSError * _Nullable __autoreleasing *error) {
    AudioObjectPropertyAddress address = {
        .mSelector = kAudioTapPropertyUID,
        .mScope = kAudioObjectPropertyScopeGlobal,
        .mElement = kAudioObjectPropertyElementMain,
    };
    CFStringRef uid = NULL;
    UInt32 size = sizeof(uid);
    OSStatus status = AudioObjectGetPropertyData(tapID, &address, 0, NULL, &size, &uid);
    if (status != noErr || uid == NULL) {
        if (error != NULL) {
            *error = XCWAudioCaptureStatusError(22, @"Read Core Audio tap UID", status);
        }
        return nil;
    }
    return CFBridgingRelease(uid);
}

static BOOL XCWAudioGetObjectStreamFormat(AudioObjectID objectID,
                                          AudioObjectPropertySelector selector,
                                          AudioObjectPropertyScope scope,
                                          AudioStreamBasicDescription *asbd) {
    if (asbd == NULL || objectID == kAudioObjectUnknown) {
        return NO;
    }
    AudioObjectPropertyAddress address = {
        .mSelector = selector,
        .mScope = scope,
        .mElement = kAudioObjectPropertyElementMain,
    };
    UInt32 size = sizeof(*asbd);
    OSStatus status = AudioObjectGetPropertyData(objectID, &address, 0, NULL, &size, asbd);
    return status == noErr && asbd->mSampleRate > 0 && asbd->mChannelsPerFrame > 0;
}

typedef struct XCWOpusInputContext {
    const uint8_t *bytes;
    UInt32 byteCount;
    UInt32 bytesPerFrame;
    UInt16 channels;
    UInt32 consumedBytes;
} XCWOpusInputContext;

static OSStatus XCWOpusEncoderInputProc(AudioConverterRef inAudioConverter,
                                        UInt32 *ioNumberDataPackets,
                                        AudioBufferList *ioData,
                                        AudioStreamPacketDescription **outDataPacketDescription,
                                        void *inUserData) {
    (void)inAudioConverter;
    if (outDataPacketDescription != NULL) {
        *outDataPacketDescription = NULL;
    }
    if (ioNumberDataPackets == NULL || ioData == NULL || inUserData == NULL) {
        return paramErr;
    }
    XCWOpusInputContext *context = (XCWOpusInputContext *)inUserData;
    if (context->bytes == NULL || context->bytesPerFrame == 0 || context->consumedBytes >= context->byteCount) {
        *ioNumberDataPackets = 0;
        return XCWAudioConverterNoDataStatus;
    }

    UInt32 availableBytes = context->byteCount - context->consumedBytes;
    UInt32 availablePackets = availableBytes / context->bytesPerFrame;
    UInt32 packets = MIN(*ioNumberDataPackets, availablePackets);
    if (packets == 0) {
        *ioNumberDataPackets = 0;
        return XCWAudioConverterNoDataStatus;
    }

    UInt32 bytes = packets * context->bytesPerFrame;
    ioData->mNumberBuffers = 1;
    ioData->mBuffers[0].mNumberChannels = context->channels;
    ioData->mBuffers[0].mDataByteSize = bytes;
    ioData->mBuffers[0].mData = (void *)(context->bytes + context->consumedBytes);
    context->consumedBytes += bytes;
    *ioNumberDataPackets = packets;
    return noErr;
}

@interface XCWOpusAudioEncoder : NSObject

@property (nonatomic, readonly) uint16_t channels;

- (NSArray<NSData *> *)encodePCM:(NSData *)pcm
                      sampleRate:(uint32_t)sampleRate
                        channels:(uint16_t)channels
                           error:(NSError * _Nullable __autoreleasing *)error;
- (void)invalidate;

@end

@implementation XCWOpusAudioEncoder {
    AudioConverterRef _converter;
    NSMutableData *_pendingPCM;
    uint32_t _inputSampleRate;
    uint16_t _inputChannels;
    UInt32 _inputBytesPerFrame;
    UInt32 _maxOutputPacketSize;
    NSUInteger _inputFramesPerOpusPacket;
}

- (instancetype)init {
    self = [super init];
    if (self == nil) {
        return nil;
    }
    _pendingPCM = [NSMutableData data];
    _channels = XCWOpusChannels;
    return self;
}

- (void)dealloc {
    [self invalidate];
}

- (NSArray<NSData *> *)encodePCM:(NSData *)pcm
                      sampleRate:(uint32_t)sampleRate
                        channels:(uint16_t)channels
                           error:(NSError * _Nullable __autoreleasing *)error {
    if (pcm.length == 0 || sampleRate == 0 || channels == 0) {
        return @[];
    }
    if (_converter == NULL || _inputSampleRate != sampleRate || _inputChannels != channels) {
        [self invalidate];
        if (![self configureWithSampleRate:sampleRate channels:channels error:error]) {
            return @[];
        }
    }

    [_pendingPCM appendData:pcm];
    NSMutableArray<NSData *> *packets = [NSMutableArray array];
    while ([self pendingFrameCount] >= _inputFramesPerOpusPacket) {
        NSData *packet = [self encodeNextPacket:error];
        if (packet == nil) {
            break;
        }
        if (packet.length > 0) {
            [packets addObject:packet];
        }
    }
    return packets;
}

- (BOOL)configureWithSampleRate:(uint32_t)sampleRate
                       channels:(uint16_t)channels
                          error:(NSError * _Nullable __autoreleasing *)error {
    _inputSampleRate = sampleRate;
    _inputChannels = channels;
    _inputBytesPerFrame = MAX((UInt32)channels * (UInt32)sizeof(int16_t), 1);
    _inputFramesPerOpusPacket = MAX((NSUInteger)llround(((double)sampleRate * (double)XCWOpusFramesPerPacket) / (double)XCWOpusSampleRate), (NSUInteger)1);

    AudioStreamBasicDescription input = {0};
    input.mSampleRate = sampleRate;
    input.mFormatID = kAudioFormatLinearPCM;
    input.mFormatFlags = kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked;
    input.mBytesPerPacket = _inputBytesPerFrame;
    input.mFramesPerPacket = 1;
    input.mBytesPerFrame = _inputBytesPerFrame;
    input.mChannelsPerFrame = channels;
    input.mBitsPerChannel = 16;

    AudioStreamBasicDescription output = {0};
    output.mSampleRate = XCWOpusSampleRate;
    output.mFormatID = kAudioFormatOpus;
    output.mChannelsPerFrame = XCWOpusChannels;
    output.mFramesPerPacket = XCWOpusFramesPerPacket;

    OSStatus status = AudioConverterNew(&input, &output, &_converter);
    if (status != noErr || _converter == NULL) {
        if (error != NULL) {
            *error = XCWAudioCaptureStatusError(31, @"Create Core Audio Opus encoder", status);
        }
        _converter = NULL;
        return NO;
    }

    UInt32 bitRate = XCWOpusBitRate;
    (void)AudioConverterSetProperty(_converter, kAudioConverterEncodeBitRate, sizeof(bitRate), &bitRate);

    _maxOutputPacketSize = 0;
    UInt32 propertySize = sizeof(_maxOutputPacketSize);
    status = AudioConverterGetProperty(_converter,
                                       kAudioConverterPropertyMaximumOutputPacketSize,
                                       &propertySize,
                                       &_maxOutputPacketSize);
    if (status != noErr || _maxOutputPacketSize == 0) {
        _maxOutputPacketSize = XCWOpusFallbackMaxPacketBytes;
    }
    _maxOutputPacketSize = MIN(MAX(_maxOutputPacketSize, (UInt32)256), (UInt32)4096);
    [_pendingPCM setLength:0];
    return YES;
}

- (NSUInteger)pendingFrameCount {
    if (_inputBytesPerFrame == 0) {
        return 0;
    }
    return _pendingPCM.length / _inputBytesPerFrame;
}

- (NSData *)encodeNextPacket:(NSError * _Nullable __autoreleasing *)error {
    if (_converter == NULL || _inputBytesPerFrame == 0 || _inputFramesPerOpusPacket == 0) {
        return nil;
    }
    const NSUInteger inputBytes = MIN(_pendingPCM.length, _inputFramesPerOpusPacket * (NSUInteger)_inputBytesPerFrame);
    if (inputBytes == 0 || inputBytes > UINT32_MAX) {
        return nil;
    }

    XCWOpusInputContext context = {
        .bytes = (const uint8_t *)_pendingPCM.bytes,
        .byteCount = (UInt32)inputBytes,
        .bytesPerFrame = _inputBytesPerFrame,
        .channels = _inputChannels,
        .consumedBytes = 0,
    };
    NSMutableData *output = [NSMutableData dataWithLength:_maxOutputPacketSize];
    AudioBufferList outputBuffer = {
        .mNumberBuffers = 1,
        .mBuffers = {
            {
                .mNumberChannels = XCWOpusChannels,
                .mDataByteSize = _maxOutputPacketSize,
                .mData = output.mutableBytes,
            },
        },
    };
    UInt32 outputPackets = 1;
    AudioStreamPacketDescription packetDescription = {0};
    OSStatus status = AudioConverterFillComplexBuffer(_converter,
                                                      XCWOpusEncoderInputProc,
                                                      &context,
                                                      &outputPackets,
                                                      &outputBuffer,
                                                      &packetDescription);
    if (context.consumedBytes > 0 && context.consumedBytes <= _pendingPCM.length) {
        [_pendingPCM replaceBytesInRange:NSMakeRange(0, context.consumedBytes)
                               withBytes:NULL
                                  length:0];
    }
    if (status == XCWAudioConverterNoDataStatus || outputPackets == 0 || outputBuffer.mBuffers[0].mDataByteSize == 0) {
        if (context.consumedBytes == 0) {
            return nil;
        }
        return [NSData data];
    }
    if (status != noErr) {
        if (error != NULL) {
            *error = XCWAudioCaptureStatusError(32, @"Encode Opus audio packet", status);
        }
        return nil;
    }
    output.length = outputBuffer.mBuffers[0].mDataByteSize;
    return output;
}

- (void)invalidate {
    if (_converter != NULL) {
        AudioConverterDispose(_converter);
        _converter = NULL;
    }
    [_pendingPCM setLength:0];
    _inputSampleRate = 0;
    _inputChannels = 0;
    _inputBytesPerFrame = 0;
    _maxOutputPacketSize = 0;
    _inputFramesPerOpusPacket = 0;
}

@end

@class XCWNativeAudioCapture;
static OSStatus XCWNativeAudioDeviceIOProc(AudioObjectID inDevice,
                                           const AudioTimeStamp *inNow,
                                           const AudioBufferList *inInputData,
                                           const AudioTimeStamp *inInputTime,
                                           AudioBufferList *outOutputData,
                                           const AudioTimeStamp *inOutputTime,
                                           void *inClientData);

@interface XCWNativeAudioCapture : NSObject

- (instancetype)initWithAudioCallback:(xcw_native_audio_callback)callback
                             userData:(void *)userData;
- (BOOL)startWithProcessIDs:(const int32_t *)processIDs
                      count:(size_t)processCount
                      error:(NSError * _Nullable __autoreleasing *)error;
- (BOOL)updateProcessIDs:(const int32_t *)processIDs
                   count:(size_t)processCount
                   error:(NSError * _Nullable __autoreleasing *)error;
- (BOOL)applyProcessIDs:(const int32_t *)processIDs
                  count:(size_t)processCount
       requireProcesses:(BOOL)requireProcesses
        debounceChanges:(BOOL)debounceChanges
                  error:(NSError * _Nullable __autoreleasing *)error;
- (BOOL)shouldApplyProcessObjectIDs:(NSArray<NSNumber *> *)processObjectIDs;
- (void)clearPendingProcessObjectIDs;
- (void)invalidate;
- (void)handleInputData:(const AudioBufferList *)inputData
              inputTime:(const AudioTimeStamp *)inputTime;

@end

@implementation XCWNativeAudioCapture {
    xcw_native_audio_callback _callback;
    void *_callbackUserData;
    BOOL _invalidated;
    AudioObjectID _tapID;
    AudioObjectID _aggregateDeviceID;
    AudioDeviceIOProcID _ioProcID;
    AudioStreamBasicDescription _streamDescription;
    NSArray<NSNumber *> *_processObjectIDs;
    NSArray<NSNumber *> *_pendingProcessObjectIDs;
    NSUInteger _pendingProcessObjectIDRefreshes;
    XCWOpusAudioEncoder *_opusEncoder;
}

- (instancetype)initWithAudioCallback:(xcw_native_audio_callback)callback
                             userData:(void *)userData {
    self = [super init];
    if (self == nil) {
        return nil;
    }
    _callback = callback;
    _callbackUserData = userData;
    _tapID = kAudioObjectUnknown;
    _aggregateDeviceID = kAudioObjectUnknown;
    _ioProcID = NULL;
    _processObjectIDs = @[];
    _pendingProcessObjectIDs = nil;
    _pendingProcessObjectIDRefreshes = 0;
    _opusEncoder = [[XCWOpusAudioEncoder alloc] init];
    return self;
}

- (void)dealloc {
    [self invalidate];
}

- (BOOL)startWithProcessIDs:(const int32_t *)processIDs
                      count:(size_t)processCount
                      error:(NSError * _Nullable __autoreleasing *)error {
    return [self applyProcessIDs:processIDs count:processCount requireProcesses:YES debounceChanges:NO error:error];
}

- (BOOL)updateProcessIDs:(const int32_t *)processIDs
                   count:(size_t)processCount
                   error:(NSError * _Nullable __autoreleasing *)error {
    return [self applyProcessIDs:processIDs count:processCount requireProcesses:NO debounceChanges:YES error:error];
}

- (BOOL)applyProcessIDs:(const int32_t *)processIDs
                  count:(size_t)processCount
       requireProcesses:(BOOL)requireProcesses
        debounceChanges:(BOOL)debounceChanges
                  error:(NSError * _Nullable __autoreleasing *)error {
    if (@available(macOS 14.2, *)) {
        NSArray<NSNumber *> *processObjectIDs = XCWAudioProcessObjectIDsForProcessIDs(processIDs, processCount);
        if (_aggregateDeviceID != kAudioObjectUnknown && [_processObjectIDs isEqualToArray:processObjectIDs]) {
            [self clearPendingProcessObjectIDs];
            return YES;
        }
        if (debounceChanges && _aggregateDeviceID != kAudioObjectUnknown && ![self shouldApplyProcessObjectIDs:processObjectIDs]) {
            return YES;
        }
        if (processObjectIDs.count == 0) {
            [self clearPendingProcessObjectIDs];
            [self stopGraph];
            if (requireProcesses && error != NULL) {
                *error = XCWAudioCaptureError(20, @"No simulator audio processes are currently connected to Core Audio.");
            }
            return !requireProcesses;
        }
        [self stopGraph];
        return [self startGraphWithProcessObjectIDs:processObjectIDs error:error];
    }

    if (error != NULL) {
        *error = XCWAudioCaptureError(21, @"Per-simulator audio capture requires macOS 14.2 or newer.");
    }
    return NO;
}

- (BOOL)shouldApplyProcessObjectIDs:(NSArray<NSNumber *> *)processObjectIDs {
    if (_pendingProcessObjectIDs != nil && [_pendingProcessObjectIDs isEqualToArray:processObjectIDs]) {
        _pendingProcessObjectIDRefreshes += 1;
    } else {
        _pendingProcessObjectIDs = [processObjectIDs copy];
        _pendingProcessObjectIDRefreshes = 1;
    }
    return _pendingProcessObjectIDRefreshes >= XCWAudioProcessStableRefreshes;
}

- (void)clearPendingProcessObjectIDs {
    _pendingProcessObjectIDs = nil;
    _pendingProcessObjectIDRefreshes = 0;
}

- (BOOL)startGraphWithProcessObjectIDs:(NSArray<NSNumber *> *)processObjectIDs
                                 error:(NSError * _Nullable __autoreleasing *)error API_AVAILABLE(macos(14.2)) {
    CATapDescription *tapDescription = XCWAudioTapDescription(processObjectIDs);
    OSStatus status = AudioHardwareCreateProcessTap(tapDescription, &_tapID);
    if (status != noErr || _tapID == kAudioObjectUnknown) {
        if (error != NULL) {
            *error = XCWAudioCaptureStatusError(23, @"Create Core Audio process tap", status);
        }
        _tapID = kAudioObjectUnknown;
        return NO;
    }

    NSError *tapUIDError = nil;
    NSString *tapUID = XCWAudioTapUID(_tapID, &tapUIDError);
    if (tapUID.length == 0) {
        if (error != NULL) {
            *error = tapUIDError ?: XCWAudioCaptureError(24, @"Core Audio process tap did not expose a UID.");
        }
        [self stopGraph];
        return NO;
    }

    NSString *aggregateUID = [NSString stringWithFormat:@"dev.simdeck.audio.%@", NSUUID.UUID.UUIDString];
    NSDictionary *aggregateDescription = @{
        XCWAudioDictionaryKey(kAudioAggregateDeviceNameKey): @"SimDeck Simulator Audio",
        XCWAudioDictionaryKey(kAudioAggregateDeviceUIDKey): aggregateUID,
        XCWAudioDictionaryKey(kAudioAggregateDeviceIsPrivateKey): @YES,
        XCWAudioDictionaryKey(kAudioAggregateDeviceTapListKey): @[
            @{ XCWAudioDictionaryKey(kAudioSubTapUIDKey): tapUID }
        ],
    };
    status = AudioHardwareCreateAggregateDevice((__bridge CFDictionaryRef)aggregateDescription, &_aggregateDeviceID);
    if (status != noErr || _aggregateDeviceID == kAudioObjectUnknown) {
        if (error != NULL) {
            *error = XCWAudioCaptureStatusError(25, @"Create Core Audio aggregate device", status);
        }
        [self stopGraph];
        return NO;
    }

    CFArrayRef tapList = (__bridge CFArrayRef)@[ tapUID ];
    AudioObjectPropertyAddress tapListAddress = {
        .mSelector = kAudioAggregateDevicePropertyTapList,
        .mScope = kAudioObjectPropertyScopeGlobal,
        .mElement = kAudioObjectPropertyElementMain,
    };
    status = AudioObjectSetPropertyData(_aggregateDeviceID,
                                        &tapListAddress,
                                        0,
                                        NULL,
                                        sizeof(tapList),
                                        &tapList);
    if (status != noErr) {
        if (error != NULL) {
            *error = XCWAudioCaptureStatusError(26, @"Attach Core Audio tap to aggregate device", status);
        }
        [self stopGraph];
        return NO;
    }

    memset(&_streamDescription, 0, sizeof(_streamDescription));
    if (!XCWAudioGetObjectStreamFormat(_aggregateDeviceID, kAudioDevicePropertyStreamFormat, kAudioObjectPropertyScopeInput, &_streamDescription) &&
        !XCWAudioGetObjectStreamFormat(_tapID, kAudioTapPropertyFormat, kAudioObjectPropertyScopeGlobal, &_streamDescription)) {
        if (error != NULL) {
            *error = XCWAudioCaptureError(27, @"Core Audio tap did not expose a readable linear PCM format.");
        }
        [self stopGraph];
        return NO;
    }

    status = AudioDeviceCreateIOProcID(_aggregateDeviceID,
                                       XCWNativeAudioDeviceIOProc,
                                       (__bridge void *)self,
                                       &_ioProcID);
    if (status != noErr || _ioProcID == NULL) {
        if (error != NULL) {
            *error = XCWAudioCaptureStatusError(28, @"Create Core Audio tap IOProc", status);
        }
        [self stopGraph];
        return NO;
    }

    status = AudioDeviceStart(_aggregateDeviceID, _ioProcID);
    if (status != noErr) {
        if (error != NULL) {
            *error = XCWAudioCaptureStatusError(29, @"Start Core Audio tap device", status);
        }
        [self stopGraph];
        return NO;
    }

    _processObjectIDs = [processObjectIDs copy];
    [self clearPendingProcessObjectIDs];
    return YES;
}

- (void)stopGraph {
    if (_aggregateDeviceID != kAudioObjectUnknown && _ioProcID != NULL) {
        AudioDeviceStop(_aggregateDeviceID, _ioProcID);
        AudioDeviceDestroyIOProcID(_aggregateDeviceID, _ioProcID);
        _ioProcID = NULL;
    }
    if (_aggregateDeviceID != kAudioObjectUnknown) {
        AudioHardwareDestroyAggregateDevice(_aggregateDeviceID);
        _aggregateDeviceID = kAudioObjectUnknown;
    }
    if (_tapID != kAudioObjectUnknown) {
        AudioHardwareDestroyProcessTap(_tapID);
        _tapID = kAudioObjectUnknown;
    }
    _processObjectIDs = @[];
    [self clearPendingProcessObjectIDs];
    memset(&_streamDescription, 0, sizeof(_streamDescription));
    [_opusEncoder invalidate];
}

- (void)invalidate {
    _invalidated = YES;
    [self stopGraph];
}

- (void)handleInputData:(const AudioBufferList *)inputData
              inputTime:(const AudioTimeStamp *)inputTime {
    if (_invalidated || _callback == NULL || inputData == NULL) {
        return;
    }

    AudioStreamBasicDescription streamDescription = _streamDescription;
    uint32_t sampleRate = 0;
    uint16_t channels = 0;
    NSData *pcm = XCWPCM16InterleavedDataFromAudioBufferList(inputData, &streamDescription, &sampleRate, &channels);
    if (pcm.length == 0 || sampleRate == 0 || channels == 0) {
        return;
    }

    NSError *encodeError = nil;
    NSArray<NSData *> *packets = [_opusEncoder encodePCM:pcm
                                              sampleRate:sampleRate
                                                channels:channels
                                                   error:&encodeError];
    if (encodeError != nil) {
        NSLog(@"SimDeck audio capture failed to encode Opus packet: %@", encodeError.localizedDescription);
        return;
    }

    uint64_t timestampUS = XCWAudioTimestampUS(inputTime);
    for (NSData *packet in packets) {
        if (packet.length == 0) {
            continue;
        }
        xcw_native_audio_sample sample = {
            .timestamp_us = timestampUS,
            .sample_rate = XCWOpusSampleRate,
            .channels = _opusEncoder.channels,
            .data = XCWSharedBytesFromData(packet),
        };
        _callback(&sample, _callbackUserData);
    }
}

@end

static OSStatus XCWNativeAudioDeviceIOProc(AudioObjectID inDevice,
                                           const AudioTimeStamp *inNow,
                                           const AudioBufferList *inInputData,
                                           const AudioTimeStamp *inInputTime,
                                           AudioBufferList *outOutputData,
                                           const AudioTimeStamp *inOutputTime,
                                           void *inClientData) {
    (void)inDevice;
    (void)inNow;
    (void)outOutputData;
    (void)inOutputTime;
    @autoreleasepool {
        XCWNativeAudioCapture *capture = (__bridge XCWNativeAudioCapture *)inClientData;
        [capture handleInputData:inInputData inputTime:inInputTime];
    }
    return noErr;
}

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

void xcw_native_h264_encoder_request_keyframe(void *handle) {
    @autoreleasepool {
        [XCWNativeH264EncoderFromHandle(handle) requestKeyFrame];
    }
}

void *xcw_native_audio_capture_create(const int32_t *process_ids, size_t process_count, xcw_native_audio_callback callback, void *user_data, char **error_message) {
    @autoreleasepool {
        XCWNativeAudioCapture *capture = [[XCWNativeAudioCapture alloc] initWithAudioCallback:callback
                                                                                    userData:user_data];
        NSError *error = nil;
        BOOL ok = [capture startWithProcessIDs:process_ids count:process_count error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
            return NULL;
        }
        return (__bridge_retained void *)capture;
    }
}

bool xcw_native_audio_capture_update_processes(void *handle, const int32_t *process_ids, size_t process_count, char **error_message) {
    if (handle == NULL) {
        XCWSetErrorMessage(error_message, XCWAudioCaptureError(30, @"Audio capture handle is null."));
        return false;
    }
    @autoreleasepool {
        XCWNativeAudioCapture *capture = (__bridge XCWNativeAudioCapture *)handle;
        NSError *error = nil;
        BOOL ok = [capture updateProcessIDs:process_ids count:process_count error:&error];
        if (!ok) {
            XCWSetErrorMessage(error_message, error);
        }
        return ok;
    }
}

void xcw_native_audio_capture_destroy(void *handle) {
    if (handle == NULL) {
        return;
    }
    @autoreleasepool {
        XCWNativeAudioCapture *capture = CFBridgingRelease(handle);
        [capture invalidate];
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
