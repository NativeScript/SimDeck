#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

typedef void (^XCWPrivateSimulatorEncodedFrameHandler)(NSData *sampleData,
                                                       NSUInteger frameSequence,
                                                       uint64_t timestampUs,
                                                       BOOL isKeyFrame,
                                                       NSString * _Nullable codec,
                                                       NSData * _Nullable decoderConfig,
                                                       CGSize dimensions);

@interface XCWPrivateSimulatorSession : NSObject

- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;
- (nullable instancetype)initWithUDID:(NSString *)udid
                        simulatorName:(NSString *)simulatorName
                                error:(NSError * _Nullable * _Nullable)error NS_DESIGNATED_INITIALIZER;

@property (nonatomic, copy, readonly) NSString *udid;
@property (nonatomic, copy, readonly) NSString *simulatorName;
@property (nonatomic, readonly, getter=isDisplayReady) BOOL displayReady;
@property (nonatomic, copy, readonly) NSString *displayStatus;
@property (nonatomic, readonly) CGSize displaySize;
@property (nonatomic, readonly) NSUInteger frameSequence;

- (BOOL)waitUntilReadyWithTimeout:(NSTimeInterval)timeout;
- (BOOL)waitForFirstEncodedFrameWithTimeout:(NSTimeInterval)timeout;
- (NSDictionary *)sessionInfoRepresentation;
- (nullable NSDictionary *)latestEncodedKeyFrameRepresentation;
- (void)refreshCurrentFrame;
- (void)requestKeyFrameRefresh;
- (id)addEncodedFrameListener:(XCWPrivateSimulatorEncodedFrameHandler)handler;
- (void)removeEncodedFrameListener:(id)token;

- (BOOL)sendTouchWithNormalizedX:(double)normalizedX
                     normalizedY:(double)normalizedY
                           phase:(NSString *)phase
                           error:(NSError * _Nullable * _Nullable)error;

- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(NSUInteger)modifiers
              error:(NSError * _Nullable * _Nullable)error;

- (BOOL)pressHomeButton:(NSError * _Nullable * _Nullable)error;
- (BOOL)rotateRight:(NSError * _Nullable * _Nullable)error;
- (void)disconnect;

@end

NS_ASSUME_NONNULL_END
