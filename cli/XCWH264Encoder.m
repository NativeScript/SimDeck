#import "XCWH264Encoder.h"

#import <Accelerate/Accelerate.h>
#import <CoreMedia/CoreMedia.h>
#import <os/lock.h>
#import <QuartzCore/QuartzCore.h>
#import <VideoToolbox/VideoToolbox.h>
#import <x264.h>
#include <stdlib.h>
#include <string.h>

static const int32_t XCWMaximumEncodedDimension = 4096;
static const int32_t XCWMaximumRealtimeHardwareEncodedDimension = 1440;
static const int32_t XCWMaximumSoftwareEncodedDimension = 1600;
static const int32_t XCWMaximumLowLatencySoftwareEncodedDimension = 1170;
static const int32_t XCWTargetRealTimeFrameRate = 120;
static const int32_t XCWTargetRealtimeHardwareFrameRate = 30;
static const int32_t XCWTargetSoftwareFrameRate = 120;
static const int32_t XCWMinimumLocalStreamFrameRate = 15;
static const int32_t XCWMaximumLocalStreamFrameRate = 240;
static const int32_t XCWTargetLowLatencySoftwareFrameRate = 15;
static const NSUInteger XCWMaximumInFlightFrames = 2;
static const int32_t XCWMinimumAverageBitRate = 18000000;
static const int32_t XCWMinimumRealtimeAverageBitRate = 3000000;
static const int32_t XCWMinimumSoftwareAverageBitRate = 3000000;
static const int32_t XCWMinimumLowLatencySoftwareAverageBitRate = 2000000;
static const int64_t XCWBitsPerPixelBudget = 10;
static const int64_t XCWRealtimeBitsPerPixelBudget = 4;
static const int64_t XCWSoftwareBitsPerPixelBudget = 6;
static const int64_t XCWLowLatencySoftwareBitsPerPixelBudget = 3;
static const uint64_t XCWSoftwareMinimumFrameIntervalUs = 16667;
static const uint64_t XCWSoftwareInitialFrameIntervalUs = 16667;
static const uint64_t XCWSoftwareMaximumFrameIntervalUs = 50000;
static const uint64_t XCWSoftwareFrameIntervalStepUs = 5556;
static const NSUInteger XCWSoftwareHealthyFrameWindow = 4;
static const uint64_t XCWLowLatencySoftwareMinimumFrameIntervalUs = 66667;
static const uint64_t XCWLowLatencySoftwareInitialFrameIntervalUs = 66667;
static const uint64_t XCWLowLatencySoftwareMaximumFrameIntervalUs = 133333;
static const uint64_t XCWLowLatencySoftwareFrameIntervalStepUs = 11111;
static const NSUInteger XCWLowLatencySoftwareHealthyFrameWindow = 8;
static const NSUInteger XCWMaximumRealtimeInFlightFrames = 3;
static const int32_t XCWRealtimeKeyFrameIntervalSeconds = 60;
static const double XCWEncoderLatencyEWMAAlpha = 0.2;
static const double XCWEncoderStrainedLoadPercent = 85.0;
static const double XCWEncoderOverloadedLoadPercent = 105.0;
static const NSUInteger XCWEncoderConsecutiveOverBudgetFrameThreshold = 3;
static const double XCWHardwareFallbackLoadPercent = 500.0;
static const NSUInteger XCWHardwareFallbackConsecutiveOverBudgetFrameThreshold = 60;
static const uint64_t XCWAutoHardwareRetryIntervalUs = 10000000;
static void *XCWH264EncoderQueueSpecificKey = &XCWH264EncoderQueueSpecificKey;

typedef NS_ENUM(NSUInteger, XCWVideoEncoderMode) {
    XCWVideoEncoderModeAuto,
    XCWVideoEncoderModeH264Hardware,
    XCWVideoEncoderModeH264Software,
};

static XCWVideoEncoderMode XCWVideoEncoderModeFromEnvironment(void) {
    const char *rawValue = getenv("SIMDECK_VIDEO_CODEC");
    NSString *value = rawValue != NULL ? [[[NSString alloc] initWithUTF8String:rawValue] lowercaseString] : @"";
    if (value.length == 0 || [value isEqualToString:@"auto"]) {
        return XCWVideoEncoderModeAuto;
    }
    if ([value isEqualToString:@"hardware"]) {
        return XCWVideoEncoderModeH264Hardware;
    }
    if ([value isEqualToString:@"software"]) {
        return XCWVideoEncoderModeH264Software;
    }
    return XCWVideoEncoderModeAuto;
}

static BOOL XCWLowLatencyModeFromEnvironment(void) {
    const char *rawValue = getenv("SIMDECK_LOW_LATENCY");
    NSString *value = rawValue != NULL ? [[[NSString alloc] initWithUTF8String:rawValue] lowercaseString] : @"";
    return [value isEqualToString:@"1"] ||
        [value isEqualToString:@"true"] ||
        [value isEqualToString:@"yes"] ||
        [value isEqualToString:@"on"];
}

static BOOL XCWRealtimeStreamModeFromEnvironment(void) {
    const char *rawValue = getenv("SIMDECK_REALTIME_STREAM");
    NSString *value = rawValue != NULL ? [[[NSString alloc] initWithUTF8String:rawValue] lowercaseString] : @"";
    return [value isEqualToString:@"1"] ||
        [value isEqualToString:@"true"] ||
        [value isEqualToString:@"yes"] ||
        [value isEqualToString:@"on"];
}

static int32_t XCWIntFromEnvironment(NSString *name, int32_t fallback, int32_t minimum, int32_t maximum) {
    const char *rawValue = getenv(name.UTF8String);
    if (rawValue == NULL) {
        return fallback;
    }
    char *end = NULL;
    long parsed = strtol(rawValue, &end, 10);
    if (end == rawValue) {
        return fallback;
    }
    if (parsed < minimum) {
        return minimum;
    }
    if (parsed > maximum) {
        return maximum;
    }
    return (int32_t)parsed;
}

static int64_t XCWInt64FromEnvironment(NSString *name, int64_t fallback, int64_t minimum, int64_t maximum) {
    const char *rawValue = getenv(name.UTF8String);
    if (rawValue == NULL) {
        return fallback;
    }
    char *end = NULL;
    long long parsed = strtoll(rawValue, &end, 10);
    if (end == rawValue) {
        return fallback;
    }
    if (parsed < minimum) {
        return minimum;
    }
    if (parsed > maximum) {
        return maximum;
    }
    return (int64_t)parsed;
}

static int32_t XCWRealtimeMaximumEncodedDimension(void) {
    return XCWIntFromEnvironment(@"SIMDECK_REALTIME_MAX_EDGE",
                                 XCWMaximumRealtimeHardwareEncodedDimension,
                                 720,
                                 XCWMaximumEncodedDimension);
}

static int32_t XCWRealtimeTargetFrameRate(void) {
    return XCWIntFromEnvironment(@"SIMDECK_REALTIME_FPS",
                                 XCWTargetRealtimeHardwareFrameRate,
                                 15,
                                 XCWMaximumLocalStreamFrameRate);
}

static uint64_t XCWRealtimeFrameIntervalUs(void) {
    int32_t fps = MAX(1, XCWRealtimeTargetFrameRate());
    return (uint64_t)llround(1000000.0 / (double)fps);
}

static uint64_t XCWRealtimeMaximumFrameIntervalUs(void) {
    return MAX(XCWRealtimeFrameIntervalUs() * 2, XCWRealtimeFrameIntervalUs());
}

static int32_t XCWLocalStreamTargetFrameRate(void) {
    return XCWIntFromEnvironment(@"SIMDECK_LOCAL_STREAM_FPS",
                                 XCWTargetRealTimeFrameRate,
                                 XCWMinimumLocalStreamFrameRate,
                                 XCWMaximumLocalStreamFrameRate);
}

static uint64_t XCWLocalStreamFrameIntervalUs(void) {
    int32_t fps = MAX(1, XCWLocalStreamTargetFrameRate());
    return (uint64_t)llround(1000000.0 / (double)fps);
}

static int64_t XCWRealtimeBitsPerPixelBudgetValue(void) {
    return XCWInt64FromEnvironment(@"SIMDECK_REALTIME_BITS_PER_PIXEL",
                                   XCWRealtimeBitsPerPixelBudget,
                                   1,
                                   XCWBitsPerPixelBudget);
}

static int32_t XCWRealtimeMinimumAverageBitRate(void) {
    return XCWIntFromEnvironment(@"SIMDECK_REALTIME_MIN_BITRATE",
                                 XCWMinimumRealtimeAverageBitRate,
                                 750000,
                                 20000000);
}

static CMVideoCodecType XCWVideoCodecTypeForMode(XCWVideoEncoderMode mode) {
    switch (mode) {
        case XCWVideoEncoderModeAuto:
        case XCWVideoEncoderModeH264Hardware:
        case XCWVideoEncoderModeH264Software:
        default:
            return kCMVideoCodecType_H264;
    }
}

static NSString *XCWVideoEncoderModeName(XCWVideoEncoderMode mode) {
    switch (mode) {
        case XCWVideoEncoderModeAuto:
            return @"auto";
        case XCWVideoEncoderModeH264Hardware:
            return @"hardware";
        case XCWVideoEncoderModeH264Software:
        default:
            return @"software";
    }
}

static NSString *XCWVideoEncoderIDForMode(XCWVideoEncoderMode mode) {
    switch (mode) {
        case XCWVideoEncoderModeAuto:
            return nil;
        case XCWVideoEncoderModeH264Hardware:
            return @"com.apple.videotoolbox.videoencoder.ave.avc";
        case XCWVideoEncoderModeH264Software:
            return @"org.videolan.x264";
        default:
            return nil;
    }
}

static NSData *XCWDecoderConfigurationRecordFromFormatDescription(CMFormatDescriptionRef formatDescription,
                                                                  NSString *atomKey) {
    if (formatDescription == NULL || atomKey.length == 0) {
        return nil;
    }

    CFDictionaryRef atoms = CMFormatDescriptionGetExtension(formatDescription,
                                                            kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms);
    if (atoms == NULL || CFGetTypeID(atoms) != CFDictionaryGetTypeID()) {
        return nil;
    }

    CFTypeRef value = CFDictionaryGetValue(atoms, (__bridge CFStringRef)atomKey);
    if (value == NULL || CFGetTypeID(value) != CFDataGetTypeID()) {
        return nil;
    }

    return [(__bridge NSData *)value copy];
}

static NSString *XCWCodecStringFromSPS(NSData *spsData) {
    const uint8_t *bytes = spsData.bytes;
    if (spsData.length < 4 || bytes == NULL) {
        return @"avc1.640028";
    }
    return [NSString stringWithFormat:@"avc1.%02x%02x%02x", bytes[1], bytes[2], bytes[3]];
}

static NSData *XCWFirstAnnexBNALUOfType(NSData *sampleData, uint8_t nalType) {
    const uint8_t *bytes = sampleData.bytes;
    NSUInteger length = sampleData.length;
    NSUInteger offset = 0;
    while (offset + 4 < length) {
        NSUInteger startCodeLength = 0;
        if (offset + 4 <= length &&
            bytes[offset] == 0 &&
            bytes[offset + 1] == 0 &&
            bytes[offset + 2] == 0 &&
            bytes[offset + 3] == 1) {
            startCodeLength = 4;
        } else if (offset + 3 <= length &&
                   bytes[offset] == 0 &&
                   bytes[offset + 1] == 0 &&
                   bytes[offset + 2] == 1) {
            startCodeLength = 3;
        }
        if (startCodeLength == 0) {
            offset += 1;
            continue;
        }

        NSUInteger nalStart = offset + startCodeLength;
        if (nalStart >= length) {
            break;
        }
        NSUInteger nalEnd = nalStart;
        BOOL foundNextStartCode = NO;
        while (nalEnd + 3 < length) {
            if ((nalEnd + 4 <= length &&
                 bytes[nalEnd] == 0 &&
                 bytes[nalEnd + 1] == 0 &&
                 bytes[nalEnd + 2] == 0 &&
                 bytes[nalEnd + 3] == 1) ||
                (bytes[nalEnd] == 0 &&
                 bytes[nalEnd + 1] == 0 &&
                 bytes[nalEnd + 2] == 1)) {
                foundNextStartCode = YES;
                break;
            }
            nalEnd += 1;
        }
        if (!foundNextStartCode) {
            nalEnd = length;
        }
        if ((bytes[nalStart] & 0x1f) == nalType && nalEnd > nalStart) {
            return [NSData dataWithBytes:bytes + nalStart length:nalEnd - nalStart];
        }
        offset = nalEnd;
    }
    return nil;
}

static NSString *XCWCodecStringFromAnnexBSample(NSData *sampleData) {
    NSData *spsData = XCWFirstAnnexBNALUOfType(sampleData, 7);
    return spsData.length >= 4 ? XCWCodecStringFromSPS(spsData) : @"avc1.42e01f";
}

static NSData *XCWAVCDecoderConfigurationRecord(NSData *spsData, NSData *ppsData, int nalLengthHeader) {
    if (spsData.length == 0 || ppsData.length == 0) {
        return nil;
    }

    uint8_t lengthSizeMinusOne = (uint8_t)MIN(MAX(nalLengthHeader, 1), 4) - 1;
    const uint8_t *spsBytes = spsData.bytes;
    NSMutableData *record = [NSMutableData data];
    uint8_t header[6] = {
        0x01,
        spsBytes[1],
        spsBytes[2],
        spsBytes[3],
        (uint8_t)(0xFC | lengthSizeMinusOne),
        0xE1,
    };
    [record appendBytes:header length:sizeof(header)];

    uint16_t spsLength = CFSwapInt16HostToBig((uint16_t)spsData.length);
    [record appendBytes:&spsLength length:sizeof(spsLength)];
    [record appendData:spsData];

    uint8_t ppsCount = 0x01;
    [record appendBytes:&ppsCount length:sizeof(ppsCount)];
    uint16_t ppsLength = CFSwapInt16HostToBig((uint16_t)ppsData.length);
    [record appendBytes:&ppsLength length:sizeof(ppsLength)];
    [record appendData:ppsData];
    return record;
}

static NSString *XCWCodecName(CMVideoCodecType codecType) {
    switch (codecType) {
        case kCMVideoCodecType_H264:
            return @"h264";
        default:
            return [NSString stringWithFormat:@"0x%08x", (unsigned int)codecType];
    }
}

static NSData *XCWCopySampleData(CMSampleBufferRef sampleBuffer) {
    CMBlockBufferRef blockBuffer = CMSampleBufferGetDataBuffer(sampleBuffer);
    if (blockBuffer == NULL) {
        return nil;
    }

    size_t totalLength = 0;
    size_t contiguousLength = 0;
    char *dataPointer = NULL;
    OSStatus contiguousStatus =
        CMBlockBufferGetDataPointer(blockBuffer, 0, &contiguousLength, &totalLength, &dataPointer);
    if (contiguousStatus == noErr && dataPointer != NULL && totalLength > 0 && contiguousLength == totalLength) {
        CMBlockBufferRef retainedBlockBuffer = (CMBlockBufferRef)CFRetain(blockBuffer);
        return [[NSData alloc] initWithBytesNoCopy:dataPointer
                                            length:totalLength
                                       deallocator:^(__unused void *bytes, __unused NSUInteger length) {
            CFRelease(retainedBlockBuffer);
        }];
    }

    if (totalLength == 0) {
        totalLength = CMBlockBufferGetDataLength(blockBuffer);
    }
    if (totalLength == 0) {
        return nil;
    }

    NSMutableData *data = [NSMutableData dataWithLength:totalLength];
    OSStatus status = CMBlockBufferCopyDataBytes(blockBuffer, 0, totalLength, data.mutableBytes);
    return status == noErr ? data : nil;
}

static BOOL XCWSampleBufferIsKeyFrame(CMSampleBufferRef sampleBuffer) {
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, false);
    if (attachments == NULL || CFArrayGetCount(attachments) == 0) {
        return YES;
    }

    CFDictionaryRef attachment = CFArrayGetValueAtIndex(attachments, 0);
    return !CFDictionaryContainsKey(attachment, kCMSampleAttachmentKey_NotSync);
}

static int32_t XCWRoundToEvenDimension(double value) {
    int32_t rounded = (int32_t)llround(value);
    if (rounded < 2) {
        rounded = 2;
    }
    if ((rounded & 1) != 0) {
        rounded -= 1;
    }
    return rounded;
}

static CGSize XCWScaledDimensionsForSourceSize(int32_t width, int32_t height, XCWVideoEncoderMode mode, BOOL lowLatencyMode, BOOL realtimeStreamMode) {
    if (width <= 0 || height <= 0) {
        return CGSizeZero;
    }

    int32_t maximumDimension = realtimeStreamMode
        ? XCWRealtimeMaximumEncodedDimension()
        : XCWMaximumSoftwareEncodedDimension;
    if (mode == XCWVideoEncoderModeH264Software && lowLatencyMode) {
        maximumDimension = MIN(maximumDimension, XCWMaximumLowLatencySoftwareEncodedDimension);
    }
    int32_t longestEdge = MAX(width, height);
    if (longestEdge <= maximumDimension) {
        return CGSizeMake(width, height);
    }

    double scale = (double)maximumDimension / (double)longestEdge;
    return CGSizeMake(XCWRoundToEvenDimension(width * scale),
                      XCWRoundToEvenDimension(height * scale));
}

static int32_t XCWAverageBitRateForDimensions(int32_t width, int32_t height, XCWVideoEncoderMode mode, BOOL lowLatencyMode, BOOL realtimeStreamMode) {
    int64_t bitsPerPixelBudget = XCWBitsPerPixelBudget;
    int64_t minimumAverageBitRate = XCWMinimumAverageBitRate;
    if (realtimeStreamMode && !lowLatencyMode) {
        bitsPerPixelBudget = XCWRealtimeBitsPerPixelBudgetValue();
        minimumAverageBitRate = XCWRealtimeMinimumAverageBitRate();
    } else if (mode == XCWVideoEncoderModeH264Software || !realtimeStreamMode) {
        bitsPerPixelBudget = lowLatencyMode
            ? XCWLowLatencySoftwareBitsPerPixelBudget
            : XCWSoftwareBitsPerPixelBudget;
        minimumAverageBitRate = lowLatencyMode
            ? XCWMinimumLowLatencySoftwareAverageBitRate
            : XCWMinimumSoftwareAverageBitRate;
    }
    int64_t computedBitRate = (int64_t)width * (int64_t)height * bitsPerPixelBudget;
    if (computedBitRate < minimumAverageBitRate) {
        computedBitRate = minimumAverageBitRate;
    }
    if (computedBitRate > INT32_MAX) {
        computedBitRate = INT32_MAX;
    }
    return (int32_t)computedBitRate;
}

static void XCWSetCompressionProperty(const void *key, const void *value, void *context) {
    VTCompressionSessionRef session = (VTCompressionSessionRef)context;
    if (session == NULL || key == NULL || value == NULL) {
        return;
    }
    VTSessionSetProperty(session, (CFStringRef)key, (CFTypeRef)value);
}

static void XCWApplyCompressionPresetIfAvailable(VTCompressionSessionRef session) {
    if (session == NULL) {
        return;
    }

    if (@available(macOS 26.0, *)) {
        CFStringRef supportedPresetDictionariesKey = CFSTR("SupportedPresetDictionaries");
        CFStringRef videoConferencingPresetKey = CFSTR("VideoConferencing");
        CFStringRef highSpeedPresetKey = CFSTR("HighSpeed");
        CFTypeRef supportedPresets = NULL;
        OSStatus status = VTSessionCopyProperty(session,
                                                supportedPresetDictionariesKey,
                                                kCFAllocatorDefault,
                                                &supportedPresets);
        if (status != noErr || supportedPresets == NULL || CFGetTypeID(supportedPresets) != CFDictionaryGetTypeID()) {
            if (supportedPresets != NULL) {
                CFRelease(supportedPresets);
            }
            return;
        }

        CFDictionaryRef presets = (CFDictionaryRef)supportedPresets;
        CFDictionaryRef preset = CFDictionaryGetValue(presets, videoConferencingPresetKey);
        if (preset == NULL) {
            preset = CFDictionaryGetValue(presets, highSpeedPresetKey);
        }
        if (preset != NULL && CFGetTypeID(preset) == CFDictionaryGetTypeID()) {
            CFDictionaryApplyFunction(preset, XCWSetCompressionProperty, session);
        }
        CFRelease(supportedPresets);
    }
}

static BOOL XCWCompressionSessionUsesHardwareEncoder(VTCompressionSessionRef session) {
    if (session == NULL) {
        return NO;
    }

    CFTypeRef value = NULL;
    OSStatus status = VTSessionCopyProperty(session,
                                            kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
                                            kCFAllocatorDefault,
                                            &value);
    if (status != noErr || value == NULL) {
        return NO;
    }

    BOOL isHardware = CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value);
    CFRelease(value);
    return isHardware;
}

static NSString *XCWCompressionSessionEncoderID(VTCompressionSessionRef session) {
    if (session == NULL) {
        return nil;
    }

    CFTypeRef value = NULL;
    OSStatus status = VTSessionCopyProperty(session,
                                            kVTCompressionPropertyKey_EncoderID,
                                            kCFAllocatorDefault,
                                            &value);
    if (status != noErr || value == NULL) {
        return nil;
    }

    NSString *encoderID = nil;
    if (CFGetTypeID(value) == CFStringGetTypeID()) {
        encoderID = [(__bridge NSString *)value copy];
    }
    CFRelease(value);
    return encoderID;
}

static BOOL XCWPixelFormatSupportsSoftwareScaling(OSType pixelFormat) {
    switch (pixelFormat) {
        case kCVPixelFormatType_32ARGB:
        case kCVPixelFormatType_32BGRA:
        case kCVPixelFormatType_32ABGR:
        case kCVPixelFormatType_32RGBA:
            return YES;
        default:
            return NO;
    }
}

static void XCWCopyPlaneRows(uint8_t *destination,
                             size_t destinationStride,
                             const uint8_t *source,
                             size_t sourceStride,
                             size_t rowBytes,
                             size_t rows) {
    for (size_t row = 0; row < rows; row++) {
        memcpy(destination + (row * destinationStride),
               source + (row * sourceStride),
               rowBytes);
    }
}

static void XCWH264EncoderOutputCallback(void *outputCallbackRefCon,
                                         void *sourceFrameRefCon,
                                         OSStatus status,
                                         VTEncodeInfoFlags infoFlags,
                                         CMSampleBufferRef sampleBuffer);

@interface XCWH264Encoder ()

@property (nonatomic, copy, readonly) XCWH264EncoderOutputHandler outputHandler;

- (nullable CVPixelBufferRef)copySoftwareScaledPixelBuffer:(CVPixelBufferRef)pixelBuffer
                                               targetWidth:(int32_t)targetWidth
                                              targetHeight:(int32_t)targetHeight;
- (BOOL)shouldUseSoftwareScalerForSourceWidth:(int32_t)sourceWidth
                                  sourceHeight:(int32_t)sourceHeight
                                   targetWidth:(int32_t)targetWidth
                                  targetHeight:(int32_t)targetHeight
                                   pixelFormat:(OSType)pixelFormat;
- (nullable CVPixelBufferRef)copyPixelBufferFromScalingPoolWithWidth:(int32_t)targetWidth
                                                              height:(int32_t)targetHeight
                                                         pixelFormat:(OSType)pixelFormat;
- (BOOL)encodePixelBufferWithX264Locked:(CVPixelBufferRef)pixelBuffer
                             targetWidth:(int32_t)targetWidth
                            targetHeight:(int32_t)targetHeight
                                  timeUs:(uint64_t)nowUs
                      relativeTimestampUs:(uint64_t)relativeTimestampUs
                           forceKeyFrame:(BOOL)forceKeyFrame;
- (BOOL)ensureX264EncoderWithWidth:(int32_t)width height:(int32_t)height;
- (BOOL)copyPixelBufferIntoX264PictureLocked:(CVPixelBufferRef)pixelBuffer;
- (BOOL)ensureX264ColorConversionLocked;
- (void)recordEncodeLatencyLockedWithSubmittedAtUs:(uint64_t)submittedAtUs measuredAtUs:(uint64_t)measuredAtUs;
- (void)invalidateX264EncoderLocked;
- (void)handleCompressionOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
                              submittedAtUs:(uint64_t)submittedAtUs;
- (uint64_t)activeFrameIntervalUsLocked;
- (uint64_t)encoderLatencyBudgetUsLocked;
- (uint64_t)pacingDelayBeforeNextFrameAtTimeUs:(uint64_t)nowUs;
- (void)recordFrameSubmissionLockedAtTimeUs:(uint64_t)nowUs software:(BOOL)software;
- (void)scheduleDrainAfterDelayUs:(uint64_t)delayUs;

@end

@implementation XCWH264Encoder {
    dispatch_queue_t _queue;
    VTCompressionSessionRef _compressionSession;
    os_unfair_lock _pendingLock;
    CVPixelBufferRef _pendingPixelBuffer;
    BOOL _drainScheduled;
    BOOL _drainingPendingFrames;
    BOOL _needsKeyFrame;
    NSUInteger _inFlightFrameCount;
    int32_t _width;
    int32_t _height;
    uint64_t _timestampOriginUs;
    VTPixelTransferSessionRef _pixelTransferSession;
    CVPixelBufferPoolRef _scaledPixelBufferPool;
    int32_t _scaledPixelBufferWidth;
    int32_t _scaledPixelBufferHeight;
    OSType _scaledPixelFormat;
    BOOL _scalingActive;
    XCWVideoEncoderMode _encoderMode;
    XCWVideoEncoderMode _activeEncoderMode;
    BOOL _clientForeground;
    BOOL _lowLatencyMode;
    BOOL _realtimeStreamMode;
    CMVideoCodecType _codecType;
    BOOL _hardwareAccelerated;
    NSUInteger _inputFrameCount;
    NSUInteger _pendingReplacementCount;
    NSUInteger _submittedFrameCount;
    NSUInteger _encodeFailureCount;
    NSUInteger _outputFrameCount;
    NSUInteger _keyFrameOutputCount;
    NSUInteger _maxInFlightFrameCount;
    uint64_t _latestEncodeLatencyUs;
    double _averageEncodeLatencyUs;
    uint64_t _peakEncodeLatencyUs;
    NSUInteger _overBudgetFrameCount;
    NSUInteger _consecutiveOverBudgetFrameCount;
    NSUInteger _consecutiveStrainedFrameCount;
    NSUInteger _overloadEventCount;
    BOOL _wasOverloaded;
    uint64_t _softwareFrameIntervalUs;
    uint64_t _lastSoftwareSubmissionUs;
    uint64_t _nextSoftwareSubmissionDueUs;
    NSUInteger _softwarePacedFrameCount;
    NSUInteger _softwareHealthyFrameCount;
    uint64_t _hardwareFrameIntervalUs;
    uint64_t _lastHardwareSubmissionUs;
    uint64_t _nextHardwareSubmissionDueUs;
    NSUInteger _hardwarePacedFrameCount;
    uint64_t _autoSoftwareFallbackUntilUs;
    NSUInteger _autoSoftwareFallbackCount;
    NSUInteger _autoHardwareRetryCount;
    NSString *_selectedEncoderID;
    NSInteger _lastSessionStatus;
    NSInteger _lastPrepareStatus;
    NSInteger _lastScaleStatus;
    NSInteger _lastEncodeStatus;
    x264_t *_x264Encoder;
    x264_picture_t _x264Picture;
    BOOL _x264PictureAllocated;
    vImage_ARGBToYpCbCr _x264ColorConversion;
    BOOL _x264ColorConversionReady;
    int32_t _x264Width;
    int32_t _x264Height;
    uint64_t _x264FrameIndex;
    NSString *_x264Codec;
}

- (instancetype)initWithOutputHandler:(XCWH264EncoderOutputHandler)outputHandler {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _outputHandler = [outputHandler copy];
    dispatch_queue_attr_t queueAttributes =
        dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_USER_INITIATED, 0);
    _queue = dispatch_queue_create("com.simdeck.h264-encoder", queueAttributes);
    dispatch_queue_set_specific(_queue,
                                XCWH264EncoderQueueSpecificKey,
                                XCWH264EncoderQueueSpecificKey,
                                NULL);
    _pendingLock = OS_UNFAIR_LOCK_INIT;
    _needsKeyFrame = YES;
    _encoderMode = XCWVideoEncoderModeFromEnvironment();
    _activeEncoderMode = _encoderMode;
    _clientForeground = YES;
    _lowLatencyMode = (_encoderMode == XCWVideoEncoderModeH264Software) && XCWLowLatencyModeFromEnvironment();
    _realtimeStreamMode = XCWRealtimeStreamModeFromEnvironment() || _lowLatencyMode;
    _codecType = XCWVideoCodecTypeForMode(_activeEncoderMode);
    _softwareFrameIntervalUs = [self initialSoftwareFrameIntervalUsLocked];
    _hardwareFrameIntervalUs = [self initialHardwareFrameIntervalUsLocked];
    return self;
}

- (void)dealloc {
    [self invalidate];
}

- (void)encodePixelBuffer:(CVPixelBufferRef)pixelBuffer {
    if (pixelBuffer == NULL) {
        return;
    }

    CVPixelBufferRetain(pixelBuffer);
    BOOL shouldScheduleDrain = NO;
    os_unfair_lock_lock(&_pendingLock);
    _inputFrameCount += 1;
    if (_pendingPixelBuffer != NULL) {
        _pendingReplacementCount += 1;
        CVPixelBufferRelease(_pendingPixelBuffer);
    }
    _pendingPixelBuffer = pixelBuffer;
    if (!_drainScheduled) {
        _drainScheduled = YES;
        shouldScheduleDrain = YES;
    }
    os_unfair_lock_unlock(&_pendingLock);

    if (!shouldScheduleDrain) {
        return;
    }

    dispatch_async(_queue, ^{
        [self drainPendingFramesLocked];
    });
}

- (void)requestKeyFrame {
    dispatch_async(_queue, ^{
        self->_needsKeyFrame = YES;
    });
}

- (void)reconfigureForStreamQualityChange {
    dispatch_async(_queue, ^{
        [self invalidateCompressionSessionLocked];
        self->_encoderMode = XCWVideoEncoderModeFromEnvironment();
        self->_activeEncoderMode = self->_encoderMode;
        self->_lowLatencyMode = (self->_encoderMode == XCWVideoEncoderModeH264Software) && XCWLowLatencyModeFromEnvironment();
        self->_realtimeStreamMode = XCWRealtimeStreamModeFromEnvironment() || self->_lowLatencyMode;
        self->_codecType = XCWVideoCodecTypeForMode(self->_activeEncoderMode);
        self->_needsKeyFrame = YES;
        self->_softwareFrameIntervalUs = [self initialSoftwareFrameIntervalUsLocked];
        self->_softwarePacedFrameCount = 0;
        self->_softwareHealthyFrameCount = 0;
        self->_nextSoftwareSubmissionDueUs = 0;
        self->_hardwareFrameIntervalUs = [self initialHardwareFrameIntervalUsLocked];
        self->_hardwarePacedFrameCount = 0;
        self->_nextHardwareSubmissionDueUs = 0;
        self->_autoSoftwareFallbackUntilUs = 0;
        [self updateActiveEncoderModeForClientForegroundLockedAtTimeUs:(uint64_t)(CACurrentMediaTime() * 1000000.0)];
    });
}

- (void)setClientForeground:(BOOL)foreground {
    dispatch_async(_queue, ^{
        if (self->_clientForeground == foreground) {
            return;
        }
        self->_clientForeground = foreground;
        [self updateActiveEncoderModeForClientForegroundLockedAtTimeUs:(uint64_t)(CACurrentMediaTime() * 1000000.0)];
    });
}

- (void)resetStatistics {
    os_unfair_lock_lock(&_pendingLock);
    _inputFrameCount = 0;
    _pendingReplacementCount = 0;
    os_unfair_lock_unlock(&_pendingLock);

    dispatch_sync(_queue, ^{
        self->_submittedFrameCount = 0;
        self->_encodeFailureCount = 0;
        self->_outputFrameCount = 0;
        self->_keyFrameOutputCount = 0;
        self->_maxInFlightFrameCount = self->_inFlightFrameCount;
        self->_latestEncodeLatencyUs = 0;
        self->_averageEncodeLatencyUs = 0;
        self->_peakEncodeLatencyUs = 0;
        self->_overBudgetFrameCount = 0;
        self->_consecutiveOverBudgetFrameCount = 0;
        self->_consecutiveStrainedFrameCount = 0;
        self->_overloadEventCount = 0;
        self->_wasOverloaded = NO;
        self->_softwarePacedFrameCount = 0;
        self->_softwareHealthyFrameCount = 0;
        self->_hardwarePacedFrameCount = 0;
        self->_nextSoftwareSubmissionDueUs = 0;
        self->_nextHardwareSubmissionDueUs = 0;
    });
}

- (NSDictionary *)statsRepresentation {
    __block NSUInteger inputFrameCount = 0;
    __block NSUInteger pendingReplacementCount = 0;
    __block BOOL pendingFrame = NO;
    __block BOOL drainScheduled = NO;
    os_unfair_lock_lock(&_pendingLock);
    inputFrameCount = _inputFrameCount;
    pendingReplacementCount = _pendingReplacementCount;
    pendingFrame = _pendingPixelBuffer != NULL;
    drainScheduled = _drainScheduled;
    os_unfair_lock_unlock(&_pendingLock);

    __block NSDictionary *stats = nil;
    dispatch_sync(_queue, ^{
        uint64_t encoderBudgetUs = [self encoderLatencyBudgetUsLocked];
        double latestLoadPercent = encoderBudgetUs > 0
            ? ((double)self->_latestEncodeLatencyUs * 100.0) / (double)encoderBudgetUs
            : 0.0;
        double averageLoadPercent = encoderBudgetUs > 0
            ? (self->_averageEncodeLatencyUs * 100.0) / (double)encoderBudgetUs
            : 0.0;
        BOOL overloaded = averageLoadPercent >= XCWEncoderOverloadedLoadPercent ||
            self->_consecutiveOverBudgetFrameCount >= XCWEncoderConsecutiveOverBudgetFrameThreshold;
        BOOL strained = overloaded ||
            averageLoadPercent >= XCWEncoderStrainedLoadPercent ||
            self->_consecutiveStrainedFrameCount >= XCWEncoderConsecutiveOverBudgetFrameThreshold;
        NSString *overloadState = overloaded
            ? @"overloaded"
            : strained
                ? @"strained"
                : @"nominal";
        NSString *overloadReason = @"within-budget";
        if (overloaded) {
            overloadReason = self->_consecutiveOverBudgetFrameCount >= XCWEncoderConsecutiveOverBudgetFrameThreshold
                ? @"consecutive-frames-over-budget"
                : @"average-latency-over-budget";
        } else if (strained) {
            overloadReason = averageLoadPercent >= XCWEncoderStrainedLoadPercent
                ? @"average-latency-near-budget"
                : @"consecutive-frames-near-budget";
        }
        uint64_t nowUs = (uint64_t)(CACurrentMediaTime() * 1000000.0);
        BOOL autoSoftwareFallbackActive = [self isAutoSoftwareFallbackActiveLocked];
        uint64_t autoSoftwareFallbackRemainingUs = autoSoftwareFallbackActive &&
            self->_autoSoftwareFallbackUntilUs > nowUs
                ? self->_autoSoftwareFallbackUntilUs - nowUs
                : 0;
        stats = @{
            @"inputFrames": @(inputFrameCount),
            @"pendingReplacements": @(pendingReplacementCount),
            @"submittedFrames": @(self->_submittedFrameCount),
            @"encodeFailures": @(self->_encodeFailureCount),
            @"outputFrames": @(self->_outputFrameCount),
            @"keyFrameOutputs": @(self->_keyFrameOutputCount),
            @"inFlightFrames": @(self->_inFlightFrameCount),
            @"pendingFrame": @(pendingFrame),
            @"drainScheduled": @(drainScheduled),
            @"maxInFlightFrames": @(self->_maxInFlightFrameCount),
            @"latestEncodeLatencyUs": @(self->_latestEncodeLatencyUs),
            @"averageEncodeLatencyUs": @(self->_averageEncodeLatencyUs),
            @"peakEncodeLatencyUs": @(self->_peakEncodeLatencyUs),
            @"encoderBudgetUs": @(encoderBudgetUs),
            @"encoderLoadPercent": @(latestLoadPercent),
            @"averageEncoderLoadPercent": @(averageLoadPercent),
            @"overloadState": overloadState,
            @"overloaded": @(overloaded),
            @"overloadReason": overloadReason,
            @"overBudgetFrames": @(self->_overBudgetFrameCount),
            @"consecutiveOverBudgetFrames": @(self->_consecutiveOverBudgetFrameCount),
            @"consecutiveStrainedFrames": @(self->_consecutiveStrainedFrameCount),
            @"overloadEvents": @(self->_overloadEventCount),
            @"softwareFrameIntervalUs": @(self->_softwareFrameIntervalUs),
            @"softwareTargetFps": @(self->_softwareFrameIntervalUs > 0 ? (1000000.0 / (double)self->_softwareFrameIntervalUs) : 0.0),
            @"softwarePacedFrames": @(self->_softwarePacedFrameCount),
            @"localStreamTargetFps": @(XCWLocalStreamTargetFrameRate()),
            @"hardwareFrameIntervalUs": @(self->_hardwareFrameIntervalUs),
            @"hardwareTargetFps": @(self->_hardwareFrameIntervalUs > 0 ? (1000000.0 / (double)self->_hardwareFrameIntervalUs) : 0.0),
            @"hardwarePacedFrames": @(self->_hardwarePacedFrameCount),
            @"transportCodec": XCWCodecName(self->_codecType),
            @"codecString": self->_x264Codec ?: NSNull.null,
            @"encoderMode": XCWVideoEncoderModeName(self->_encoderMode),
            @"activeEncoderMode": XCWVideoEncoderModeName(self->_activeEncoderMode),
            @"clientForeground": @(self->_clientForeground),
            @"autoSoftwareFallbackActive": @(autoSoftwareFallbackActive),
            @"autoSoftwareFallbackRemainingUs": @(autoSoftwareFallbackRemainingUs),
            @"autoSoftwareFallbacks": @(self->_autoSoftwareFallbackCount),
            @"autoHardwareRetries": @(self->_autoHardwareRetryCount),
            @"lowLatencyMode": @(self->_lowLatencyMode),
            @"realtimeStreamMode": @(self->_realtimeStreamMode),
            @"encoderId": XCWVideoEncoderIDForMode(self->_activeEncoderMode) ?: @"automatic",
            @"selectedEncoderId": self->_selectedEncoderID ?: NSNull.null,
            @"hardwareAccelerated": @(self->_hardwareAccelerated),
            @"lastSessionStatus": @(self->_lastSessionStatus),
            @"lastPrepareStatus": @(self->_lastPrepareStatus),
            @"lastScaleStatus": @(self->_lastScaleStatus),
            @"lastEncodeStatus": @(self->_lastEncodeStatus),
        };
    });
    return stats;
}

- (void)invalidate {
    dispatch_sync(_queue, ^{
        [self drainPendingFramesLocked];
        [self invalidateCompressionSessionLocked];
    });

    os_unfair_lock_lock(&_pendingLock);
    if (_pendingPixelBuffer != NULL) {
        CVPixelBufferRelease(_pendingPixelBuffer);
        _pendingPixelBuffer = NULL;
    }
    _drainScheduled = NO;
    os_unfair_lock_unlock(&_pendingLock);
}

- (NSUInteger)maximumInFlightFrameCountLocked {
    if (_lowLatencyMode) {
        return 1;
    }
    if (_realtimeStreamMode) {
        return XCWMaximumRealtimeInFlightFrames;
    }
    return XCWMaximumInFlightFrames;
}

- (uint64_t)minimumSoftwareFrameIntervalUsLocked {
    if (_realtimeStreamMode && !_lowLatencyMode) {
        return XCWRealtimeFrameIntervalUs();
    }
    return _lowLatencyMode ? XCWLowLatencySoftwareMinimumFrameIntervalUs : XCWSoftwareMinimumFrameIntervalUs;
}

- (uint64_t)initialSoftwareFrameIntervalUsLocked {
    if (_realtimeStreamMode && !_lowLatencyMode) {
        return XCWRealtimeFrameIntervalUs();
    }
    return _lowLatencyMode ? XCWLowLatencySoftwareInitialFrameIntervalUs : XCWSoftwareInitialFrameIntervalUs;
}

- (uint64_t)maximumSoftwareFrameIntervalUsLocked {
    if (_realtimeStreamMode && !_lowLatencyMode) {
        return XCWRealtimeMaximumFrameIntervalUs();
    }
    return _lowLatencyMode ? XCWLowLatencySoftwareMaximumFrameIntervalUs : XCWSoftwareMaximumFrameIntervalUs;
}

- (uint64_t)softwareFrameIntervalStepUsLocked {
    return _lowLatencyMode ? XCWLowLatencySoftwareFrameIntervalStepUs : XCWSoftwareFrameIntervalStepUs;
}

- (NSUInteger)softwareHealthyFrameWindowLocked {
    return _lowLatencyMode ? XCWLowLatencySoftwareHealthyFrameWindow : XCWSoftwareHealthyFrameWindow;
}

- (uint64_t)minimumHardwareFrameIntervalUsLocked {
    return _realtimeStreamMode ? XCWRealtimeFrameIntervalUs() : XCWLocalStreamFrameIntervalUs();
}

- (uint64_t)initialHardwareFrameIntervalUsLocked {
    return _realtimeStreamMode ? XCWRealtimeFrameIntervalUs() : XCWLocalStreamFrameIntervalUs();
}

- (BOOL)isAutoSoftwareFallbackActiveLocked {
    return _encoderMode == XCWVideoEncoderModeAuto &&
        _autoSoftwareFallbackUntilUs != 0 &&
        _activeEncoderMode == XCWVideoEncoderModeH264Software;
}

- (void)resetAutoFallbackLatencyStateLocked {
    _latestEncodeLatencyUs = 0;
    _averageEncodeLatencyUs = 0;
    _peakEncodeLatencyUs = 0;
    _consecutiveOverBudgetFrameCount = 0;
    _consecutiveStrainedFrameCount = 0;
    _wasOverloaded = NO;
}

- (void)switchActiveEncoderModeLocked:(XCWVideoEncoderMode)mode {
    if (_activeEncoderMode == mode) {
        return;
    }
    _activeEncoderMode = mode;
    _codecType = XCWVideoCodecTypeForMode(_activeEncoderMode);
    if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
        _softwareFrameIntervalUs = [self initialSoftwareFrameIntervalUsLocked];
        _softwareHealthyFrameCount = 0;
        _softwarePacedFrameCount = 0;
    } else {
        _hardwareFrameIntervalUs = [self initialHardwareFrameIntervalUsLocked];
        _hardwarePacedFrameCount = 0;
    }
    [self invalidateCompressionSessionLocked];
    [self resetAutoFallbackLatencyStateLocked];
    _needsKeyFrame = YES;
}

- (void)updateActiveEncoderModeForClientForegroundLockedAtTimeUs:(uint64_t)nowUs {
    if (_encoderMode == XCWVideoEncoderModeAuto &&
        _autoSoftwareFallbackUntilUs != 0 &&
        nowUs < _autoSoftwareFallbackUntilUs) {
        [self switchActiveEncoderModeLocked:XCWVideoEncoderModeH264Software];
        return;
    }
    if (_encoderMode == XCWVideoEncoderModeAuto && _autoSoftwareFallbackUntilUs != 0) {
        _autoSoftwareFallbackUntilUs = 0;
        _autoHardwareRetryCount += 1;
    }
    [self switchActiveEncoderModeLocked:_encoderMode];
}

- (void)enterAutoSoftwareFallbackLockedAtTimeUs:(uint64_t)nowUs {
    if (_encoderMode != XCWVideoEncoderModeAuto ||
        _activeEncoderMode == XCWVideoEncoderModeH264Software) {
        return;
    }
    _autoSoftwareFallbackUntilUs = nowUs + XCWAutoHardwareRetryIntervalUs;
    _autoSoftwareFallbackCount += 1;
    [self switchActiveEncoderModeLocked:XCWVideoEncoderModeH264Software];
}

- (void)retryAutoHardwareIfNeededLockedAtTimeUs:(uint64_t)nowUs {
    if (![self isAutoSoftwareFallbackActiveLocked] ||
        !_clientForeground ||
        _autoSoftwareFallbackUntilUs == 0 ||
        nowUs < _autoSoftwareFallbackUntilUs) {
        return;
    }
    _autoSoftwareFallbackUntilUs = 0;
    _autoHardwareRetryCount += 1;
    [self switchActiveEncoderModeLocked:XCWVideoEncoderModeAuto];
}

- (uint64_t)activeFrameIntervalUsLocked {
    if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
        return _softwareFrameIntervalUs > 0 ? _softwareFrameIntervalUs : [self initialSoftwareFrameIntervalUsLocked];
    }
    if (_activeEncoderMode == XCWVideoEncoderModeAuto || _activeEncoderMode == XCWVideoEncoderModeH264Hardware) {
        return _hardwareFrameIntervalUs > 0 ? _hardwareFrameIntervalUs : [self initialHardwareFrameIntervalUsLocked];
    }
    int32_t expectedFrameRate = MAX(1, [self expectedFrameRateLocked]);
    return (uint64_t)llround(1000000.0 / (double)expectedFrameRate);
}

- (uint64_t)encoderLatencyBudgetUsLocked {
    uint64_t frameIntervalUs = [self activeFrameIntervalUsLocked];
    if (frameIntervalUs == 0 || _activeEncoderMode == XCWVideoEncoderModeH264Software) {
        return frameIntervalUs;
    }
    if (_realtimeStreamMode) {
        return frameIntervalUs * (MAX((NSUInteger)1, [self maximumInFlightFrameCountLocked]) + 1);
    }
    return frameIntervalUs;
}

- (uint64_t)pacingDelayBeforeNextFrameAtTimeUs:(uint64_t)nowUs {
    if (_needsKeyFrame) {
        return 0;
    }

    uint64_t nextDueUs = 0;
    BOOL software = NO;
    if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
        if (_softwareFrameIntervalUs == 0) {
            _softwareFrameIntervalUs = [self initialSoftwareFrameIntervalUsLocked];
        }
        nextDueUs = _nextSoftwareSubmissionDueUs;
        software = YES;
    } else if (_activeEncoderMode == XCWVideoEncoderModeAuto || _activeEncoderMode == XCWVideoEncoderModeH264Hardware) {
        if (_hardwareFrameIntervalUs == 0) {
            _hardwareFrameIntervalUs = [self initialHardwareFrameIntervalUsLocked];
        }
        nextDueUs = _nextHardwareSubmissionDueUs;
    }
    if (nextDueUs == 0 || nowUs >= nextDueUs) {
        return 0;
    }

    if (software) {
        _softwarePacedFrameCount += 1;
    } else {
        _hardwarePacedFrameCount += 1;
    }
    return nextDueUs - nowUs;
}

- (void)recordFrameSubmissionLockedAtTimeUs:(uint64_t)nowUs software:(BOOL)software {
    uint64_t intervalUs = 0;
    uint64_t nextDueUs = 0;
    if (software) {
        if (_softwareFrameIntervalUs == 0) {
            _softwareFrameIntervalUs = [self initialSoftwareFrameIntervalUsLocked];
        }
        intervalUs = _softwareFrameIntervalUs;
        nextDueUs = _nextSoftwareSubmissionDueUs;
        _lastSoftwareSubmissionUs = nowUs;
    } else {
        if (_hardwareFrameIntervalUs == 0) {
            _hardwareFrameIntervalUs = [self initialHardwareFrameIntervalUsLocked];
        }
        intervalUs = _hardwareFrameIntervalUs;
        nextDueUs = _nextHardwareSubmissionDueUs;
        _lastHardwareSubmissionUs = nowUs;
    }

    if (intervalUs == 0 || nextDueUs == 0) {
        nextDueUs = nowUs + intervalUs;
    } else if (nowUs >= nextDueUs) {
        uint64_t skippedIntervals = ((nowUs - nextDueUs) / intervalUs) + 1;
        nextDueUs += skippedIntervals * intervalUs;
    }

    if (software) {
        _nextSoftwareSubmissionDueUs = nextDueUs;
    } else {
        _nextHardwareSubmissionDueUs = nextDueUs;
    }
}

- (void)scheduleDrainAfterDelayUs:(uint64_t)delayUs {
    uint64_t clampedDelayUs = MAX(delayUs, 1000);
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(clampedDelayUs * 1000ull)), _queue, ^{
        [self drainPendingFramesLocked];
    });
}

- (int32_t)expectedFrameRateLocked {
    if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
        if (_lowLatencyMode) {
            return XCWTargetLowLatencySoftwareFrameRate;
        }
        return _realtimeStreamMode ? XCWRealtimeTargetFrameRate() : XCWTargetSoftwareFrameRate;
    }
    if (_realtimeStreamMode) {
        return XCWRealtimeTargetFrameRate();
    }
    return XCWLocalStreamTargetFrameRate();
}

- (void)adaptSoftwarePacingForLatencyUs:(uint64_t)latencyUs {
    if (_activeEncoderMode != XCWVideoEncoderModeH264Software || !_lowLatencyMode || latencyUs == 0) {
        return;
    }
    if (_softwareFrameIntervalUs == 0) {
        _softwareFrameIntervalUs = [self initialSoftwareFrameIntervalUsLocked];
    }

    uint64_t highLatencyBudgetUs = _lowLatencyMode ? _softwareFrameIntervalUs : ((_softwareFrameIntervalUs * 3) / 2);
    if (latencyUs > highLatencyBudgetUs) {
        uint64_t stepUs = [self softwareFrameIntervalStepUsLocked];
        uint64_t nextIntervalUs = _softwareFrameIntervalUs + stepUs;
        uint64_t latencyBoundIntervalUs = latencyUs + stepUs;
        if (nextIntervalUs < latencyBoundIntervalUs) {
            nextIntervalUs = latencyBoundIntervalUs;
        }
        _softwareFrameIntervalUs = MIN(nextIntervalUs, [self maximumSoftwareFrameIntervalUsLocked]);
        _softwareHealthyFrameCount = 0;
        return;
    }

    if (latencyUs < _softwareFrameIntervalUs &&
        _softwareFrameIntervalUs > [self minimumSoftwareFrameIntervalUsLocked]) {
        _softwareHealthyFrameCount += 1;
        if (_softwareHealthyFrameCount >= [self softwareHealthyFrameWindowLocked]) {
            uint64_t stepUs = [self softwareFrameIntervalStepUsLocked];
            uint64_t minimumIntervalUs = [self minimumSoftwareFrameIntervalUsLocked];
            uint64_t nextIntervalUs = _softwareFrameIntervalUs > stepUs
                ? _softwareFrameIntervalUs - stepUs
                : minimumIntervalUs;
            _softwareFrameIntervalUs = MAX(nextIntervalUs, minimumIntervalUs);
            _softwareHealthyFrameCount = 0;
        }
        return;
    }

    _softwareHealthyFrameCount = 0;
}

- (void)drainPendingFramesLocked {
    BOOL wasDraining = _drainingPendingFrames;
    _drainingPendingFrames = YES;
    while (YES) {
        if (_inFlightFrameCount >= [self maximumInFlightFrameCountLocked]) {
            _drainScheduled = NO;
            _drainingPendingFrames = wasDraining;
            return;
        }

        os_unfair_lock_lock(&_pendingLock);
        BOOL hasPendingFrame = _pendingPixelBuffer != NULL;
        if (!hasPendingFrame) {
            _drainScheduled = NO;
            os_unfair_lock_unlock(&_pendingLock);
            _drainingPendingFrames = wasDraining;
            return;
        }
        os_unfair_lock_unlock(&_pendingLock);

        uint64_t nowUs = (uint64_t)(CACurrentMediaTime() * 1000000.0);
        uint64_t pacingDelayUs = [self pacingDelayBeforeNextFrameAtTimeUs:nowUs];
        if (pacingDelayUs > 0) {
            [self scheduleDrainAfterDelayUs:pacingDelayUs];
            _drainingPendingFrames = wasDraining;
            return;
        }

        CVPixelBufferRef pixelBuffer = NULL;
        os_unfair_lock_lock(&_pendingLock);
        pixelBuffer = _pendingPixelBuffer;
        _pendingPixelBuffer = NULL;
        os_unfair_lock_unlock(&_pendingLock);

        [self encodePixelBufferLocked:pixelBuffer];
        CVPixelBufferRelease(pixelBuffer);
    }
}

- (BOOL)encodePixelBufferLocked:(CVPixelBufferRef)pixelBuffer {
    int32_t sourceWidth = (int32_t)CVPixelBufferGetWidth(pixelBuffer);
    int32_t sourceHeight = (int32_t)CVPixelBufferGetHeight(pixelBuffer);
    if (sourceWidth <= 0 || sourceHeight <= 0) {
        return NO;
    }

    uint64_t nowUs = (uint64_t)(CACurrentMediaTime() * 1000000.0);
    [self retryAutoHardwareIfNeededLockedAtTimeUs:nowUs];

    CGSize targetSize = XCWScaledDimensionsForSourceSize(sourceWidth, sourceHeight, _activeEncoderMode, _lowLatencyMode, _realtimeStreamMode);
    int32_t targetWidth = (int32_t)targetSize.width;
    int32_t targetHeight = (int32_t)targetSize.height;
    if (targetWidth <= 0 || targetHeight <= 0) {
        return NO;
    }
    _scalingActive = sourceWidth != targetWidth || sourceHeight != targetHeight;

    CVPixelBufferRef encodePixelBuffer = [self copyScaledPixelBufferIfNeeded:pixelBuffer
                                                                 targetWidth:targetWidth
                                                                targetHeight:targetHeight];
    if (encodePixelBuffer == NULL) {
        return NO;
    }

    if (_timestampOriginUs == 0) {
        _timestampOriginUs = nowUs;
    }
    uint64_t relativeTimestampUs = nowUs - _timestampOriginUs;

    BOOL forceKeyFrame = _needsKeyFrame;
    if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
        BOOL encoded = [self encodePixelBufferWithX264Locked:encodePixelBuffer
                                                 targetWidth:targetWidth
                                                targetHeight:targetHeight
                                                     timeUs:nowUs
                                         relativeTimestampUs:relativeTimestampUs
                                              forceKeyFrame:forceKeyFrame];
        CVPixelBufferRelease(encodePixelBuffer);
        if (encoded) {
            _needsKeyFrame = NO;
        }
        return encoded;
    }

    if (![self ensureCompressionSessionWithWidth:targetWidth height:targetHeight]) {
        CVPixelBufferRelease(encodePixelBuffer);
        if (_activeEncoderMode == XCWVideoEncoderModeAuto) {
            _encodeFailureCount += 1;
            [self enterAutoSoftwareFallbackLockedAtTimeUs:nowUs];
            return [self encodePixelBufferLocked:pixelBuffer];
        }
        return NO;
    }

    CMTime presentationTime = CMTimeMake((int64_t)relativeTimestampUs, 1000000);
    NSDictionary *frameOptions = nil;
    if (forceKeyFrame) {
        frameOptions = @{ (__bridge NSString *)kVTEncodeFrameOptionKey_ForceKeyFrame: @YES };
        _needsKeyFrame = NO;
    }

    OSStatus status = VTCompressionSessionEncodeFrame(_compressionSession,
                                                      encodePixelBuffer,
                                                      presentationTime,
                                                      kCMTimeInvalid,
                                                      (__bridge CFDictionaryRef _Nullable)(frameOptions),
                                                      (void *)(uintptr_t)nowUs,
                                                      NULL);
    _lastEncodeStatus = status;
    CVPixelBufferRelease(encodePixelBuffer);
    if (status != noErr) {
        _needsKeyFrame = YES;
        _encodeFailureCount += 1;
        if (_activeEncoderMode == XCWVideoEncoderModeAuto) {
            [self enterAutoSoftwareFallbackLockedAtTimeUs:nowUs];
            return [self encodePixelBufferLocked:pixelBuffer];
        }
        return NO;
    }

    _inFlightFrameCount += 1;
    _submittedFrameCount += 1;
    [self recordFrameSubmissionLockedAtTimeUs:nowUs software:NO];
    _maxInFlightFrameCount = MAX(_maxInFlightFrameCount, _inFlightFrameCount);
    if (_activeEncoderMode == XCWVideoEncoderModeH264Software || !_realtimeStreamMode) {
        VTCompressionSessionCompleteFrames(_compressionSession, presentationTime);
    }
    return YES;
}

- (BOOL)ensureCompressionSessionWithWidth:(int32_t)width height:(int32_t)height {
    if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
        return NO;
    }
    if (_compressionSession != NULL && _width == width && _height == height) {
        return YES;
    }

    [self invalidateCompressionSessionLocked];

    NSMutableDictionary *encoderSpecification = [NSMutableDictionary dictionary];
    NSString *encoderID = XCWVideoEncoderIDForMode(_activeEncoderMode);
    if (encoderID.length > 0) {
        encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_EncoderID] = encoderID;
    }
    if (_activeEncoderMode != XCWVideoEncoderModeH264Software && _lowLatencyMode) {
        if (@available(macOS 11.3, *)) {
            encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_EnableLowLatencyRateControl] = @YES;
        }
    }
    if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
        encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder] = @NO;
    } else if (_activeEncoderMode == XCWVideoEncoderModeH264Hardware) {
        encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder] = @YES;
    } else if (_activeEncoderMode == XCWVideoEncoderModeAuto && _realtimeStreamMode) {
        encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder] = @YES;
    }

    VTCompressionSessionRef session = NULL;
    OSStatus status = VTCompressionSessionCreate(kCFAllocatorDefault,
                                                 width,
                                                 height,
                                                 _codecType,
                                                 (__bridge CFDictionaryRef _Nullable)(encoderSpecification),
                                                 NULL,
                                                 NULL,
                                                 XCWH264EncoderOutputCallback,
                                                 (__bridge void *)self,
                                                 &session);
    _lastSessionStatus = status;
    if (status != noErr || session == NULL) {
        return NO;
    }

    _compressionSession = session;
    _width = width;
    _height = height;
    _timestampOriginUs = 0;
    _needsKeyFrame = YES;

    int expectedFrameRate = [self expectedFrameRateLocked];
    int averageBitRate = XCWAverageBitRateForDimensions(width, height, _activeEncoderMode, _lowLatencyMode, _realtimeStreamMode);

    VTSessionSetProperty(session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
    if (@available(macOS 10.14, *)) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_MaximizePowerEfficiency, kCFBooleanFalse);
    }
    if (_lowLatencyMode || _realtimeStreamMode) {
        XCWApplyCompressionPresetIfAvailable(session);
    }
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowTemporalCompression, kCFBooleanTrue);
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
    if (@available(macOS 10.14, *)) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowOpenGOP, kCFBooleanFalse);
    }
    if (@available(macOS 12.0, *)) {
        VTSessionSetProperty(session,
                             kVTCompressionPropertyKey_ProfileLevel,
                             kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel);
    } else {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_Baseline_AutoLevel);
    }
    VTSessionSetProperty(session, kVTCompressionPropertyKey_H264EntropyMode, kVTH264EntropyMode_CAVLC);
    VTSessionSetProperty(session, kVTCompressionPropertyKey_ExpectedFrameRate, (__bridge CFTypeRef)@(expectedFrameRate));
    BOOL shortKeyframeInterval = _lowLatencyMode;
    int keyFrameInterval = shortKeyframeInterval
        ? MAX(1, expectedFrameRate / 2)
        : MAX(1, expectedFrameRate * XCWRealtimeKeyFrameIntervalSeconds);
    double keyFrameIntervalDuration = shortKeyframeInterval ? 0.5 : (double)XCWRealtimeKeyFrameIntervalSeconds;
    VTSessionSetProperty(session, kVTCompressionPropertyKey_MaxKeyFrameInterval, (__bridge CFTypeRef)@(keyFrameInterval));
    VTSessionSetProperty(session, kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, (__bridge CFTypeRef)@(keyFrameIntervalDuration));
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AverageBitRate, (__bridge CFTypeRef)@(averageBitRate));
    if (_lowLatencyMode) {
        NSArray *dataRateLimits = @[
            @(MAX(1, averageBitRate / 8)),
            @1,
        ];
        VTSessionSetProperty(session, kVTCompressionPropertyKey_DataRateLimits, (__bridge CFTypeRef)dataRateLimits);
    }
    if (@available(macOS 11.0, *)) {
        VTSessionSetProperty(session,
                             kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality,
                             (_lowLatencyMode || _realtimeStreamMode) ? kCFBooleanTrue : kCFBooleanFalse);
    }
    if (@available(macOS 15.0, *)) {
        VTSessionSetProperty(session,
                             kVTCompressionPropertyKey_MaximumRealTimeFrameRate,
                             (__bridge CFTypeRef)@(expectedFrameRate));
    }
#ifdef kVTCompressionPropertyKey_MaxFrameDelayCount
    VTSessionSetProperty(session, kVTCompressionPropertyKey_MaxFrameDelayCount, (__bridge CFTypeRef)@0);
#endif

    status = VTCompressionSessionPrepareToEncodeFrames(session);
    _lastPrepareStatus = status;
    if (status != noErr) {
        [self invalidateCompressionSessionLocked];
        return NO;
    }
    _hardwareAccelerated = XCWCompressionSessionUsesHardwareEncoder(session);
    _selectedEncoderID = XCWCompressionSessionEncoderID(session);

    return YES;
}

- (void)invalidateCompressionSessionLocked {
    [self invalidateX264EncoderLocked];

    if (_compressionSession != NULL) {
        VTCompressionSessionInvalidate(_compressionSession);
        CFRelease(_compressionSession);
        _compressionSession = NULL;
    }
    _width = 0;
    _height = 0;
    _timestampOriginUs = 0;
    _inFlightFrameCount = 0;
    _lastSoftwareSubmissionUs = 0;
    _lastHardwareSubmissionUs = 0;
    _nextSoftwareSubmissionDueUs = 0;
    _nextHardwareSubmissionDueUs = 0;
    _latestEncodeLatencyUs = 0;
    _averageEncodeLatencyUs = 0;
    _peakEncodeLatencyUs = 0;
    _consecutiveOverBudgetFrameCount = 0;
    _consecutiveStrainedFrameCount = 0;
    _wasOverloaded = NO;
    _hardwareAccelerated = NO;
    _selectedEncoderID = nil;
    _scalingActive = NO;
    [self invalidateScalingResourcesLocked];
}

- (nullable CVPixelBufferRef)copyScaledPixelBufferIfNeeded:(CVPixelBufferRef)pixelBuffer
                                               targetWidth:(int32_t)targetWidth
                                              targetHeight:(int32_t)targetHeight {
    int32_t sourceWidth = (int32_t)CVPixelBufferGetWidth(pixelBuffer);
    int32_t sourceHeight = (int32_t)CVPixelBufferGetHeight(pixelBuffer);
    OSType sourcePixelFormat = CVPixelBufferGetPixelFormatType(pixelBuffer);
    if (sourceWidth == targetWidth && sourceHeight == targetHeight) {
        CVPixelBufferRetain(pixelBuffer);
        return pixelBuffer;
    }
    if ([self shouldUseSoftwareScalerForSourceWidth:sourceWidth
                                       sourceHeight:sourceHeight
                                        targetWidth:targetWidth
                                       targetHeight:targetHeight
                                        pixelFormat:sourcePixelFormat]) {
        CVPixelBufferRef scaledPixelBuffer = [self copySoftwareScaledPixelBuffer:pixelBuffer
                                                                     targetWidth:targetWidth
                                                                    targetHeight:targetHeight];
        if (scaledPixelBuffer != NULL) {
            return scaledPixelBuffer;
        }
    }

    if (_pixelTransferSession == NULL) {
        OSStatus sessionStatus = VTPixelTransferSessionCreate(kCFAllocatorDefault, &_pixelTransferSession);
        if (sessionStatus != noErr || _pixelTransferSession == NULL) {
            if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
                return [self copySoftwareScaledPixelBuffer:pixelBuffer
                                               targetWidth:targetWidth
                                              targetHeight:targetHeight];
            }
            return NULL;
        }
#ifdef kVTPixelTransferPropertyKey_RealTime
        if (@available(macOS 10.15, *)) {
            VTSessionSetProperty(_pixelTransferSession,
                                 kVTPixelTransferPropertyKey_RealTime,
                                 kCFBooleanTrue);
        }
#endif
        VTSessionSetProperty(_pixelTransferSession,
                             kVTPixelTransferPropertyKey_ScalingMode,
                             kVTScalingMode_Normal);
    }

    CVPixelBufferRef scaledPixelBuffer = [self copyPixelBufferFromScalingPoolWithWidth:targetWidth
                                                                                height:targetHeight
                                                                           pixelFormat:sourcePixelFormat];
    if (scaledPixelBuffer == NULL) {
        return NULL;
    }

    OSStatus transferStatus = VTPixelTransferSessionTransferImage(_pixelTransferSession,
                                                                  pixelBuffer,
                                                                  scaledPixelBuffer);
    _lastScaleStatus = transferStatus;
    if (transferStatus != noErr) {
        CVPixelBufferRelease(scaledPixelBuffer);
        if (_activeEncoderMode == XCWVideoEncoderModeH264Software) {
            return [self copySoftwareScaledPixelBuffer:pixelBuffer
                                           targetWidth:targetWidth
                                          targetHeight:targetHeight];
        }
        return NULL;
    }

    return scaledPixelBuffer;
}

- (BOOL)ensureX264EncoderWithWidth:(int32_t)width height:(int32_t)height {
    if (_x264Encoder != NULL && _x264Width == width && _x264Height == height) {
        return YES;
    }

    if (_compressionSession != NULL) {
        VTCompressionSessionInvalidate(_compressionSession);
        CFRelease(_compressionSession);
        _compressionSession = NULL;
        _inFlightFrameCount = 0;
        _lastHardwareSubmissionUs = 0;
    }
    [self invalidateX264EncoderLocked];

    x264_param_t parameters;
    if (x264_param_default_preset(&parameters, "ultrafast", "zerolatency") < 0) {
        _lastSessionStatus = -1;
        return NO;
    }

    int expectedFrameRate = MAX(1, [self expectedFrameRateLocked]);
    int averageBitRate = XCWAverageBitRateForDimensions(width,
                                                        height,
                                                        _activeEncoderMode,
                                                        _lowLatencyMode,
                                                        _realtimeStreamMode);
    parameters.i_width = width;
    parameters.i_height = height;
    parameters.i_csp = X264_CSP_NV12;
    parameters.i_fps_num = expectedFrameRate;
    parameters.i_fps_den = 1;
    parameters.i_timebase_num = 1;
    parameters.i_timebase_den = expectedFrameRate;
    parameters.i_keyint_max = MAX(1, expectedFrameRate * XCWRealtimeKeyFrameIntervalSeconds);
    parameters.i_keyint_min = parameters.i_keyint_max;
    parameters.b_intra_refresh = 0;
    parameters.b_repeat_headers = 1;
    parameters.b_annexb = 1;
    parameters.i_threads = 0;
    parameters.i_sync_lookahead = 0;
    parameters.i_log_level = X264_LOG_NONE;
    parameters.rc.i_rc_method = X264_RC_ABR;
    parameters.rc.i_bitrate = MAX(1, averageBitRate / 1000);
    parameters.rc.i_vbv_max_bitrate = parameters.rc.i_bitrate;
    parameters.rc.i_vbv_buffer_size = parameters.rc.i_bitrate;
    if (x264_param_apply_profile(&parameters, "baseline") < 0) {
        _lastSessionStatus = -2;
        return NO;
    }

    x264_t *encoder = x264_encoder_open(&parameters);
    if (encoder == NULL) {
        _lastSessionStatus = -3;
        return NO;
    }

    x264_picture_t picture;
    if (x264_picture_alloc(&picture, X264_CSP_NV12, width, height) < 0) {
        x264_encoder_close(encoder);
        _lastSessionStatus = -4;
        return NO;
    }

    _x264Encoder = encoder;
    _x264Picture = picture;
    _x264PictureAllocated = YES;
    _x264Width = width;
    _x264Height = height;
    _x264FrameIndex = 0;
    _x264ColorConversionReady = NO;
    _width = width;
    _height = height;
    _hardwareAccelerated = NO;
    _selectedEncoderID = @"org.videolan.x264";
    _x264Codec = @"avc1.42e01f";
    _lastSessionStatus = 0;
    _lastPrepareStatus = 0;
    return YES;
}

- (void)invalidateX264EncoderLocked {
    if (_x264PictureAllocated) {
        x264_picture_clean(&_x264Picture);
        _x264PictureAllocated = NO;
    }
    if (_x264Encoder != NULL) {
        x264_encoder_close(_x264Encoder);
        _x264Encoder = NULL;
    }
    _x264Width = 0;
    _x264Height = 0;
    _x264FrameIndex = 0;
    _x264ColorConversionReady = NO;
    _x264Codec = nil;
}

- (BOOL)ensureX264ColorConversionLocked {
    if (_x264ColorConversionReady) {
        return YES;
    }

    vImage_YpCbCrPixelRange pixelRange = {
        .Yp_bias = 16,
        .CbCr_bias = 128,
        .YpRangeMax = 235,
        .CbCrRangeMax = 240,
        .YpMax = 255,
        .YpMin = 0,
        .CbCrMax = 255,
        .CbCrMin = 0,
    };
    vImage_Error status =
        vImageConvert_ARGBToYpCbCr_GenerateConversion(kvImage_ARGBToYpCbCrMatrix_ITU_R_709_2,
                                                      &pixelRange,
                                                      &_x264ColorConversion,
                                                      kvImageARGB8888,
                                                      kvImage420Yp8_CbCr8,
                                                      kvImageNoFlags);
    _lastScaleStatus = status;
    _x264ColorConversionReady = status == kvImageNoError;
    return _x264ColorConversionReady;
}

- (BOOL)copyPixelBufferIntoX264PictureLocked:(CVPixelBufferRef)pixelBuffer {
    if (_x264Encoder == NULL || !_x264PictureAllocated) {
        return NO;
    }

    int32_t width = (int32_t)CVPixelBufferGetWidth(pixelBuffer);
    int32_t height = (int32_t)CVPixelBufferGetHeight(pixelBuffer);
    if (width != _x264Width || height != _x264Height || (width % 2) != 0 || (height % 2) != 0) {
        _lastScaleStatus = -10;
        return NO;
    }

    OSType pixelFormat = CVPixelBufferGetPixelFormatType(pixelBuffer);
    CVReturn lockStatus = CVPixelBufferLockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
    if (lockStatus != kCVReturnSuccess) {
        _lastScaleStatus = lockStatus;
        return NO;
    }

    BOOL copied = NO;
    if (pixelFormat == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange ||
        pixelFormat == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange) {
        const uint8_t *sourceY = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 0);
        const uint8_t *sourceCbCr = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 1);
        if (sourceY != NULL && sourceCbCr != NULL) {
            XCWCopyPlaneRows(_x264Picture.img.plane[0],
                             (size_t)_x264Picture.img.i_stride[0],
                             sourceY,
                             CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 0),
                             (size_t)width,
                             (size_t)height);
            XCWCopyPlaneRows(_x264Picture.img.plane[1],
                             (size_t)_x264Picture.img.i_stride[1],
                             sourceCbCr,
                             CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 1),
                             (size_t)width,
                             (size_t)(height / 2));
            copied = YES;
            _lastScaleStatus = 0;
        }
    } else if (pixelFormat == kCVPixelFormatType_32BGRA && [self ensureX264ColorConversionLocked]) {
        vImage_Buffer sourceBuffer = {
            .data = CVPixelBufferGetBaseAddress(pixelBuffer),
            .height = (vImagePixelCount)height,
            .width = (vImagePixelCount)width,
            .rowBytes = CVPixelBufferGetBytesPerRow(pixelBuffer),
        };
        vImage_Buffer destinationY = {
            .data = _x264Picture.img.plane[0],
            .height = (vImagePixelCount)height,
            .width = (vImagePixelCount)width,
            .rowBytes = (size_t)_x264Picture.img.i_stride[0],
        };
        vImage_Buffer destinationCbCr = {
            .data = _x264Picture.img.plane[1],
            .height = (vImagePixelCount)(height / 2),
            .width = (vImagePixelCount)(width / 2),
            .rowBytes = (size_t)_x264Picture.img.i_stride[1],
        };
        const uint8_t bgraPermuteMap[4] = {3, 2, 1, 0};
        vImage_Error status =
            vImageConvert_ARGB8888To420Yp8_CbCr8(&sourceBuffer,
                                                 &destinationY,
                                                 &destinationCbCr,
                                                 &_x264ColorConversion,
                                                 bgraPermuteMap,
                                                 kvImageNoFlags);
        _lastScaleStatus = status;
        copied = status == kvImageNoError;
    } else {
        _lastScaleStatus = -11;
    }

    CVPixelBufferUnlockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
    return copied;
}

- (void)recordEncodeLatencyLockedWithSubmittedAtUs:(uint64_t)submittedAtUs measuredAtUs:(uint64_t)measuredAtUs {
    if (submittedAtUs == 0 || measuredAtUs == 0) {
        return;
    }
    _latestEncodeLatencyUs = measuredAtUs >= submittedAtUs ? measuredAtUs - submittedAtUs : 0;
    _peakEncodeLatencyUs = MAX(_peakEncodeLatencyUs, _latestEncodeLatencyUs);
    _averageEncodeLatencyUs = _averageEncodeLatencyUs <= 0.0
        ? (double)_latestEncodeLatencyUs
        : (_averageEncodeLatencyUs * (1.0 - XCWEncoderLatencyEWMAAlpha)) + ((double)_latestEncodeLatencyUs * XCWEncoderLatencyEWMAAlpha);
    uint64_t encoderBudgetUs = [self encoderLatencyBudgetUsLocked];
    double averageLoadPercent = encoderBudgetUs > 0
        ? (_averageEncodeLatencyUs * 100.0) / (double)encoderBudgetUs
        : 0.0;
    double latestLoadPercent = encoderBudgetUs > 0
        ? ((double)_latestEncodeLatencyUs * 100.0) / (double)encoderBudgetUs
        : 0.0;
    if (encoderBudgetUs > 0 && _latestEncodeLatencyUs > encoderBudgetUs) {
        _overBudgetFrameCount += 1;
        _consecutiveOverBudgetFrameCount += 1;
    } else {
        _consecutiveOverBudgetFrameCount = 0;
    }
    if (latestLoadPercent >= XCWEncoderStrainedLoadPercent) {
        _consecutiveStrainedFrameCount += 1;
    } else {
        _consecutiveStrainedFrameCount = 0;
    }
    BOOL overloaded = averageLoadPercent >= XCWEncoderOverloadedLoadPercent ||
        _consecutiveOverBudgetFrameCount >= XCWEncoderConsecutiveOverBudgetFrameThreshold;
    if (overloaded && !_wasOverloaded) {
        _overloadEventCount += 1;
    }
    _wasOverloaded = overloaded;
    [self adaptSoftwarePacingForLatencyUs:_latestEncodeLatencyUs];
}

- (BOOL)encodePixelBufferWithX264Locked:(CVPixelBufferRef)pixelBuffer
                             targetWidth:(int32_t)targetWidth
                            targetHeight:(int32_t)targetHeight
                                  timeUs:(uint64_t)nowUs
                      relativeTimestampUs:(uint64_t)relativeTimestampUs
                           forceKeyFrame:(BOOL)forceKeyFrame {
    if (![self ensureX264EncoderWithWidth:targetWidth height:targetHeight]) {
        _encodeFailureCount += 1;
        _needsKeyFrame = YES;
        return NO;
    }
    if (![self copyPixelBufferIntoX264PictureLocked:pixelBuffer]) {
        _encodeFailureCount += 1;
        _needsKeyFrame = YES;
        return NO;
    }

    _x264Picture.i_pts = (int64_t)_x264FrameIndex++;
    _x264Picture.i_type = forceKeyFrame ? X264_TYPE_IDR : X264_TYPE_AUTO;

    x264_nal_t *nals = NULL;
    int nalCount = 0;
    x264_picture_t outputPicture;
    _submittedFrameCount += 1;
    [self recordFrameSubmissionLockedAtTimeUs:nowUs software:YES];
    int frameBytes = x264_encoder_encode(_x264Encoder,
                                         &nals,
                                         &nalCount,
                                         &_x264Picture,
                                         &outputPicture);
    uint64_t measuredAtUs = (uint64_t)(CACurrentMediaTime() * 1000000.0);
    if (frameBytes < 0) {
        _lastEncodeStatus = frameBytes;
        _encodeFailureCount += 1;
        _needsKeyFrame = YES;
        return NO;
    }
    _lastEncodeStatus = 0;
    [self recordEncodeLatencyLockedWithSubmittedAtUs:nowUs measuredAtUs:measuredAtUs];
    if (frameBytes == 0 || nalCount <= 0 || nals == NULL) {
        return YES;
    }

    NSMutableData *sampleData = [NSMutableData dataWithCapacity:(NSUInteger)frameBytes];
    for (int index = 0; index < nalCount; index++) {
        if (nals[index].p_payload != NULL && nals[index].i_payload > 0) {
            [sampleData appendBytes:nals[index].p_payload length:(NSUInteger)nals[index].i_payload];
        }
    }
    if (sampleData.length == 0) {
        return YES;
    }

    BOOL isKeyFrame = outputPicture.b_keyframe != 0;
    if (isKeyFrame) {
        _x264Codec = XCWCodecStringFromAnnexBSample(sampleData);
        _keyFrameOutputCount += 1;
    }
    _outputFrameCount += 1;
    CGSize dimensions = CGSizeMake(_x264Width, _x264Height);
    self.outputHandler(sampleData,
                       relativeTimestampUs,
                       isKeyFrame,
                       _x264Codec ?: @"avc1.42e01f",
                       nil,
                       dimensions);
    return YES;
}

- (BOOL)shouldUseSoftwareScalerForSourceWidth:(int32_t)sourceWidth
                                  sourceHeight:(int32_t)sourceHeight
                                   targetWidth:(int32_t)targetWidth
                                  targetHeight:(int32_t)targetHeight
                                   pixelFormat:(OSType)pixelFormat {
    if (!_realtimeStreamMode || !XCWPixelFormatSupportsSoftwareScaling(pixelFormat)) {
        return NO;
    }
    if (sourceWidth == targetWidth && sourceHeight == targetHeight) {
        return NO;
    }
    return _activeEncoderMode == XCWVideoEncoderModeAuto || _activeEncoderMode == XCWVideoEncoderModeH264Hardware;
}

- (nullable CVPixelBufferRef)copySoftwareScaledPixelBuffer:(CVPixelBufferRef)pixelBuffer
                                               targetWidth:(int32_t)targetWidth
                                              targetHeight:(int32_t)targetHeight {
    OSType sourcePixelFormat = CVPixelBufferGetPixelFormatType(pixelBuffer);
    if (!XCWPixelFormatSupportsSoftwareScaling(sourcePixelFormat)) {
        _lastScaleStatus = -1;
        return NULL;
    }

    CVPixelBufferRef scaledPixelBuffer = [self copyPixelBufferFromScalingPoolWithWidth:targetWidth
                                                                                height:targetHeight
                                                                           pixelFormat:sourcePixelFormat];
    if (scaledPixelBuffer == NULL) {
        return NULL;
    }

    CVReturn sourceLockStatus = CVPixelBufferLockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
    if (sourceLockStatus != kCVReturnSuccess) {
        CVPixelBufferRelease(scaledPixelBuffer);
        _lastScaleStatus = sourceLockStatus;
        return NULL;
    }

    CVReturn targetLockStatus = CVPixelBufferLockBaseAddress(scaledPixelBuffer, 0);
    if (targetLockStatus != kCVReturnSuccess) {
        CVPixelBufferUnlockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
        CVPixelBufferRelease(scaledPixelBuffer);
        _lastScaleStatus = targetLockStatus;
        return NULL;
    }

    vImage_Buffer sourceBuffer = {
        .data = CVPixelBufferGetBaseAddress(pixelBuffer),
        .height = (vImagePixelCount)CVPixelBufferGetHeight(pixelBuffer),
        .width = (vImagePixelCount)CVPixelBufferGetWidth(pixelBuffer),
        .rowBytes = CVPixelBufferGetBytesPerRow(pixelBuffer),
    };
    vImage_Buffer targetBuffer = {
        .data = CVPixelBufferGetBaseAddress(scaledPixelBuffer),
        .height = (vImagePixelCount)CVPixelBufferGetHeight(scaledPixelBuffer),
        .width = (vImagePixelCount)CVPixelBufferGetWidth(scaledPixelBuffer),
        .rowBytes = CVPixelBufferGetBytesPerRow(scaledPixelBuffer),
    };
    vImage_Flags scaleFlags = _realtimeStreamMode ? kvImageNoFlags : kvImageHighQualityResampling;
    vImage_Error scaleStatus = vImageScale_ARGB8888(&sourceBuffer,
                                                    &targetBuffer,
                                                    NULL,
                                                    scaleFlags);
    CVPixelBufferUnlockBaseAddress(scaledPixelBuffer, 0);
    CVPixelBufferUnlockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
    _lastScaleStatus = scaleStatus;
    if (scaleStatus != kvImageNoError) {
        CVPixelBufferRelease(scaledPixelBuffer);
        return NULL;
    }

    return scaledPixelBuffer;
}

- (nullable CVPixelBufferRef)copyPixelBufferFromScalingPoolWithWidth:(int32_t)targetWidth
                                                              height:(int32_t)targetHeight
                                                         pixelFormat:(OSType)pixelFormat {
    BOOL needsNewPool = (_scaledPixelBufferPool == NULL)
        || (_scaledPixelBufferWidth != targetWidth)
        || (_scaledPixelBufferHeight != targetHeight)
        || (_scaledPixelFormat != pixelFormat);
    if (needsNewPool) {
        if (_scaledPixelBufferPool != NULL) {
            CVPixelBufferPoolRelease(_scaledPixelBufferPool);
            _scaledPixelBufferPool = NULL;
        }

        NSDictionary *attributes = @{
            (__bridge NSString *)kCVPixelBufferPixelFormatTypeKey: @(pixelFormat),
            (__bridge NSString *)kCVPixelBufferWidthKey: @(targetWidth),
            (__bridge NSString *)kCVPixelBufferHeightKey: @(targetHeight),
            (__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey: @{},
        };
        CVPixelBufferPoolRef pool = NULL;
        CVReturn poolStatus = CVPixelBufferPoolCreate(kCFAllocatorDefault,
                                                      NULL,
                                                      (__bridge CFDictionaryRef)attributes,
                                                      &pool);
        if (poolStatus != kCVReturnSuccess || pool == NULL) {
            _lastScaleStatus = poolStatus;
            return NULL;
        }
        _scaledPixelBufferPool = pool;
        _scaledPixelBufferWidth = targetWidth;
        _scaledPixelBufferHeight = targetHeight;
        _scaledPixelFormat = pixelFormat;
    }

    CVPixelBufferRef scaledPixelBuffer = NULL;
    CVReturn bufferStatus = CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault,
                                                              _scaledPixelBufferPool,
                                                              &scaledPixelBuffer);
    if (bufferStatus != kCVReturnSuccess || scaledPixelBuffer == NULL) {
        _lastScaleStatus = bufferStatus;
        return NULL;
    }
    return scaledPixelBuffer;
}

- (void)invalidateScalingResourcesLocked {
    if (_scaledPixelBufferPool != NULL) {
        CVPixelBufferPoolRelease(_scaledPixelBufferPool);
        _scaledPixelBufferPool = NULL;
    }
    _scaledPixelBufferWidth = 0;
    _scaledPixelBufferHeight = 0;
    _scaledPixelFormat = 0;
    if (_pixelTransferSession != NULL) {
        VTPixelTransferSessionInvalidate(_pixelTransferSession);
        CFRelease(_pixelTransferSession);
        _pixelTransferSession = NULL;
    }
}

- (void)handleEncodedSampleBuffer:(CMSampleBufferRef)sampleBuffer
                    submittedAtUs:(uint64_t)submittedAtUs {
    if (sampleBuffer == NULL || !CMSampleBufferDataIsReady(sampleBuffer)) {
        return;
    }

    NSData *sampleData = XCWCopySampleData(sampleBuffer);
    if (sampleData.length == 0) {
        return;
    }

    BOOL isKeyFrame = XCWSampleBufferIsKeyFrame(sampleBuffer);
    _outputFrameCount += 1;
    if (isKeyFrame) {
        _keyFrameOutputCount += 1;
    }
    BOOL shouldEnterAutoSoftwareFallback = NO;
    uint64_t measurementTimeUs = 0;
    if (submittedAtUs > 0) {
        uint64_t nowUs = (uint64_t)(CACurrentMediaTime() * 1000000.0);
        measurementTimeUs = nowUs;
        _latestEncodeLatencyUs = nowUs >= submittedAtUs ? nowUs - submittedAtUs : 0;
        _peakEncodeLatencyUs = MAX(_peakEncodeLatencyUs, _latestEncodeLatencyUs);
        _averageEncodeLatencyUs = _averageEncodeLatencyUs <= 0.0
            ? (double)_latestEncodeLatencyUs
            : (_averageEncodeLatencyUs * (1.0 - XCWEncoderLatencyEWMAAlpha)) + ((double)_latestEncodeLatencyUs * XCWEncoderLatencyEWMAAlpha);
        uint64_t encoderBudgetUs = [self encoderLatencyBudgetUsLocked];
        double averageLoadPercent = encoderBudgetUs > 0
            ? (_averageEncodeLatencyUs * 100.0) / (double)encoderBudgetUs
            : 0.0;
        double latestLoadPercent = encoderBudgetUs > 0
            ? ((double)_latestEncodeLatencyUs * 100.0) / (double)encoderBudgetUs
            : 0.0;
        if (encoderBudgetUs > 0 && _latestEncodeLatencyUs > encoderBudgetUs) {
            _overBudgetFrameCount += 1;
            _consecutiveOverBudgetFrameCount += 1;
        } else {
            _consecutiveOverBudgetFrameCount = 0;
        }
        if (latestLoadPercent >= XCWEncoderStrainedLoadPercent) {
            _consecutiveStrainedFrameCount += 1;
        } else {
            _consecutiveStrainedFrameCount = 0;
        }
        BOOL overloaded = averageLoadPercent >= XCWEncoderOverloadedLoadPercent ||
            _consecutiveOverBudgetFrameCount >= XCWEncoderConsecutiveOverBudgetFrameThreshold;
        if (overloaded && !_wasOverloaded) {
            _overloadEventCount += 1;
        }
        _wasOverloaded = overloaded;
        BOOL hardwareFallbackOverloaded = averageLoadPercent >= XCWHardwareFallbackLoadPercent ||
            _consecutiveOverBudgetFrameCount >= XCWHardwareFallbackConsecutiveOverBudgetFrameThreshold;
        shouldEnterAutoSoftwareFallback = hardwareFallbackOverloaded &&
            _encoderMode == XCWVideoEncoderModeAuto &&
            _activeEncoderMode != XCWVideoEncoderModeH264Software &&
            _hardwareAccelerated;
        [self adaptSoftwarePacingForLatencyUs:_latestEncodeLatencyUs];
    }
    NSString *codec = nil;
    NSData *decoderConfig = nil;

    if (isKeyFrame) {
        CMFormatDescriptionRef formatDescription = CMSampleBufferGetFormatDescription(sampleBuffer);
        if (formatDescription != NULL) {
            CMVideoCodecType mediaSubType = CMFormatDescriptionGetMediaSubType(formatDescription);
            if (mediaSubType == kCMVideoCodecType_H264) {
                decoderConfig = XCWDecoderConfigurationRecordFromFormatDescription(formatDescription, @"avcC");
                if (decoderConfig.length >= 4) {
                    const uint8_t *bytes = decoderConfig.bytes;
                    codec = [NSString stringWithFormat:@"avc1.%02x%02x%02x", bytes[1], bytes[2], bytes[3]];
                }
                if (decoderConfig.length == 0) {
                    const uint8_t *spsBytes = NULL;
                    size_t spsLength = 0;
                    const uint8_t *ppsBytes = NULL;
                    size_t ppsLength = 0;
                    size_t parameterSetCount = 0;
                    int nalLengthHeader = 0;

                    OSStatus spsStatus = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(formatDescription,
                                                                                            0,
                                                                                            &spsBytes,
                                                                                            &spsLength,
                                                                                            &parameterSetCount,
                                                                                            &nalLengthHeader);
                    OSStatus ppsStatus = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(formatDescription,
                                                                                            1,
                                                                                            &ppsBytes,
                                                                                            &ppsLength,
                                                                                            &parameterSetCount,
                                                                                            &nalLengthHeader);
                    if (spsStatus == noErr && ppsStatus == noErr && spsLength > 0 && ppsLength > 0) {
                        NSData *spsData = [NSData dataWithBytes:spsBytes length:spsLength];
                        NSData *ppsData = [NSData dataWithBytes:ppsBytes length:ppsLength];
                        codec = XCWCodecStringFromSPS(spsData);
                        decoderConfig = XCWAVCDecoderConfigurationRecord(spsData, ppsData, nalLengthHeader);
                    }
                }
            }
        }
    }

    CMTime presentationTime = CMSampleBufferGetPresentationTimeStamp(sampleBuffer);
    uint64_t timestampUs = 0;
    if (presentationTime.timescale > 0) {
        timestampUs = (uint64_t)llround(CMTimeGetSeconds(presentationTime) * 1000000.0);
    }

    CGSize dimensions = CGSizeMake(_width, _height);
    self.outputHandler(sampleData, timestampUs, isKeyFrame, codec, decoderConfig, dimensions);
    if (shouldEnterAutoSoftwareFallback) {
        [self enterAutoSoftwareFallbackLockedAtTimeUs:measurementTimeUs];
    }
}

- (void)completeInFlightFrame {
    dispatch_async(_queue, ^{
        if (self->_inFlightFrameCount > 0) {
            self->_inFlightFrameCount -= 1;
        }
        [self drainPendingFramesLocked];
    });
}

- (void)completeEncodedSampleBufferLocked:(CMSampleBufferRef)sampleBuffer
                            submittedAtUs:(uint64_t)submittedAtUs
                              shouldDrain:(BOOL)shouldDrain {
    [self handleEncodedSampleBuffer:sampleBuffer submittedAtUs:submittedAtUs];
    if (_inFlightFrameCount > 0) {
        _inFlightFrameCount -= 1;
    }
    if (shouldDrain) {
        [self drainPendingFramesLocked];
    }
}

- (void)completeFailedFrame {
    dispatch_async(_queue, ^{
        self->_encodeFailureCount += 1;
        if (self->_inFlightFrameCount > 0) {
            self->_inFlightFrameCount -= 1;
        }
        [self drainPendingFramesLocked];
    });
}

- (void)handleCompressionOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
                              submittedAtUs:(uint64_t)submittedAtUs {
    if (sampleBuffer == NULL) {
        [self completeFailedFrame];
        return;
    }

    if (dispatch_get_specific(XCWH264EncoderQueueSpecificKey) == XCWH264EncoderQueueSpecificKey) {
        [self completeEncodedSampleBufferLocked:sampleBuffer
                                  submittedAtUs:submittedAtUs
                                    shouldDrain:!_drainingPendingFrames];
        return;
    }

    CFRetain(sampleBuffer);
    dispatch_async(_queue, ^{
        [self completeEncodedSampleBufferLocked:sampleBuffer
                                  submittedAtUs:submittedAtUs
                                    shouldDrain:YES];
        CFRelease(sampleBuffer);
    });
}

@end

static void XCWH264EncoderOutputCallback(void *outputCallbackRefCon,
                                         void *sourceFrameRefCon,
                                         OSStatus status,
                                         __unused VTEncodeInfoFlags infoFlags,
                                         CMSampleBufferRef sampleBuffer) {
    if (status != noErr || sampleBuffer == NULL) {
        XCWH264Encoder *encoder = (__bridge XCWH264Encoder *)outputCallbackRefCon;
        [encoder completeFailedFrame];
        return;
    }

    XCWH264Encoder *encoder = (__bridge XCWH264Encoder *)outputCallbackRefCon;
    [encoder handleCompressionOutputSampleBuffer:sampleBuffer
                                   submittedAtUs:(uint64_t)(uintptr_t)sourceFrameRefCon];
}
