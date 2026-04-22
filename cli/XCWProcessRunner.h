#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface XCWProcessResult : NSObject

@property (nonatomic, readonly) int terminationStatus;
@property (nonatomic, copy, readonly) NSData *stdoutData;
@property (nonatomic, copy, readonly) NSData *stderrData;
@property (nonatomic, copy, readonly) NSString *stdoutString;
@property (nonatomic, copy, readonly) NSString *stderrString;

- (instancetype)initWithTerminationStatus:(int)terminationStatus
                               stdoutData:(NSData *)stdoutData
                               stderrData:(NSData *)stderrData NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;

@end

@interface XCWProcessRunner : NSObject

+ (XCWProcessResult *)runLaunchPath:(NSString *)launchPath
                          arguments:(NSArray<NSString *> *)arguments
                          inputData:(nullable NSData *)inputData
                              error:(NSError * _Nullable * _Nullable)error;

@end

NS_ASSUME_NONNULL_END
