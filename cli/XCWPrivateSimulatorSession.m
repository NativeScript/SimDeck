#import "XCWPrivateSimulatorSession.h"

#import <CoreGraphics/CoreGraphics.h>
#import <CoreVideo/CoreVideo.h>

#import "DFPrivateSimulatorDisplayBridge.h"
#import "XCWH264Encoder.h"

static NSString * const XCWPrivateSimulatorSessionErrorDomain = @"XcodeCanvasWeb.PrivateSimulatorSession";

@interface XCWPrivateSimulatorSession () <DFPrivateSimulatorDisplayBridgeDelegate>

@property (nonatomic, copy, readwrite) NSString *udid;
@property (nonatomic, copy, readwrite) NSString *simulatorName;

@end

@implementation XCWPrivateSimulatorSession {
    DFPrivateSimulatorDisplayBridge *_displayBridge;
    dispatch_queue_t _stateQueue;
    dispatch_semaphore_t _readinessSemaphore;
    XCWH264Encoder *_videoEncoder;
    NSString *_displayStatusValue;
    CGSize _displaySizeValue;
    NSMutableDictionary<NSUUID *, XCWPrivateSimulatorEncodedFrameHandler> *_encodedFrameListeners;
    NSUInteger _encodedFrameSequenceValue;
    NSData *_latestKeyFrameData;
    uint64_t _latestKeyFrameTimestampUs;
    NSString *_latestKeyFrameCodec;
    NSData *_latestKeyFrameDecoderConfig;
    CGSize _latestKeyFrameDimensions;
    NSUInteger _latestKeyFrameSequenceValue;
    NSUInteger _displayFrameCount;
    NSUInteger _manualRefreshFrameCount;
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
            *error = bridgeError;
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
    _stateQueue = dispatch_queue_create("com.xcodecanvasweb.private-session.state", queueAttributes);
    _readinessSemaphore = dispatch_semaphore_create(0);
    _encodedFrameListeners = [NSMutableDictionary dictionary];
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

- (NSDictionary *)sessionInfoRepresentation {
    __block NSDictionary *representation = nil;
    dispatch_sync(_stateQueue, ^{
        NSMutableDictionary *payload = [@{
            @"displayReady": @(self->_displayReadyValue),
            @"displayStatus": self->_displayStatusValue ?: @"",
            @"displayWidth": @(self->_displaySizeValue.width),
            @"displayHeight": @(self->_displaySizeValue.height),
            @"frameSequence": @(self->_encodedFrameSequenceValue),
            @"displayFrameCount": @(self->_displayFrameCount),
            @"manualRefreshFrameCount": @(self->_manualRefreshFrameCount),
            @"encoder": [self->_videoEncoder statsRepresentation],
        } mutableCopy];
        if (self->_latestKeyFrameCodec.length > 0) {
            payload[@"latestKeyFrameCodec"] = self->_latestKeyFrameCodec;
        }
        if (self->_latestKeyFrameDecoderConfig.length > 0) {
            payload[@"latestDecoderConfigBytes"] = @(self->_latestKeyFrameDecoderConfig.length);
        }
        representation = payload;
    });
    return representation;
}

- (nullable NSDictionary *)latestEncodedKeyFrameRepresentation {
    __block NSDictionary *representation = nil;
    dispatch_sync(_stateQueue, ^{
        if (self->_latestKeyFrameData.length == 0) {
            return;
        }

        NSMutableDictionary *payload = [@{
            @"sampleData": self->_latestKeyFrameData,
            @"frameSequence": @(self->_latestKeyFrameSequenceValue),
            @"timestampUs": @(self->_latestKeyFrameTimestampUs),
            @"width": @(self->_latestKeyFrameDimensions.width),
            @"height": @(self->_latestKeyFrameDimensions.height),
        } mutableCopy];
        if (self->_latestKeyFrameCodec.length > 0) {
            payload[@"codec"] = self->_latestKeyFrameCodec;
        }
        if (self->_latestKeyFrameDecoderConfig.length > 0) {
            payload[@"decoderConfig"] = self->_latestKeyFrameDecoderConfig;
        }
        representation = payload;
    });
    return representation;
}

- (void)refreshCurrentFrame {
    CVPixelBufferRef pixelBuffer = [_displayBridge copyPixelBuffer];
    if (pixelBuffer == nil) {
        return;
    }

    CGSize displaySize = CGSizeMake((CGFloat)CVPixelBufferGetWidth(pixelBuffer), (CGFloat)CVPixelBufferGetHeight(pixelBuffer));
    dispatch_async(_stateQueue, ^{
        self->_manualRefreshFrameCount += 1;
        self->_displaySizeValue = displaySize;
        self->_displayReadyValue = YES;
        self->_displayStatusValue = [NSString stringWithFormat:@"Private display ready (%.0fx%.0f)", displaySize.width, displaySize.height];
        [self signalReadinessIfNeededLocked];
    });
    [_videoEncoder encodePixelBuffer:pixelBuffer];
    CVPixelBufferRelease(pixelBuffer);
}

- (void)requestKeyFrameRefresh {
    [self refreshCurrentFrame];
    [_videoEncoder requestKeyFrame];
}

- (id)addEncodedFrameListener:(XCWPrivateSimulatorEncodedFrameHandler)handler {
    if (handler == nil) {
        return [NSUUID UUID];
    }

    NSUUID *token = [NSUUID UUID];
    dispatch_sync(_stateQueue, ^{
        self->_encodedFrameListeners[token] = [handler copy];
        if (self->_latestKeyFrameData.length > 0) {
            handler(self->_latestKeyFrameData,
                    self->_latestKeyFrameSequenceValue,
                    self->_latestKeyFrameTimestampUs,
                    YES,
                    self->_latestKeyFrameCodec,
                    self->_latestKeyFrameDecoderConfig,
                    self->_latestKeyFrameDimensions);
        }
    });
    [self refreshCurrentFrame];
    [_videoEncoder requestKeyFrame];
    return token;
}

- (void)removeEncodedFrameListener:(id)token {
    if (![token isKindOfClass:[NSUUID class]]) {
        return;
    }

    dispatch_async(_stateQueue, ^{
        [self->_encodedFrameListeners removeObjectForKey:(NSUUID *)token];
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
        self->_displayFrameCount += 1;
        self->_displaySizeValue = displaySize;
        self->_displayReadyValue = YES;
        self->_displayStatusValue = [NSString stringWithFormat:@"Private display ready (%.0fx%.0f)", displaySize.width, displaySize.height];
        [self signalReadinessIfNeededLocked];
    });
    [_videoEncoder encodePixelBuffer:pixelBuffer];
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
            self->_manualRefreshFrameCount += 1;
            self->_displaySizeValue = displaySize;
            self->_displayReadyValue = YES;
            self->_displayStatusValue = [NSString stringWithFormat:@"Private display ready (%.0fx%.0f)", displaySize.width, displaySize.height];
            [self signalReadinessIfNeededLocked];
        });
        [_videoEncoder encodePixelBuffer:pixelBuffer];
        CVPixelBufferRelease(pixelBuffer);
    }
}

- (void)signalReadinessIfNeededLocked {
    if (_didSignalReadiness || !_displayReadyValue) {
        return;
    }
    _didSignalReadiness = YES;
    dispatch_semaphore_signal(_readinessSemaphore);
}

@end
