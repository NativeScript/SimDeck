#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

typedef NS_ENUM(NSInteger, DFDTUHIDTouchPhase) {
    DFDTUHIDTouchPhaseDown,
    DFDTUHIDTouchPhaseUp,
};

/// Input for simulators whose guest routes HID through `dtuhidd` (CoreSimulator 1155.4, Xcode 27, and later).
/// Those guests accept legacy Indigo messages from SimulatorKit's HID client and then drop them, so touches, buttons
/// and keys must go to `dtuhidd`'s digitizer service as plain XPC dictionaries instead.
@interface DFDTUHIDClient : NSObject

/// Whether the CoreSimulator loaded in this process routes guest input through `dtuhidd`.
+ (BOOL)isRequiredByLoadedCoreSimulator;

/// Connects to the device's digitizer service and waits until `dtuhidd` answers, retrying while it starts.
- (nullable instancetype)initWithDevice:(id)device error:(NSError **)error;

/// One contact at a normalized top-left point in 0...1. The first down starts the contact, later downs move it.
- (BOOL)sendTouchPhase:(DFDTUHIDTouchPhase)phase
                     x:(double)x
                     y:(double)y
                  edge:(uint32_t)edge
                 error:(NSError **)error;

/// Two contacts at normalized points, tracked separately from the single contact.
- (BOOL)sendTwoFingerTouchPhase:(DFDTUHIDTouchPhase)phase
                             x1:(double)x1
                             y1:(double)y1
                             x2:(double)x2
                             y2:(double)y2
                          error:(NSError **)error;

/// A hardware button identified by its HID usage page and usage (Home is consumer 0x0c / 0x40).
- (BOOL)sendButtonUsagePage:(uint32_t)usagePage usage:(uint32_t)usage down:(BOOL)down error:(NSError **)error;

/// A USB HID keyboard usage.
- (BOOL)sendKeyboardUsage:(uint32_t)usage down:(BOOL)down error:(NSError **)error;

/// Whether the connection is still usable; `dtuhidd` invalidates it when the simulator shuts down.
@property (nonatomic, readonly, getter=isValid) BOOL valid;

- (void)disconnect;

@end

NS_ASSUME_NONNULL_END
