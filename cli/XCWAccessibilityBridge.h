#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface XCWAccessibilityBridge : NSObject

+ (nullable NSDictionary *)accessibilitySnapshotForSimulatorUDID:(NSString *)udid
                                                         atPoint:(nullable NSValue *)pointValue
                                                        maxDepth:(NSUInteger)maxDepth
                                                           error:(NSError * _Nullable * _Nullable)error;

@end

NS_ASSUME_NONNULL_END
