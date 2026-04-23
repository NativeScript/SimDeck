#import "XCWNativeSession.h"

#import "XCWPrivateSimulatorSession.h"
#import "XCWSimctl.h"

@interface XCWNativeSession ()

@property (nonatomic, strong, readonly) XCWPrivateSimulatorSession *session;
@property (nonatomic, copy, readonly) NSString *udid;

@end

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

@implementation XCWNativeSession {
    id _listenerToken;
    xcw_native_frame_callback _frameCallback;
    void *_frameCallbackUserData;
}

- (nullable instancetype)initWithUDID:(NSString *)udid
                                error:(NSError * _Nullable __autoreleasing *)error {
    XCWSimctl *simctl = [[XCWSimctl alloc] init];
    NSError *lookupError = nil;
    NSDictionary *simulator = [simctl simulatorWithUDID:udid error:&lookupError];
    if (simulator == nil) {
        if (error != NULL) {
            *error = lookupError;
        }
        return nil;
    }

    XCWPrivateSimulatorSession *session = [[XCWPrivateSimulatorSession alloc] initWithUDID:udid
                                                                             simulatorName:simulator[@"name"] ?: udid
                                                                                     error:error];
    if (session == nil) {
        return nil;
    }

    self = [super init];
    if (self == nil) {
        [session disconnect];
        return nil;
    }

    _udid = [udid copy];
    _session = session;
    return self;
}

- (void)dealloc {
    [self disconnect];
}

- (BOOL)start:(NSError * _Nullable __autoreleasing *)error {
    if (![self.session waitUntilReadyWithTimeout:10.0]) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"XcodeCanvasWeb.NativeSession"
                                         code:1
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Timed out waiting for the private simulator display to become ready.",
            }];
        }
        return NO;
    }

    [self.session requestKeyFrameRefresh];
    if (![self.session waitForFirstEncodedFrameWithTimeout:2.0]) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"XcodeCanvasWeb.NativeSession"
                                         code:2
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Timed out waiting for the first encoded simulator frame.",
            }];
        }
        return NO;
    }

    return YES;
}

- (void)requestRefresh {
    [self.session requestKeyFrameRefresh];
}

- (NSDictionary *)sessionInfoRepresentation {
    return [self.session sessionInfoRepresentation];
}

- (BOOL)sendTouchAtX:(double)x
                   y:(double)y
               phase:(NSString *)phase
               error:(NSError * _Nullable __autoreleasing *)error {
    return [self.session sendTouchWithNormalizedX:x normalizedY:y phase:phase error:error];
}

- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(uint32_t)modifiers
              error:(NSError * _Nullable __autoreleasing *)error {
    return [self.session sendKeyCode:keyCode modifiers:modifiers error:error];
}

- (BOOL)pressHome:(NSError * _Nullable __autoreleasing *)error {
    return [self.session pressHomeButton:error];
}

- (BOOL)rotateRight:(NSError * _Nullable __autoreleasing *)error {
    return [self.session rotateRight:error];
}

- (BOOL)rotateLeft:(NSError * _Nullable __autoreleasing *)error {
    return [self.session rotateLeft:error];
}

- (void)setFrameCallback:(xcw_native_frame_callback)callback
                 userData:(void *)userData {
    _frameCallback = callback;
    _frameCallbackUserData = userData;

    if (_listenerToken != nil) {
        [self.session removeEncodedFrameListener:_listenerToken];
        _listenerToken = nil;
    }

    if (callback == NULL) {
        return;
    }

    __weak typeof(self) weakSelf = self;
    _listenerToken = [self.session addEncodedFrameListener:^(NSData *sampleData,
                                                             NSUInteger frameSequence,
                                                             uint64_t timestampUs,
                                                             BOOL isKeyFrame,
                                                             NSString * _Nullable codec,
                                                             NSData * _Nullable decoderConfig,
                                                             CGSize dimensions) {
        __strong typeof(weakSelf) strongSelf = weakSelf;
        if (strongSelf == nil || strongSelf->_frameCallback == NULL) {
            return;
        }

        xcw_native_frame frame = {
            .frame_sequence = (uint64_t)frameSequence,
            .timestamp_us = timestampUs,
            .is_keyframe = isKeyFrame,
            .width = (uint32_t)llround(dimensions.width),
            .height = (uint32_t)llround(dimensions.height),
            .codec = codec.UTF8String,
            .description = XCWSharedBytesFromData(decoderConfig),
            .data = XCWSharedBytesFromData(sampleData),
        };
        strongSelf->_frameCallback(&frame, strongSelf->_frameCallbackUserData);
    }];
}

- (void)disconnect {
    if (_listenerToken != nil) {
        [self.session removeEncodedFrameListener:_listenerToken];
        _listenerToken = nil;
    }
    [self.session disconnect];
}

@end
