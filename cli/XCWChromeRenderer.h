#import <AppKit/AppKit.h>

NS_ASSUME_NONNULL_BEGIN

@interface XCWChromeRenderer : NSObject

+ (nullable NSData *)PNGDataForDeviceName:(NSString *)deviceName
                                    error:(NSError * _Nullable * _Nullable)error;
+ (nullable NSData *)PNGDataForDeviceName:(NSString *)deviceName
                            includeButtons:(BOOL)includeButtons
                                      error:(NSError * _Nullable * _Nullable)error;
+ (nullable NSData *)buttonPNGDataForDeviceName:(NSString *)deviceName
                                     buttonName:(NSString *)buttonName
                                        pressed:(BOOL)pressed
                                          error:(NSError * _Nullable * _Nullable)error;
+ (nullable NSData *)screenMaskPNGDataForDeviceName:(NSString *)deviceName
                                              error:(NSError * _Nullable * _Nullable)error;
+ (nullable NSData *)screenshotPNGDataForDeviceName:(NSString *)deviceName
                                      screenPNGData:(NSData *)screenPNGData
                                              error:(NSError * _Nullable * _Nullable)error;
+ (nullable NSDictionary<NSString *, id> *)profileForDeviceName:(NSString *)deviceName
                                                          error:(NSError * _Nullable * _Nullable)error;

@end

NS_ASSUME_NONNULL_END
