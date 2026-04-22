#import <AppKit/AppKit.h>

NS_ASSUME_NONNULL_BEGIN

@interface XCWChromeRenderer : NSObject

+ (nullable NSData *)PNGDataForDeviceName:(NSString *)deviceName
                                    error:(NSError * _Nullable * _Nullable)error;
+ (nullable NSDictionary<NSString *, id> *)profileForDeviceName:(NSString *)deviceName
                                                          error:(NSError * _Nullable * _Nullable)error;

@end

NS_ASSUME_NONNULL_END
