#import <AppKit/AppKit.h>
#import <CoreVideo/CoreVideo.h>

NS_ASSUME_NONNULL_BEGIN

@class DFPrivateSimulatorDisplayBridge;
@class DFPrivateSimulatorChromeButton;

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
- (nullable instancetype)initWithUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error NS_DESIGNATED_INITIALIZER NS_SWIFT_NAME(init(udid:));

@property (nonatomic, weak, nullable) id<DFPrivateSimulatorDisplayBridgeDelegate> delegate;
@property (nonatomic, readonly) NSView *displayView;
@property (nonatomic, readonly, getter=isDisplayReady) BOOL displayReady;
@property (nonatomic, readonly) NSString *displayStatus;

- (void)activateDisplayIfNeeded;
- (nullable CVPixelBufferRef)copyPixelBuffer CF_RETURNS_RETAINED;

- (BOOL)sendTouchAtNormalizedX:(double)normalizedX
                   normalizedY:(double)normalizedY
                         phase:(DFPrivateSimulatorTouchPhase)phase
                         error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendTouch(normalizedX:normalizedY:phase:));

- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(NSUInteger)modifiers
              error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendKey(keyCode:modifiers:));

- (BOOL)sendKeyEvent:(NSEvent *)event
               error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(sendKey(event:));

- (NSArray<DFPrivateSimulatorChromeButton *> *)availableChromeButtons NS_SWIFT_NAME(availableChromeButtons());
- (BOOL)pressChromeButtonWithIdentifier:(NSString *)identifier
                                  error:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(pressChromeButton(identifier:));

- (BOOL)pressHomeButton:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(pressHomeButton());

- (BOOL)rotateRight:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(rotateRight());
- (BOOL)rotateLeft:(NSError * _Nullable * _Nullable)error NS_SWIFT_NAME(rotateLeft());

- (void)disconnect;

@end

NS_SWIFT_NAME(PrivateSimulatorChromeButton)
@interface DFPrivateSimulatorChromeButton : NSObject

- (instancetype)init NS_UNAVAILABLE;
- (instancetype)new NS_UNAVAILABLE;

@property (nonatomic, copy, readonly) NSString *identifier;
@property (nonatomic, copy, readonly) NSString *title;
@property (nonatomic, copy, readonly) NSString *toolTip;
@property (nonatomic, copy, readonly) NSString *accessibilityLabel;
@property (nonatomic, copy, readonly) NSString *summary;

@end

NS_ASSUME_NONNULL_END
