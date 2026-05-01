#import <Foundation/Foundation.h>

#import "XCWNativeBridge.h"

NS_ASSUME_NONNULL_BEGIN

@interface XCWNativeSession : NSObject

- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;
- (nullable instancetype)initWithUDID:(NSString *)udid
                                error:(NSError * _Nullable * _Nullable)error NS_DESIGNATED_INITIALIZER;

- (BOOL)start:(NSError * _Nullable * _Nullable)error;
- (void)requestRefresh;
- (void)requestKeyFrame;
- (void)reconfigureVideoEncoder;
- (BOOL)sendTouchAtX:(double)x
                   y:(double)y
               phase:(NSString *)phase
               error:(NSError * _Nullable * _Nullable)error;
- (BOOL)sendMultiTouchAtX1:(double)x1
                        y1:(double)y1
                        x2:(double)x2
                        y2:(double)y2
                     phase:(NSString *)phase
                     error:(NSError * _Nullable * _Nullable)error;
- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(uint32_t)modifiers
              error:(NSError * _Nullable * _Nullable)error;
- (BOOL)pressHome:(NSError * _Nullable * _Nullable)error;
- (BOOL)openAppSwitcher:(NSError * _Nullable * _Nullable)error;
- (BOOL)rotateRight:(NSError * _Nullable * _Nullable)error;
- (BOOL)rotateLeft:(NSError * _Nullable * _Nullable)error;
- (void)setFrameCallback:(xcw_native_frame_callback _Nullable)callback
                 userData:(void * _Nullable)userData;
- (void)disconnect;

@end

NS_ASSUME_NONNULL_END
