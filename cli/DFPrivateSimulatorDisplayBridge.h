#import <AppKit/AppKit.h>
#import <CoreVideo/CoreVideo.h>

NS_ASSUME_NONNULL_BEGIN

@class DFPrivateSimulatorDisplayBridge;

typedef NS_ENUM(NSInteger, DFPrivateSimulatorTouchPhase) {
    DFPrivateSimulatorTouchPhaseBegan = 0,
    DFPrivateSimulatorTouchPhaseMoved = 1,
    DFPrivateSimulatorTouchPhaseEnded = 2,
    DFPrivateSimulatorTouchPhaseCancelled = 3,
} NS_SWIFT_NAME(PrivateSimulatorTouchPhase);

NS_SWIFT_NAME(PrivateSimulatorDisplayBridgeDelegate)
@protocol DFPrivateSimulatorDisplayBridgeDelegate <NSObject>

- (void)privateSimulatorDisplayBridge:(DFPrivateSimulatorDisplayBridge *)bridge
                      didUpdateFrame:(CVPixelBufferRef)pixelBuffer NS_SWIFT_NAME(privateSimulatorDisplayBridge(_:didUpdateFrame:));

- (void)privateSimulatorDisplayBridge:(DFPrivateSimulatorDisplayBridge *)bridge
                didChangeDisplayStatus:(NSString *)status
                               isReady:(BOOL)isReady NS_SWIFT_NAME(privateSimulatorDisplayBridge(_:didChangeDisplayStatus:isReady:));

@end

NS_SWIFT_NAME(PrivateSimulatorDisplayBridge)
@interface DFPrivateSimulatorDisplayBridge : NSObject

- (instancetype)init NS_UNAVAILABLE;
- (instancetype)new NS_UNAVAILABLE;
- (nullable instancetype)initWithUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(init(udid:));
- (nullable instancetype)initWithUDID:(NSString *)udid
                         attachDisplay:(BOOL)attachDisplay
                                 error:(NSError * _Nullable * _Nullable)error NS_DESIGNATED_INITIALIZER;

@property (nonatomic, weak, nullable) id<DFPrivateSimulatorDisplayBridgeDelegate> delegate;
@property (nonatomic, readonly, getter=isDisplayReady) BOOL displayReady;
@property (nonatomic, readonly) NSString *displayStatus;

- (nullable CVPixelBufferRef)copyPixelBuffer CF_RETURNS_RETAINED;

- (BOOL)sendTouchAtNormalizedX:(double)normalizedX
                   normalizedY:(double)normalizedY
                         phase:(DFPrivateSimulatorTouchPhase)phase
                         error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendTouch(normalizedX:normalizedY:phase:));

- (BOOL)sendMultiTouchAtNormalizedX1:(double)normalizedX1
                          normalizedY1:(double)normalizedY1
                          normalizedX2:(double)normalizedX2
                          normalizedY2:(double)normalizedY2
                                phase:(DFPrivateSimulatorTouchPhase)phase
                                error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendMultiTouch(normalizedX1:normalizedY1:normalizedX2:normalizedY2:phase:));

- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(NSUInteger)modifiers
              error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendKey(keyCode:modifiers:));
- (BOOL)sendKeyCode:(uint16_t)keyCode
                down:(BOOL)down
               error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendKey(keyCode:down:));

- (BOOL)pressHomeButton:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(pressHomeButton());
- (BOOL)pressHardwareButtonNamed:(NSString *)buttonName
                       durationMs:(NSUInteger)durationMs
                            error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(pressHardwareButton(named:durationMs:));

- (BOOL)rotateRight:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(rotateRight());
- (BOOL)rotateLeft:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(rotateLeft());

- (void)disconnect;

@end

NS_ASSUME_NONNULL_END
