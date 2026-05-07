#import "XCWPrivateSimulatorSession.h"

#import <CoreGraphics/CoreGraphics.h>
#import <CoreImage/CoreImage.h>
#import <CoreVideo/CoreVideo.h>
#import <ImageIO/ImageIO.h>
#import <QuartzCore/QuartzCore.h>

#import "DFPrivateSimulatorDisplayBridge.h"
#import "XCWH264Encoder.h"

static NSString * const XCWPrivateSimulatorSessionErrorDomain = @"SimDeck.PrivateSimulatorSession";

@interface XCWPrivateSimulatorSession () <DFPrivateSimulatorDisplayBridgeDelegate>

@property (nonatomic, copy, readwrite) NSString *udid;
@property (nonatomic, copy, readwrite) NSString *simulatorName;

@end

@interface XCWPrivateSimulatorJPEGFrameListener : NSObject

@property (nonatomic, copy) XCWPrivateSimulatorJPEGFrameHandler handler;
@property (nonatomic) NSUInteger maxEdge;
@property (nonatomic) double quality;

@end

@implementation XCWPrivateSimulatorJPEGFrameListener

@end

static uint64_t XCWCurrentTimestampUs(void) {
    return (uint64_t)llround(CACurrentMediaTime() * 1000000.0);
}

static CIContext *XCWSharedCIContext(void) {
    static CIContext *context = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        context = [CIContext contextWithOptions:@{
            kCIContextUseSoftwareRenderer: @NO,
        }];
    });
    return context;
}

static CGImageRef XCWCreateScaledImageIfNeeded(CGImageRef image, NSUInteger maxEdge) CF_RETURNS_RETAINED {
    if (image == NULL || maxEdge == 0) {
        return image == NULL ? NULL : CGImageRetain(image);
    }

    size_t sourceWidth = CGImageGetWidth(image);
    size_t sourceHeight = CGImageGetHeight(image);
    size_t sourceMaxEdge = MAX(sourceWidth, sourceHeight);
    if (sourceMaxEdge <= maxEdge) {
        return CGImageRetain(image);
    }

    CGFloat scale = (CGFloat)maxEdge / (CGFloat)sourceMaxEdge;
    size_t targetWidth = MAX((size_t)1, (size_t)llround((CGFloat)sourceWidth * scale));
    size_t targetHeight = MAX((size_t)1, (size_t)llround((CGFloat)sourceHeight * scale));
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(NULL,
                                                 targetWidth,
                                                 targetHeight,
                                                 8,
                                                 0,
                                                 colorSpace,
                                                 kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little);
    CGColorSpaceRelease(colorSpace);
    if (context == NULL) {
        return CGImageRetain(image);
    }

    CGContextSetInterpolationQuality(context, kCGInterpolationMedium);
    CGContextDrawImage(context, CGRectMake(0, 0, (CGFloat)targetWidth, (CGFloat)targetHeight), image);
    CGImageRef scaledImage = CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    return scaledImage ?: CGImageRetain(image);
}

static NSData *XCWJPEGDataFromPixelBuffer(CVPixelBufferRef pixelBuffer, NSUInteger maxEdge, double quality, CGSize *dimensions) {
    if (pixelBuffer == NULL) {
        return nil;
    }

    size_t width = CVPixelBufferGetWidth(pixelBuffer);
    size_t height = CVPixelBufferGetHeight(pixelBuffer);
    if (width == 0 || height == 0) {
        return nil;
    }

    CIImage *ciImage = [CIImage imageWithCVPixelBuffer:pixelBuffer];
    CGImageRef sourceImage = [XCWSharedCIContext() createCGImage:ciImage
                                                        fromRect:CGRectMake(0, 0, (CGFloat)width, (CGFloat)height)];
    if (sourceImage == NULL) {
        return nil;
    }

    CGImageRef outputImage = XCWCreateScaledImageIfNeeded(sourceImage, maxEdge);
    CGImageRelease(sourceImage);
    if (outputImage == NULL) {
        return nil;
    }

    NSMutableData *data = [NSMutableData data];
    CGImageDestinationRef destination = CGImageDestinationCreateWithData((__bridge CFMutableDataRef)data,
                                                                        CFSTR("public.jpeg"),
                                                                        1,
                                                                        NULL);
    if (destination == NULL) {
        CGImageRelease(outputImage);
        return nil;
    }

    NSDictionary *properties = @{
        (__bridge NSString *)kCGImageDestinationLossyCompressionQuality: @(MIN(MAX(quality, 0.2), 0.95)),
    };
    CGImageDestinationAddImage(destination, outputImage, (__bridge CFDictionaryRef)properties);
    BOOL finalized = CGImageDestinationFinalize(destination);
    CFRelease(destination);
    if (dimensions != NULL) {
        *dimensions = CGSizeMake((CGFloat)CGImageGetWidth(outputImage), (CGFloat)CGImageGetHeight(outputImage));
    }
    CGImageRelease(outputImage);
    return finalized && data.length > 0 ? data : nil;
}

@implementation XCWPrivateSimulatorSession {
    DFPrivateSimulatorDisplayBridge *_displayBridge;
    dispatch_queue_t _stateQueue;
    dispatch_queue_t _jpegQueue;
    dispatch_semaphore_t _readinessSemaphore;
    XCWH264Encoder *_videoEncoder;
    NSString *_displayStatusValue;
    CGSize _displaySizeValue;
    NSMutableDictionary<NSUUID *, XCWPrivateSimulatorEncodedFrameHandler> *_encodedFrameListeners;
    NSMutableDictionary<NSUUID *, XCWPrivateSimulatorJPEGFrameListener *> *_jpegFrameListeners;
    NSUInteger _encodedFrameSequenceValue;
    NSUInteger _jpegFrameSequenceValue;
    CVPixelBufferRef _pendingJPEGPixelBuffer;
    BOOL _jpegEncodeInFlight;
    NSData *_latestKeyFrameData;
    uint64_t _latestKeyFrameTimestampUs;
    NSString *_latestKeyFrameCodec;
    NSData *_latestKeyFrameDecoderConfig;
    CGSize _latestKeyFrameDimensions;
    NSUInteger _latestKeyFrameSequenceValue;
    BOOL _displayReadyValue;
    BOOL _didSignalReadiness;
}

- (nullable instancetype)initWithUDID:(NSString *)udid
                        simulatorName:(NSString *)simulatorName
                                error:(NSError * _Nullable __autoreleasing *)error {
    NSError *bridgeError = nil;
    DFPrivateSimulatorDisplayBridge *bridge = [[DFPrivateSimulatorDisplayBridge alloc] initWithUDID:udid error:&bridgeError];
    if (bridge == nil) {
        if (error != NULL) {
            *error = bridgeError ?: [NSError errorWithDomain:XCWPrivateSimulatorSessionErrorDomain
                                                         code:1
                                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to initialize the private simulator display bridge.",
            }];
        }
        return nil;
    }

    self = [super init];
    if (self == nil) {
        return nil;
    }

    _udid = [udid copy];
    _simulatorName = [simulatorName copy];
    _displayBridge = bridge;
    _displayBridge.delegate = self;
    dispatch_queue_attr_t queueAttributes =
        dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_USER_INITIATED, 0);
    _stateQueue = dispatch_queue_create("com.simdeck.private-session.state", queueAttributes);
    _jpegQueue = dispatch_queue_create("com.simdeck.private-session.jpeg", queueAttributes);
    _readinessSemaphore = dispatch_semaphore_create(0);
    _encodedFrameListeners = [NSMutableDictionary dictionary];
    _jpegFrameListeners = [NSMutableDictionary dictionary];
    _displayStatusValue = bridge.displayStatus ?: @"Initializing private simulator display";
    _displayReadyValue = bridge.isDisplayReady;
    __weak typeof(self) weakSelf = self;
    _videoEncoder = [[XCWH264Encoder alloc] initWithOutputHandler:^(NSData *sampleData,
                                                                    uint64_t timestampUs,
                                                                    BOOL isKeyFrame,
                                                                    NSString * _Nullable codec,
                                                                    NSData * _Nullable decoderConfig,
                                                                    CGSize dimensions) {
        __strong typeof(weakSelf) strongSelf = weakSelf;
        if (strongSelf == nil || sampleData.length == 0) {
            return;
        }

        dispatch_async(strongSelf->_stateQueue, ^{
            strongSelf->_encodedFrameSequenceValue += 1;
            NSUInteger frameSequence = strongSelf->_encodedFrameSequenceValue;
            if (isKeyFrame) {
                strongSelf->_latestKeyFrameData = sampleData;
                strongSelf->_latestKeyFrameTimestampUs = timestampUs;
                strongSelf->_latestKeyFrameCodec = [codec copy];
                strongSelf->_latestKeyFrameDecoderConfig = decoderConfig;
                strongSelf->_latestKeyFrameDimensions = dimensions;
                strongSelf->_latestKeyFrameSequenceValue = frameSequence;
            }
            if (strongSelf->_encodedFrameListeners.count == 0) {
                return;
            }

            NSDictionary<NSUUID *, XCWPrivateSimulatorEncodedFrameHandler> *listeners = [strongSelf->_encodedFrameListeners copy];
            [listeners enumerateKeysAndObjectsUsingBlock:^(__unused NSUUID *token, XCWPrivateSimulatorEncodedFrameHandler handler, __unused BOOL *stop) {
                handler(sampleData,
                        frameSequence,
                        timestampUs,
                        isKeyFrame,
                        codec,
                        decoderConfig,
                        dimensions);
            }];
        });
    }];

    [self primeStateFromBridge];
    return self;
}

- (void)dealloc {
    [_videoEncoder invalidate];
    if (_pendingJPEGPixelBuffer != nil) {
        CVPixelBufferRelease(_pendingJPEGPixelBuffer);
        _pendingJPEGPixelBuffer = nil;
    }
}

- (BOOL)waitUntilReadyWithTimeout:(NSTimeInterval)timeout {
    if (self.displayReady) {
        return YES;
    }
    long result = dispatch_semaphore_wait(_readinessSemaphore, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(timeout * NSEC_PER_SEC)));
    return result == 0 ? YES : self.displayReady;
}

- (BOOL)waitForFirstEncodedFrameWithTimeout:(NSTimeInterval)timeout {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:timeout];
    [self refreshCurrentFrame];
    [_videoEncoder requestKeyFrame];

    while ([deadline timeIntervalSinceNow] > 0) {
        __block BOOL hasFrame = NO;
        dispatch_sync(_stateQueue, ^{
            hasFrame = self->_latestKeyFrameData.length > 0;
        });
        if (hasFrame) {
            return YES;
        }
        [NSThread sleepForTimeInterval:0.01];
    }

    __block BOOL hasFrame = NO;
    dispatch_sync(_stateQueue, ^{
        hasFrame = self->_latestKeyFrameData.length > 0;
    });
    return hasFrame;
}

- (void)refreshCurrentFrame {
    CVPixelBufferRef pixelBuffer = [_displayBridge copyPixelBuffer];
    if (pixelBuffer == nil) {
        return;
    }

    CGSize displaySize = CGSizeMake((CGFloat)CVPixelBufferGetWidth(pixelBuffer), (CGFloat)CVPixelBufferGetHeight(pixelBuffer));
    dispatch_async(_stateQueue, ^{
        self->_displaySizeValue = displaySize;
        self->_displayReadyValue = YES;
        self->_displayStatusValue = [NSString stringWithFormat:@"Private display ready (%.0fx%.0f)", displaySize.width, displaySize.height];
        [self signalReadinessIfNeededLocked];
    });
    [_videoEncoder encodePixelBuffer:pixelBuffer];
    [self enqueueJPEGPixelBufferIfNeeded:pixelBuffer];
    CVPixelBufferRelease(pixelBuffer);
}

- (void)requestKeyFrameRefresh {
    [_videoEncoder requestKeyFrame];
    [self refreshCurrentFrame];
}

- (void)requestFrameRefresh {
    [self refreshCurrentFrame];
}

- (void)reconfigureVideoEncoder {
    [_videoEncoder reconfigureForStreamQualityChange];
    [_videoEncoder requestKeyFrame];
    [self refreshCurrentFrame];
}

- (NSDictionary *)videoEncoderStats {
    return [_videoEncoder statsRepresentation];
}

- (id)addEncodedFrameListener:(XCWPrivateSimulatorEncodedFrameHandler)handler {
    if (handler == nil) {
        return [NSUUID UUID];
    }

    NSUUID *token = [NSUUID UUID];
    dispatch_sync(_stateQueue, ^{
        self->_encodedFrameListeners[token] = [handler copy];
    });
    [_videoEncoder requestKeyFrame];
    [self refreshCurrentFrame];
    return token;
}

- (void)removeEncodedFrameListener:(id)token {
    if (![token isKindOfClass:[NSUUID class]]) {
        return;
    }

    dispatch_sync(_stateQueue, ^{
        [self->_encodedFrameListeners removeObjectForKey:(NSUUID *)token];
    });
}

- (id)addJPEGFrameListenerWithMaxEdge:(NSUInteger)maxEdge
                               quality:(double)quality
                               handler:(XCWPrivateSimulatorJPEGFrameHandler)handler {
    if (handler == nil) {
        return [NSUUID UUID];
    }

    NSUUID *token = [NSUUID UUID];
    XCWPrivateSimulatorJPEGFrameListener *listener = [XCWPrivateSimulatorJPEGFrameListener new];
    listener.handler = handler;
    listener.maxEdge = maxEdge;
    listener.quality = quality;
    dispatch_sync(_stateQueue, ^{
        self->_jpegFrameListeners[token] = listener;
    });
    [self refreshCurrentFrame];
    return token;
}

- (void)removeJPEGFrameListener:(id)token {
    if (![token isKindOfClass:[NSUUID class]]) {
        return;
    }

    dispatch_sync(_stateQueue, ^{
        [self->_jpegFrameListeners removeObjectForKey:(NSUUID *)token];
    });
}

- (BOOL)isDisplayReady {
    __block BOOL ready = NO;
    dispatch_sync(_stateQueue, ^{
        ready = self->_displayReadyValue;
    });
    return ready;
}

- (NSString *)displayStatus {
    __block NSString *status = nil;
    dispatch_sync(_stateQueue, ^{
        status = self->_displayStatusValue ?: @"";
    });
    return status;
}

- (CGSize)displaySize {
    __block CGSize size = CGSizeZero;
    dispatch_sync(_stateQueue, ^{
        size = self->_displaySizeValue;
    });
    return size;
}

- (NSInteger)rotationQuarterTurns {
    return _displayBridge.rotationQuarterTurns;
}

- (NSUInteger)frameSequence {
    __block NSUInteger sequence = 0;
    dispatch_sync(_stateQueue, ^{
        sequence = self->_encodedFrameSequenceValue;
    });
    return sequence;
}

- (BOOL)sendTouchWithNormalizedX:(double)normalizedX
                     normalizedY:(double)normalizedY
                           phase:(NSString *)phase
                           error:(NSError * _Nullable __autoreleasing *)error {
    DFPrivateSimulatorTouchPhase touchPhase = DFPrivateSimulatorTouchPhaseMoved;
    if (![self touchPhaseFromString:phase outPhase:&touchPhase error:error]) {
        return NO;
    }

    return [_displayBridge sendTouchAtNormalizedX:normalizedX normalizedY:normalizedY phase:touchPhase error:error];
}

- (BOOL)sendMultiTouchWithNormalizedX1:(double)normalizedX1
                           normalizedY1:(double)normalizedY1
                           normalizedX2:(double)normalizedX2
                           normalizedY2:(double)normalizedY2
                                 phase:(NSString *)phase
                                 error:(NSError * _Nullable __autoreleasing *)error {
    DFPrivateSimulatorTouchPhase touchPhase = DFPrivateSimulatorTouchPhaseMoved;
    if (![self touchPhaseFromString:phase outPhase:&touchPhase error:error]) {
        return NO;
    }

    return [_displayBridge sendMultiTouchAtNormalizedX1:normalizedX1
                                           normalizedY1:normalizedY1
                                           normalizedX2:normalizedX2
                                           normalizedY2:normalizedY2
                                                 phase:touchPhase
                                                 error:error];
}

- (BOOL)touchPhaseFromString:(NSString *)phase
                    outPhase:(DFPrivateSimulatorTouchPhase *)outPhase
                       error:(NSError * _Nullable __autoreleasing *)error {
    NSString *phaseValue = phase.lowercaseString;
    if ([phaseValue isEqualToString:@"began"]) {
        *outPhase = DFPrivateSimulatorTouchPhaseBegan;
    } else if ([phaseValue isEqualToString:@"moved"]) {
        *outPhase = DFPrivateSimulatorTouchPhaseMoved;
    } else if ([phaseValue isEqualToString:@"ended"]) {
        *outPhase = DFPrivateSimulatorTouchPhaseEnded;
    } else if ([phaseValue isEqualToString:@"cancelled"]) {
        *outPhase = DFPrivateSimulatorTouchPhaseCancelled;
    } else {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWPrivateSimulatorSessionErrorDomain
                                         code:1
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Unsupported touch phase `%@`.", phase ?: @""],
            }];
        }
        return NO;
    }
    return YES;
}

- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(NSUInteger)modifiers
              error:(NSError * _Nullable __autoreleasing *)error {
    return [_displayBridge sendKeyCode:keyCode modifiers:modifiers error:error];
}

- (BOOL)pressHomeButton:(NSError * _Nullable __autoreleasing *)error {
    return [_displayBridge pressHomeButton:error];
}

- (BOOL)openAppSwitcher:(NSError * _Nullable __autoreleasing *)error {
    return [_displayBridge openAppSwitcher:error];
}

- (BOOL)rotateRight:(NSError * _Nullable __autoreleasing *)error {
    return [_displayBridge rotateRight:error];
}

- (BOOL)rotateLeft:(NSError * _Nullable __autoreleasing *)error {
    return [_displayBridge rotateLeft:error];
}

- (void)disconnect {
    [_displayBridge disconnect];
    _displayBridge.delegate = nil;
    [_videoEncoder invalidate];
}

- (void)privateSimulatorDisplayBridge:(DFPrivateSimulatorDisplayBridge *)bridge didUpdateFrame:(CVPixelBufferRef)pixelBuffer {
    CGSize displaySize = CGSizeMake((CGFloat)CVPixelBufferGetWidth(pixelBuffer), (CGFloat)CVPixelBufferGetHeight(pixelBuffer));
    dispatch_async(_stateQueue, ^{
        self->_displaySizeValue = displaySize;
        self->_displayReadyValue = YES;
        self->_displayStatusValue = [NSString stringWithFormat:@"Private display ready (%.0fx%.0f)", displaySize.width, displaySize.height];
        [self signalReadinessIfNeededLocked];
    });
    [_videoEncoder encodePixelBuffer:pixelBuffer];
    [self enqueueJPEGPixelBufferIfNeeded:pixelBuffer];
}

- (void)privateSimulatorDisplayBridge:(DFPrivateSimulatorDisplayBridge *)bridge
                didChangeDisplayStatus:(NSString *)status
                               isReady:(BOOL)isReady {
    dispatch_async(_stateQueue, ^{
        self->_displayStatusValue = [status copy];
        self->_displayReadyValue = isReady;
        [self signalReadinessIfNeededLocked];
    });
}

- (void)primeStateFromBridge {
    dispatch_async(_stateQueue, ^{
        self->_displayStatusValue = self->_displayBridge.displayStatus ?: self->_displayStatusValue;
        self->_displayReadyValue = self->_displayBridge.isDisplayReady;
        [self signalReadinessIfNeededLocked];
    });

    CVPixelBufferRef pixelBuffer = [_displayBridge copyPixelBuffer];
    if (pixelBuffer != nil) {
        CGSize displaySize = CGSizeMake((CGFloat)CVPixelBufferGetWidth(pixelBuffer), (CGFloat)CVPixelBufferGetHeight(pixelBuffer));
        dispatch_async(_stateQueue, ^{
            self->_displaySizeValue = displaySize;
            self->_displayReadyValue = YES;
            self->_displayStatusValue = [NSString stringWithFormat:@"Private display ready (%.0fx%.0f)", displaySize.width, displaySize.height];
            [self signalReadinessIfNeededLocked];
        });
        [_videoEncoder encodePixelBuffer:pixelBuffer];
        [self enqueueJPEGPixelBufferIfNeeded:pixelBuffer];
        CVPixelBufferRelease(pixelBuffer);
    }
}

- (void)enqueueJPEGPixelBufferIfNeeded:(CVPixelBufferRef)pixelBuffer {
    if (pixelBuffer == NULL) {
        return;
    }

    __block BOOL hasListeners = NO;
    dispatch_sync(_stateQueue, ^{
        hasListeners = self->_jpegFrameListeners.count > 0;
        if (!hasListeners) {
            if (self->_pendingJPEGPixelBuffer != nil) {
                CVPixelBufferRelease(self->_pendingJPEGPixelBuffer);
                self->_pendingJPEGPixelBuffer = nil;
            }
            self->_jpegEncodeInFlight = NO;
        }
    });
    if (!hasListeners) {
        return;
    }

    CVPixelBufferRetain(pixelBuffer);
    __block BOOL shouldStartEncode = NO;
    dispatch_sync(_stateQueue, ^{
        if (self->_jpegEncodeInFlight) {
            if (self->_pendingJPEGPixelBuffer != nil) {
                CVPixelBufferRelease(self->_pendingJPEGPixelBuffer);
            }
            self->_pendingJPEGPixelBuffer = pixelBuffer;
        } else {
            self->_jpegEncodeInFlight = YES;
            shouldStartEncode = YES;
        }
    });

    if (shouldStartEncode) {
        [self encodeJPEGPixelBuffer:pixelBuffer];
    }
}

- (void)encodeJPEGPixelBuffer:(CVPixelBufferRef)pixelBuffer {
    dispatch_async(_jpegQueue, ^{
        __block NSDictionary<NSUUID *, XCWPrivateSimulatorJPEGFrameListener *> *listeners = nil;
        __block NSUInteger frameSequence = 0;
        dispatch_sync(self->_stateQueue, ^{
            listeners = [self->_jpegFrameListeners copy];
            self->_jpegFrameSequenceValue += 1;
            frameSequence = self->_jpegFrameSequenceValue;
        });

        uint64_t timestampUs = XCWCurrentTimestampUs();
        [listeners enumerateKeysAndObjectsUsingBlock:^(__unused NSUUID *token,
                                                       XCWPrivateSimulatorJPEGFrameListener *listener,
                                                       __unused BOOL *stop) {
            CGSize dimensions = CGSizeZero;
            NSData *jpegData = XCWJPEGDataFromPixelBuffer(pixelBuffer,
                                                          listener.maxEdge,
                                                          listener.quality,
                                                          &dimensions);
            if (jpegData.length > 0) {
                listener.handler(jpegData, frameSequence, timestampUs, dimensions);
            }
        }];
        CVPixelBufferRelease(pixelBuffer);

        __block CVPixelBufferRef nextPixelBuffer = nil;
        dispatch_sync(self->_stateQueue, ^{
            nextPixelBuffer = self->_pendingJPEGPixelBuffer;
            self->_pendingJPEGPixelBuffer = nil;
            if (nextPixelBuffer == nil) {
                self->_jpegEncodeInFlight = NO;
            }
        });
        if (nextPixelBuffer != nil) {
            [self encodeJPEGPixelBuffer:nextPixelBuffer];
        }
    });
}

- (void)signalReadinessIfNeededLocked {
    if (_didSignalReadiness || !_displayReadyValue) {
        return;
    }
    _didSignalReadiness = YES;
    dispatch_semaphore_signal(_readinessSemaphore);
}

@end
