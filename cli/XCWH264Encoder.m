#import "XCWH264Encoder.h"

#import <CoreMedia/CoreMedia.h>
#import <os/lock.h>
#import <QuartzCore/QuartzCore.h>
#import <VideoToolbox/VideoToolbox.h>

static const int32_t XCWMaximumEncodedDimension = 1920;
static const int32_t XCWTargetRealTimeFrameRate = 60;
static const int32_t XCWMinimumAverageBitRate = 18000000;
static const int64_t XCWBitsPerPixelBudget = 10;

static NSString *XCWCodecStringFromSPS(NSData *spsData) {
    const uint8_t *bytes = spsData.bytes;
    if (spsData.length < 4 || bytes == NULL) {
        return @"avc1.640028";
    }
    return [NSString stringWithFormat:@"avc1.%02x%02x%02x", bytes[1], bytes[2], bytes[3]];
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

static CGSize XCWScaledDimensionsForSourceSize(int32_t width, int32_t height) {
    if (width <= 0 || height <= 0) {
        return CGSizeZero;
    }

    int32_t longestEdge = MAX(width, height);
    if (longestEdge <= XCWMaximumEncodedDimension) {
        return CGSizeMake(width, height);
    }

    double scale = (double)XCWMaximumEncodedDimension / (double)longestEdge;
    return CGSizeMake(XCWRoundToEvenDimension(width * scale),
                      XCWRoundToEvenDimension(height * scale));
}

static int32_t XCWAverageBitRateForDimensions(int32_t width, int32_t height) {
    int64_t computedBitRate = (int64_t)width * (int64_t)height * XCWBitsPerPixelBudget;
    if (computedBitRate < (int64_t)XCWMinimumAverageBitRate) {
        computedBitRate = XCWMinimumAverageBitRate;
    }
    if (computedBitRate > INT32_MAX) {
        computedBitRate = INT32_MAX;
    }
    return (int32_t)computedBitRate;
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
    int32_t _width;
    int32_t _height;
    uint64_t _timestampOriginUs;
    VTPixelTransferSessionRef _pixelTransferSession;
    CVPixelBufferRef _scaledPixelBuffer;
    OSType _scaledPixelFormat;
}

- (instancetype)initWithOutputHandler:(XCWH264EncoderOutputHandler)outputHandler {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _outputHandler = [outputHandler copy];
    dispatch_queue_attr_t queueAttributes =
        dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_USER_INITIATED, 0);
    _queue = dispatch_queue_create("com.xcodecanvasweb.h264-encoder", queueAttributes);
    _pendingLock = OS_UNFAIR_LOCK_INIT;
    _needsKeyFrame = YES;
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
    if (_pendingPixelBuffer != NULL) {
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

- (void)drainPendingFramesLocked {
    while (YES) {
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

- (void)encodePixelBufferLocked:(CVPixelBufferRef)pixelBuffer {
    int32_t sourceWidth = (int32_t)CVPixelBufferGetWidth(pixelBuffer);
    int32_t sourceHeight = (int32_t)CVPixelBufferGetHeight(pixelBuffer);
    if (sourceWidth <= 0 || sourceHeight <= 0) {
        return;
    }

    CGSize targetSize = XCWScaledDimensionsForSourceSize(sourceWidth, sourceHeight);
    int32_t targetWidth = (int32_t)targetSize.width;
    int32_t targetHeight = (int32_t)targetSize.height;
    if (targetWidth <= 0 || targetHeight <= 0) {
        return;
    }

    if (![self ensureCompressionSessionWithWidth:targetWidth height:targetHeight]) {
        return;
    }

    CVPixelBufferRef encodePixelBuffer = [self copyScaledPixelBufferIfNeeded:pixelBuffer
                                                                 targetWidth:targetWidth
                                                                targetHeight:targetHeight];
    if (encodePixelBuffer == NULL) {
        return;
    }

    uint64_t nowUs = (uint64_t)(CACurrentMediaTime() * 1000000.0);
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
                                                      NULL,
                                                      NULL);
    CVPixelBufferRelease(encodePixelBuffer);
    if (status != noErr) {
        _needsKeyFrame = YES;
    }
}

- (BOOL)ensureCompressionSessionWithWidth:(int32_t)width height:(int32_t)height {
    if (_compressionSession != NULL && _width == width && _height == height) {
        return YES;
    }

    [self invalidateCompressionSessionLocked];

    NSDictionary *encoderSpecification = nil;
    if (@available(macOS 11.3, *)) {
        encoderSpecification = @{
            (__bridge NSString *)kVTVideoEncoderSpecification_EnableLowLatencyRateControl: @YES,
        };
    }

    VTCompressionSessionRef session = NULL;
    OSStatus status = VTCompressionSessionCreate(kCFAllocatorDefault,
                                                 width,
                                                 height,
                                                 kCMVideoCodecType_H264,
                                                 (__bridge CFDictionaryRef _Nullable)(encoderSpecification),
                                                 NULL,
                                                 NULL,
                                                 XCWH264EncoderOutputCallback,
                                                 (__bridge void *)self,
                                                 &session);
    if (status != noErr || session == NULL) {
        return NO;
    }

    _compressionSession = session;
    _width = width;
    _height = height;
    _timestampOriginUs = 0;
    _needsKeyFrame = YES;

    int expectedFrameRate = XCWTargetRealTimeFrameRate;
    int averageBitRate = XCWAverageBitRateForDimensions(width, height);

    VTSessionSetProperty(session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowTemporalCompression, kCFBooleanTrue);
    VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
    if (@available(macOS 10.14, *)) {
        VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowOpenGOP, kCFBooleanFalse);
    }
    VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel);
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
    if (status != noErr) {
        [self invalidateCompressionSessionLocked];
        return NO;
    }

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

- (void)handleEncodedSampleBuffer:(CMSampleBufferRef)sampleBuffer {
    if (sampleBuffer == NULL || !CMSampleBufferDataIsReady(sampleBuffer)) {
        return;
    }

    NSData *sampleData = XCWCopySampleData(sampleBuffer);
    if (sampleData.length == 0) {
        return;
    }

    BOOL isKeyFrame = XCWSampleBufferIsKeyFrame(sampleBuffer);
    NSString *codec = nil;
    NSData *decoderConfig = nil;

    if (isKeyFrame) {
        CMFormatDescriptionRef formatDescription = CMSampleBufferGetFormatDescription(sampleBuffer);
        if (formatDescription != NULL) {
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

    CMTime presentationTime = CMSampleBufferGetPresentationTimeStamp(sampleBuffer);
    uint64_t timestampUs = 0;
    if (presentationTime.timescale > 0) {
        timestampUs = (uint64_t)llround(CMTimeGetSeconds(presentationTime) * 1000000.0);
    }

    CGSize dimensions = CGSizeMake(_width, _height);
    self.outputHandler(sampleData, timestampUs, isKeyFrame, codec, decoderConfig, dimensions);
}

@end

static void XCWH264EncoderOutputCallback(void *outputCallbackRefCon,
                                         __unused void *sourceFrameRefCon,
                                         OSStatus status,
                                         __unused VTEncodeInfoFlags infoFlags,
                                         CMSampleBufferRef sampleBuffer) {
    if (status != noErr || sampleBuffer == NULL) {
        return;
    }

    XCWH264Encoder *encoder = (__bridge XCWH264Encoder *)outputCallbackRefCon;
    [encoder handleEncodedSampleBuffer:sampleBuffer];
}
