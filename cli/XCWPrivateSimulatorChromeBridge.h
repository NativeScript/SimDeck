#import <AppKit/AppKit.h>

NS_ASSUME_NONNULL_BEGIN

@interface XCWPrivateSimulatorChromeBridge : NSObject

- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;

+ (nullable NSView *)chromeViewForDeviceName:(NSString *)deviceName
                                 displaySize:(CGSize)displaySize;

@end

NS_ASSUME_NONNULL_END
