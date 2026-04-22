#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface XCWPrivateSimulatorBooter : NSObject

- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;

+ (BOOL)bootDeviceWithUDID:(NSString *)udid
                     error:(NSError * _Nullable * _Nullable)error;

@end

NS_ASSUME_NONNULL_END
