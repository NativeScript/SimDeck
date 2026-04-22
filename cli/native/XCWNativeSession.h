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
- (BOOL)sendTouchAtX:(double)x
                   y:(double)y
               phase:(NSString *)phase
               error:(NSError * _Nullable * _Nullable)error;
- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(uint32_t)modifiers
              error:(NSError * _Nullable * _Nullable)error;
- (BOOL)pressHome:(NSError * _Nullable * _Nullable)error;
- (BOOL)rotateRight:(NSError * _Nullable * _Nullable)error;
- (void)setFrameCallback:(xcw_native_frame_callback _Nullable)callback
                 userData:(void * _Nullable)userData;
- (void)disconnect;

@end

NS_ASSUME_NONNULL_END
