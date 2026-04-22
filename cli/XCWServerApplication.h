#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface XCWServerApplication : NSObject

- (instancetype)initWithPort:(uint16_t)port
                 clientRoot:(NSString *)clientRoot NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;

- (BOOL)start:(NSError * _Nullable * _Nullable)error;

@end

NS_ASSUME_NONNULL_END
