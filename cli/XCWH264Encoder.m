#import "XCWH264Encoder.h"

#import <CoreMedia/CoreMedia.h>
#import <os/lock.h>
#import <QuartzCore/QuartzCore.h>
#import <VideoToolbox/VideoToolbox.h>
#include <stdlib.h>

static const int32_t XCWMaximumEncodedDimension = 1920;
static const int32_t XCWMaximumSoftwareEncodedDimension = 1600;
static const int32_t XCWTargetRealTimeFrameRate = 60;
static const int32_t XCWTargetSoftwareFrameRate = 60;
static const NSUInteger XCWMaximumInFlightFrames = 2;
static const int32_t XCWMinimumAverageBitRate = 18000000;
static const int32_t XCWMinimumSoftwareAverageBitRate = 3000000;
static const int64_t XCWBitsPerPixelBudget = 10;
static const int64_t XCWSoftwareBitsPerPixelBudget = 6;
static const uint64_t XCWSoftwareMinimumFrameIntervalUs = 16667;
static const uint64_t XCWSoftwareInitialFrameIntervalUs = 16667;
static const uint64_t XCWSoftwareMaximumFrameIntervalUs = 50000;
static const uint64_t XCWSoftwareFrameIntervalStepUs = 5556;
static const NSUInteger XCWSoftwareHealthyFrameWindow = 4;

typedef NS_ENUM(NSUInteger, XCWVideoEncoderMode) {
    XCWVideoEncoderModeHEVCHardware,
    XCWVideoEncoderModeH264Hardware,
    XCWVideoEncoderModeH264Software,
};

static XCWVideoEncoderMode XCWVideoEncoderModeFromEnvironment(void) {
    const char *rawValue = getenv("SIMDECK_VIDEO_CODEC");
    NSString *value = rawValue != NULL ? [[[NSString alloc] initWithUTF8String:rawValue] lowercaseString] : @"";
    if ([value isEqualToString:@"h264"] || [value isEqualToString:@"h264-hardware"] || [value isEqualToString:@"avc"]) {
        return XCWVideoEncoderModeH264Hardware;
    }
    if ([value isEqualToString:@"h264-software"] || [value isEqualToString:@"software-h264"]) {
        return XCWVideoEncoderModeH264Software;
    }
    return XCWVideoEncoderModeHEVCHardware;
}

static CMVideoCodecType XCWVideoCodecTypeForMode(XCWVideoEncoderMode mode) {
    switch (mode) {
        case XCWVideoEncoderModeH264Hardware:
        case XCWVideoEncoderModeH264Software:
            return kCMVideoCodecType_H264;
        case XCWVideoEncoderModeHEVCHardware:
        default:
            return kCMVideoCodecType_HEVC;
    }
}

static NSString *XCWVideoEncoderModeName(XCWVideoEncoderMode mode) {
    switch (mode) {
        case XCWVideoEncoderModeH264Hardware:
            return @"h264";
        case XCWVideoEncoderModeH264Software:
            return @"h264-software";
        case XCWVideoEncoderModeHEVCHardware:
        default:
            return @"hevc";
    }
}

static NSString *XCWVideoEncoderIDForMode(XCWVideoEncoderMode mode) {
    switch (mode) {
        case XCWVideoEncoderModeH264Hardware:
            return nil;
        case XCWVideoEncoderModeH264Software:
            return @"com.apple.videotoolbox.videoencoder.h264";
        case XCWVideoEncoderModeHEVCHardware:
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

static uint32_t XCWReverseBits32(uint32_t value) {
    value = ((value & 0x55555555u) << 1) | ((value >> 1) & 0x55555555u);
    value = ((value & 0x33333333u) << 2) | ((value >> 2) & 0x33333333u);
    value = ((value & 0x0f0f0f0fu) << 4) | ((value >> 4) & 0x0f0f0f0fu);
    value = ((value & 0x00ff00ffu) << 8) | ((value >> 8) & 0x00ff00ffu);
    return (value << 16) | (value >> 16);
}

static NSString *XCWCodecStringFromHVCC(NSData *hvcCData) {
    const uint8_t *bytes = hvcCData.bytes;
    if (bytes == NULL || hvcCData.length < 13) {
        return @"hvc1.1.6.L123.B0";
    }

    static NSString *const profileSpaces[] = { @"", @"A", @"B", @"C" };
    uint8_t profileByte = bytes[1];
    uint8_t profileSpace = (profileByte >> 6) & 0x03;
    BOOL tierFlag = (profileByte & 0x20) != 0;
    uint8_t profileIdc = profileByte & 0x1f;
    uint32_t rawCompatibility = ((uint32_t)bytes[2] << 24)
        | ((uint32_t)bytes[3] << 16)
        | ((uint32_t)bytes[4] << 8)
        | (uint32_t)bytes[5];
    uint32_t compatibility = XCWReverseBits32(rawCompatibility);
    uint8_t levelIdc = bytes[12];

    NSMutableArray<NSString *> *constraintParts = [NSMutableArray array];
    NSInteger lastConstraintIndex = 11;
    while (lastConstraintIndex >= 6 && bytes[lastConstraintIndex] == 0) {
        lastConstraintIndex -= 1;
    }
    for (NSInteger index = 6; index <= lastConstraintIndex; index += 1) {
        [constraintParts addObject:[NSString stringWithFormat:@"%02X", bytes[index]]];
    }
    if (constraintParts.count == 0) {
        [constraintParts addObject:@"00"];
    }

    return [NSString stringWithFormat:@"hvc1.%@%u.%x.%@%u.%@",
                                      profileSpaces[profileSpace],
                                      profileIdc,
                                      compatibility,
                                      tierFlag ? @"H" : @"L",
                                      levelIdc,
                                      [constraintParts componentsJoinedByString:@"."]];
}

static NSData *XCWAVCDecoderConfigurationRecord(NSData *spsData, NSData *ppsData) {
    if (spsData.length == 0 || ppsData.length == 0) {
        return nil;
    }

    const uint8_t *spsBytes = spsData.bytes;
    NSMutableData *record = [NSMutableData data];
    uint8_t header[6] = {
        0x01,
        spsBytes[1],
        spsBytes[2],
        spsBytes[3],
        0xFF,
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
        case kCMVideoCodecType_HEVC:
            return @"hevc";
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

static CGSize XCWScaledDimensionsForSourceSize(int32_t width, int32_t height, XCWVideoEncoderMode mode) {
    if (width <= 0 || height <= 0) {
        return CGSizeZero;
    }

    int32_t maximumDimension = mode == XCWVideoEncoderModeH264Software
        ? XCWMaximumSoftwareEncodedDimension
        : XCWMaximumEncodedDimension;
    int32_t longestEdge = MAX(width, height);
    if (longestEdge <= maximumDimension) {
        return CGSizeMake(width, height);
    }

    double scale = (double)maximumDimension / (double)longestEdge;
    return CGSizeMake(XCWRoundToEvenDimension(width * scale),
                      XCWRoundToEvenDimension(height * scale));
}

static int32_t XCWAverageBitRateForDimensions(int32_t width, int32_t height, XCWVideoEncoderMode mode) {
    int64_t bitsPerPixelBudget = mode == XCWVideoEncoderModeH264Software
        ? XCWSoftwareBitsPerPixelBudget
        : XCWBitsPerPixelBudget;
    int64_t minimumAverageBitRate = mode == XCWVideoEncoderModeH264Software
        ? XCWMinimumSoftwareAverageBitRate
        : XCWMinimumAverageBitRate;
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

static void XCWH264EncoderOutputCallback(void *outputCallbackRefCon,
                                         void *sourceFrameRefCon,
                                         OSStatus status,
                                         VTEncodeInfoFlags infoFlags,
                                         CMSampleBufferRef sampleBuffer);

@interface XCWH264Encoder ()

@property (nonatomic, copy, readonly) XCWH264EncoderOutputHandler outputHandler;

@end

@implementation XCWH264Encoder {
    dispatch_queue_t _queue;
    VTCompressionSessionRef _compressionSession;
    os_unfair_lock _pendingLock;
    CVPixelBufferRef _pendingPixelBuffer;
    BOOL _drainScheduled;
    BOOL _needsKeyFrame;
    NSUInteger _inFlightFrameCount;
    int32_t _width;
    int32_t _height;
    uint64_t _timestampOriginUs;
    VTPixelTransferSessionRef _pixelTransferSession;
    CVPixelBufferRef _scaledPixelBuffer;
    OSType _scaledPixelFormat;
    XCWVideoEncoderMode _encoderMode;
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
    uint64_t _softwareFrameIntervalUs;
    uint64_t _lastSoftwareSubmissionUs;
    NSUInteger _softwarePacedFrameCount;
    NSUInteger _softwareHealthyFrameCount;
    NSInteger _lastSessionStatus;
    NSInteger _lastPrepareStatus;
    NSInteger _lastScaleStatus;
    NSInteger _lastEncodeStatus;
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
    _pendingLock = OS_UNFAIR_LOCK_INIT;
    _needsKeyFrame = YES;
    _encoderMode = XCWVideoEncoderModeFromEnvironment();
    _codecType = XCWVideoCodecTypeForMode(_encoderMode);
    _softwareFrameIntervalUs = XCWSoftwareInitialFrameIntervalUs;
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

- (NSDictionary *)statsRepresentation {
    __block NSUInteger inputFrameCount = 0;
    __block NSUInteger pendingReplacementCount = 0;
    os_unfair_lock_lock(&_pendingLock);
    inputFrameCount = _inputFrameCount;
    pendingReplacementCount = _pendingReplacementCount;
    os_unfair_lock_unlock(&_pendingLock);

    __block NSDictionary *stats = nil;
    dispatch_sync(_queue, ^{
        stats = @{
            @"inputFrames": @(inputFrameCount),
            @"pendingReplacements": @(pendingReplacementCount),
            @"submittedFrames": @(self->_submittedFrameCount),
            @"encodeFailures": @(self->_encodeFailureCount),
            @"outputFrames": @(self->_outputFrameCount),
            @"keyFrameOutputs": @(self->_keyFrameOutputCount),
            @"inFlightFrames": @(self->_inFlightFrameCount),
            @"maxInFlightFrames": @(self->_maxInFlightFrameCount),
            @"latestEncodeLatencyUs": @(self->_latestEncodeLatencyUs),
            @"softwareFrameIntervalUs": @(self->_softwareFrameIntervalUs),
            @"softwareTargetFps": @(self->_softwareFrameIntervalUs > 0 ? (1000000.0 / (double)self->_softwareFrameIntervalUs) : 0.0),
            @"softwarePacedFrames": @(self->_softwarePacedFrameCount),
            @"transportCodec": XCWCodecName(self->_codecType),
            @"encoderMode": XCWVideoEncoderModeName(self->_encoderMode),
            @"encoderId": XCWVideoEncoderIDForMode(self->_encoderMode) ?: @"automatic",
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

- (BOOL)shouldPaceSoftwareFrameAtTimeUs:(uint64_t)nowUs {
    if (_encoderMode != XCWVideoEncoderModeH264Software || _needsKeyFrame) {
        return NO;
    }
    if (_softwareFrameIntervalUs == 0) {
        _softwareFrameIntervalUs = XCWSoftwareInitialFrameIntervalUs;
    }
    if (_lastSoftwareSubmissionUs == 0) {
        return NO;
    }
    uint64_t elapsedUs = nowUs >= _lastSoftwareSubmissionUs ? nowUs - _lastSoftwareSubmissionUs : 0;
    if (elapsedUs >= _softwareFrameIntervalUs) {
        return NO;
    }
    _softwarePacedFrameCount += 1;
    return YES;
}

- (void)adaptSoftwarePacingForLatencyUs:(uint64_t)latencyUs {
    if (_encoderMode != XCWVideoEncoderModeH264Software || latencyUs == 0) {
        return;
    }
    if (_softwareFrameIntervalUs == 0) {
        _softwareFrameIntervalUs = XCWSoftwareInitialFrameIntervalUs;
    }

    uint64_t highLatencyBudgetUs = (_softwareFrameIntervalUs * 3) / 2;
    if (latencyUs > highLatencyBudgetUs) {
        uint64_t nextIntervalUs = _softwareFrameIntervalUs + XCWSoftwareFrameIntervalStepUs;
        uint64_t latencyBoundIntervalUs = latencyUs + XCWSoftwareFrameIntervalStepUs;
        if (nextIntervalUs < latencyBoundIntervalUs) {
            nextIntervalUs = latencyBoundIntervalUs;
        }
        _softwareFrameIntervalUs = MIN(nextIntervalUs, XCWSoftwareMaximumFrameIntervalUs);
        _softwareHealthyFrameCount = 0;
        return;
    }

    if (latencyUs < _softwareFrameIntervalUs &&
        _softwareFrameIntervalUs > XCWSoftwareMinimumFrameIntervalUs) {
        _softwareHealthyFrameCount += 1;
        if (_softwareHealthyFrameCount >= XCWSoftwareHealthyFrameWindow) {
            uint64_t nextIntervalUs = _softwareFrameIntervalUs > XCWSoftwareFrameIntervalStepUs
                ? _softwareFrameIntervalUs - XCWSoftwareFrameIntervalStepUs
                : XCWSoftwareMinimumFrameIntervalUs;
            _softwareFrameIntervalUs = MAX(nextIntervalUs, XCWSoftwareMinimumFrameIntervalUs);
            _softwareHealthyFrameCount = 0;
        }
        return;
    }

    _softwareHealthyFrameCount = 0;
}

- (void)drainPendingFramesLocked {
    while (YES) {
        if (_inFlightFrameCount >= XCWMaximumInFlightFrames) {
            _drainScheduled = NO;
            return;
        }

        CVPixelBufferRef pixelBuffer = NULL;
        os_unfair_lock_lock(&_pendingLock);
        pixelBuffer = _pendingPixelBuffer;
        _pendingPixelBuffer = NULL;
        if (pixelBuffer == NULL) {
            _drainScheduled = NO;
            os_unfair_lock_unlock(&_pendingLock);
            return;
        }
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

    CGSize targetSize = XCWScaledDimensionsForSourceSize(sourceWidth, sourceHeight, _encoderMode);
    int32_t targetWidth = (int32_t)targetSize.width;
    int32_t targetHeight = (int32_t)targetSize.height;
    if (targetWidth <= 0 || targetHeight <= 0) {
        return NO;
    }

    uint64_t nowUs = (uint64_t)(CACurrentMediaTime() * 1000000.0);
    if ([self shouldPaceSoftwareFrameAtTimeUs:nowUs]) {
        return YES;
    }

    if (![self ensureCompressionSessionWithWidth:targetWidth height:targetHeight]) {
        return NO;
    }

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
    CMTime presentationTime = CMTimeMake((int64_t)relativeTimestampUs, 1000000);

    NSDictionary *frameOptions = nil;
    if (_needsKeyFrame) {
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
        return NO;
    }

    _inFlightFrameCount += 1;
    _submittedFrameCount += 1;
    if (_encoderMode == XCWVideoEncoderModeH264Software) {
        _lastSoftwareSubmissionUs = nowUs;
    }
    _maxInFlightFrameCount = MAX(_maxInFlightFrameCount, _inFlightFrameCount);
    if (_encoderMode == XCWVideoEncoderModeH264Software) {
        VTCompressionSessionCompleteFrames(_compressionSession, presentationTime);
    }
    return YES;
}

- (BOOL)ensureCompressionSessionWithWidth:(int32_t)width height:(int32_t)height {
    if (_compressionSession != NULL && _width == width && _height == height) {
        return YES;
    }

    [self invalidateCompressionSessionLocked];

    NSMutableDictionary *encoderSpecification = [NSMutableDictionary dictionary];
    NSString *encoderID = XCWVideoEncoderIDForMode(_encoderMode);
    if (encoderID.length > 0) {
        encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_EncoderID] = encoderID;
    }
    if (_encoderMode != XCWVideoEncoderModeH264Software) {
        if (@available(macOS 11.3, *)) {
            encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_EnableLowLatencyRateControl] = @YES;
        }
    }
    if (_encoderMode == XCWVideoEncoderModeH264Software) {
        encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder] = @NO;
    } else {
        encoderSpecification[(__bridge NSString *)kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder] = @YES;
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

    int expectedFrameRate = _encoderMode == XCWVideoEncoderModeH264Software
        ? XCWTargetSoftwareFrameRate
        : XCWTargetRealTimeFrameRate;
    int averageBitRate = XCWAverageBitRateForDimensions(width, height, _encoderMode);

    VTSessionSetProperty(session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
    if (@available(macOS 10.14, *)) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_MaximizePowerEfficiency, kCFBooleanFalse);
    }
    XCWApplyCompressionPresetIfAvailable(session);
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowTemporalCompression, kCFBooleanTrue);
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
    if (@available(macOS 10.14, *)) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowOpenGOP, kCFBooleanFalse);
    }
    if (_codecType == kCMVideoCodecType_HEVC) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_HEVC_Main_AutoLevel);
    } else if (_encoderMode == XCWVideoEncoderModeH264Software) {
        if (@available(macOS 12.0, *)) {
            VTSessionSetProperty(session,
                                 kVTCompressionPropertyKey_ProfileLevel,
                                 kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel);
        } else {
            VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_Baseline_AutoLevel);
        }
        VTSessionSetProperty(session, kVTCompressionPropertyKey_H264EntropyMode, kVTH264EntropyMode_CAVLC);
    } else {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel);
    }
    VTSessionSetProperty(session, kVTCompressionPropertyKey_ExpectedFrameRate, (__bridge CFTypeRef)@(expectedFrameRate));
    VTSessionSetProperty(session, kVTCompressionPropertyKey_MaxKeyFrameInterval, (__bridge CFTypeRef)@(expectedFrameRate * 2));
    VTSessionSetProperty(session, kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, (__bridge CFTypeRef)@2.0);
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AverageBitRate, (__bridge CFTypeRef)@(averageBitRate));
    if (@available(macOS 11.0, *)) {
        VTSessionSetProperty(session,
                             kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality,
                             kCFBooleanTrue);
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

    return YES;
}

- (void)invalidateCompressionSessionLocked {
    if (_compressionSession == NULL) {
        [self invalidateScalingResourcesLocked];
        return;
    }

    VTCompressionSessionInvalidate(_compressionSession);
    CFRelease(_compressionSession);
    _compressionSession = NULL;
    _width = 0;
    _height = 0;
    _timestampOriginUs = 0;
    _inFlightFrameCount = 0;
    _lastSoftwareSubmissionUs = 0;
    _hardwareAccelerated = NO;
    [self invalidateScalingResourcesLocked];
}

- (nullable CVPixelBufferRef)copyScaledPixelBufferIfNeeded:(CVPixelBufferRef)pixelBuffer
                                               targetWidth:(int32_t)targetWidth
                                              targetHeight:(int32_t)targetHeight {
    int32_t sourceWidth = (int32_t)CVPixelBufferGetWidth(pixelBuffer);
    int32_t sourceHeight = (int32_t)CVPixelBufferGetHeight(pixelBuffer);
    if (sourceWidth == targetWidth && sourceHeight == targetHeight) {
        CVPixelBufferRetain(pixelBuffer);
        return pixelBuffer;
    }

    if (_pixelTransferSession == NULL) {
        OSStatus sessionStatus = VTPixelTransferSessionCreate(kCFAllocatorDefault, &_pixelTransferSession);
        if (sessionStatus != noErr || _pixelTransferSession == NULL) {
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

    OSType sourcePixelFormat = CVPixelBufferGetPixelFormatType(pixelBuffer);
    BOOL needsNewBuffer = (_scaledPixelBuffer == NULL)
        || ((int32_t)CVPixelBufferGetWidth(_scaledPixelBuffer) != targetWidth)
        || ((int32_t)CVPixelBufferGetHeight(_scaledPixelBuffer) != targetHeight)
        || (_scaledPixelFormat != sourcePixelFormat);
    if (needsNewBuffer) {
        if (_scaledPixelBuffer != NULL) {
            CVPixelBufferRelease(_scaledPixelBuffer);
            _scaledPixelBuffer = NULL;
        }

        NSDictionary *attributes = @{
            (__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey: @{},
        };
        CVPixelBufferRef scaledPixelBuffer = NULL;
        OSStatus bufferStatus = CVPixelBufferCreate(kCFAllocatorDefault,
                                                    targetWidth,
                                                    targetHeight,
                                                    sourcePixelFormat,
                                                    (__bridge CFDictionaryRef)attributes,
                                                    &scaledPixelBuffer);
        if (bufferStatus != noErr || scaledPixelBuffer == NULL) {
            return NULL;
        }
        _scaledPixelBuffer = scaledPixelBuffer;
        _scaledPixelFormat = sourcePixelFormat;
    }

    OSStatus transferStatus = VTPixelTransferSessionTransferImage(_pixelTransferSession,
                                                                  pixelBuffer,
                                                                  _scaledPixelBuffer);
    _lastScaleStatus = transferStatus;
    if (transferStatus != noErr) {
        return NULL;
    }

    CVPixelBufferRetain(_scaledPixelBuffer);
    return _scaledPixelBuffer;
}

- (void)invalidateScalingResourcesLocked {
    if (_scaledPixelBuffer != NULL) {
        CVPixelBufferRelease(_scaledPixelBuffer);
        _scaledPixelBuffer = NULL;
    }
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
    if (submittedAtUs > 0) {
        uint64_t nowUs = (uint64_t)(CACurrentMediaTime() * 1000000.0);
        _latestEncodeLatencyUs = nowUs >= submittedAtUs ? nowUs - submittedAtUs : 0;
        [self adaptSoftwarePacingForLatencyUs:_latestEncodeLatencyUs];
    }
    NSString *codec = nil;
    NSData *decoderConfig = nil;

    if (isKeyFrame) {
        CMFormatDescriptionRef formatDescription = CMSampleBufferGetFormatDescription(sampleBuffer);
        if (formatDescription != NULL) {
            CMVideoCodecType mediaSubType = CMFormatDescriptionGetMediaSubType(formatDescription);
            if (mediaSubType == kCMVideoCodecType_HEVC) {
                decoderConfig = XCWDecoderConfigurationRecordFromFormatDescription(formatDescription, @"hvcC");
                codec = XCWCodecStringFromHVCC(decoderConfig);
            } else if (mediaSubType == kCMVideoCodecType_H264) {
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
                        decoderConfig = XCWAVCDecoderConfigurationRecord(spsData, ppsData);
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
}

- (void)completeInFlightFrame {
    dispatch_async(_queue, ^{
        if (self->_inFlightFrameCount > 0) {
            self->_inFlightFrameCount -= 1;
        }
        [self drainPendingFramesLocked];
    });
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
    [encoder handleEncodedSampleBuffer:sampleBuffer
                         submittedAtUs:(uint64_t)(uintptr_t)sourceFrameRefCon];
    [encoder completeInFlightFrame];
}
