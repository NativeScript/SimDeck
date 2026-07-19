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

typedef NS_ENUM(NSInteger, DFPrivateSimulatorTouchEdge) {
    DFPrivateSimulatorTouchEdgeNone = 0,
    DFPrivateSimulatorTouchEdgeLeft = 1,
    DFPrivateSimulatorTouchEdgeTop = 2,
    DFPrivateSimulatorTouchEdgeBottom = 3,
    DFPrivateSimulatorTouchEdgeRight = 4,
} NS_SWIFT_NAME(PrivateSimulatorTouchEdge);

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
                                 error:(NSError * _Nullable * _Nullable)error;
- (nullable instancetype)initWithUDID:(NSString *)udid
            preferredScreenDeviceName:(nullable NSString *)preferredScreenDeviceName
                         attachDisplay:(BOOL)attachDisplay
                                 error:(NSError * _Nullable * _Nullable)error NS_DESIGNATED_INITIALIZER
    NS_SWIFT_NAME(init(udid:preferredScreenDeviceName:attachDisplay:));

@property (nonatomic, weak, nullable) id<DFPrivateSimulatorDisplayBridgeDelegate> delegate;
@property (nonatomic, readonly, getter=isDisplayReady) BOOL displayReady;
@property (nonatomic, readonly) NSString *displayStatus;
@property (nonatomic, readonly) CGSize displaySize;
@property (nonatomic, readonly) NSInteger rotationQuarterTurns;

- (void)seedRotationQuarterTurns:(NSInteger)quarterTurns NS_SWIFT_NAME(seedRotation(quarterTurns:));
- (void)updateInputDisplaySize:(CGSize)displaySize;

- (nullable CVPixelBufferRef)copyPixelBuffer CF_RETURNS_RETAINED;

- (BOOL)sendTouchAtNormalizedX:(double)normalizedX
                   normalizedY:(double)normalizedY
                         phase:(DFPrivateSimulatorTouchPhase)phase
                         error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendTouch(normalizedX:normalizedY:phase:));

- (BOOL)sendEdgeTouchAtNormalizedX:(double)normalizedX
                        normalizedY:(double)normalizedY
                              phase:(DFPrivateSimulatorTouchPhase)phase
                               edge:(DFPrivateSimulatorTouchEdge)edge
                              error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendEdgeTouch(normalizedX:normalizedY:phase:edge:));

- (BOOL)sendMultiTouchAtNormalizedX1:(double)normalizedX1
                          normalizedY1:(double)normalizedY1
                          normalizedX2:(double)normalizedX2
                          normalizedY2:(double)normalizedY2
                                phase:(DFPrivateSimulatorTouchPhase)phase
                                error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendMultiTouch(normalizedX1:normalizedY1:normalizedX2:normalizedY2:phase:));

- (BOOL)sendScrollWithDeltaX:(double)deltaX
                      deltaY:(double)deltaY
                 normalizedX:(double)normalizedX
                 normalizedY:(double)normalizedY
                       error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendScroll(deltaX:deltaY:normalizedX:normalizedY:));

- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(NSUInteger)modifiers
              error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendKey(keyCode:modifiers:));
- (BOOL)sendKeyCode:(uint16_t)keyCode
                down:(BOOL)down
               error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendKey(keyCode:down:));

- (BOOL)pressHomeButton:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(pressHomeButton());
- (BOOL)openAppSwitcher:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(openAppSwitcher());
- (BOOL)pressHardwareButtonNamed:(NSString *)buttonName
                       durationMs:(NSUInteger)durationMs
                            error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(pressHardwareButton(named:durationMs:));
- (BOOL)sendHardwareButtonNamed:(NSString *)buttonName
                         pressed:(BOOL)pressed
                       usagePage:(nullable NSNumber *)usagePage
                           usage:(nullable NSNumber *)usage
                           error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendHardwareButton(named:pressed:usagePage:usage:));
- (BOOL)rotateDigitalCrownByDelta:(double)delta
                             error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(rotateDigitalCrown(delta:));

- (BOOL)rotateRight:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(rotateRight());
- (BOOL)rotateLeft:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(rotateLeft());
- (BOOL)rotateByDegrees:(double)deltaDegrees
                  error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(rotate(byDegrees:));

- (void)disconnect;

@end

NS_ASSUME_NONNULL_END
