#import "DFDTUHIDClient.h"

#import <dlfcn.h>
#import <mach/mach.h>
#import <objc/message.h>
#import <xpc/xpc.h>

static NSString *const DFDTUHIDErrorDomain = @"SimDeck.DTUHID";
static NSString *const DFDTUHIDDigitizerService = @"com.apple.coredevice.feature.remote.hid.digitizer";
static NSString *const DFDTUHIDFirstCoreSimulatorVersion = @"1155.4";

// dtuhidd decodes these payload enums from uint64. HIDButtonState is 1-based; 0 is rejected.
static const uint64_t DFDTUHIDContactStart = 0;
static const uint64_t DFDTUHIDContactPosition = 1;
static const uint64_t DFDTUHIDContactEnd = 2;
static const uint64_t DFDTUHIDButtonDown = 1;
static const uint64_t DFDTUHIDButtonUp = 2;

// dtuhidd is demand-launched and aborts if it starts before the guest has a display, after which launchd throttles
// its respawn while the service lookup keeps succeeding. Only an answered request proves a live daemon.
static const int64_t DFDTUHIDLivenessTimeoutNs = 3 * NSEC_PER_SEC;
static const useconds_t DFDTUHIDLivenessBackoffUs = 3 * 1000 * 1000;
static const NSUInteger DFDTUHIDLivenessAttempts = 4;
static const useconds_t DFDTUHIDDeviceOpenTailUs = 200 * 1000;
static const int64_t DFDTUHIDSendBarrierTimeoutNs = 2 * NSEC_PER_SEC;

typedef xpc_object_t (*DFXPCEndpointCreateMachPort4SimFn)(mach_port_t port, uint64_t a, uint64_t b);
typedef xpc_connection_t (*DFXPCConnectionCreateFromEndpointFn)(xpc_object_t endpoint);
typedef void (*DFXPCConnectionEnableSim2Host4SimFn)(xpc_connection_t connection);

static NSError *DFDTUHIDError(NSString *message) {
    return [NSError errorWithDomain:DFDTUHIDErrorDomain code:1 userInfo:@{ NSLocalizedDescriptionKey: message }];
}

static xpc_object_t DFDTUHIDEnvelope(const char *messageType, BOOL isBarrier, xpc_object_t payload) {
    xpc_object_t message = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(message, "messageType", messageType);
    xpc_dictionary_set_bool(message, "isBarrier", isBarrier);
    xpc_dictionary_set_string(message, "featureIdentifier", DFDTUHIDDigitizerService.UTF8String);
    xpc_dictionary_set_value(message, "payload", payload);
    return message;
}

static xpc_object_t DFDTUHIDPoint(double x, double y) {
    xpc_object_t point = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_double(point, "x", fmax(0.0, fmin(1.0, x)));
    xpc_dictionary_set_double(point, "y", fmax(0.0, fmin(1.0, y)));
    return point;
}

static xpc_object_t DFDTUHIDKeyboardPayload(uint64_t usage, uint64_t state) {
    xpc_object_t payload = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_uint64(payload, "usageCode", usage);
    xpc_dictionary_set_uint64(payload, "state", state);
    return payload;
}

@implementation DFDTUHIDClient {
    xpc_connection_t _connection;
    dispatch_queue_t _queue;
    // Guarded by _queue.
    BOOL _invalidated;
    NSUInteger _interruptions;
    BOOL _contactDown;
    NSUInteger _contactInterruptions;
    BOOL _twoFingerDown;
    NSUInteger _twoFingerInterruptions;
}

+ (BOOL)isRequiredByLoadedCoreSimulator {
    Class simDeviceClass = NSClassFromString(@"SimDevice");
    if (simDeviceClass == Nil) {
        return NO;
    }
    // CoreSimulator is a system framework the Xcode installer replaces, so the loaded bundle decides, not the
    // selected Xcode.
    NSString *version = [NSBundle bundleForClass:simDeviceClass].infoDictionary[@"CFBundleVersion"];
    if (![version isKindOfClass:[NSString class]] || version.length == 0) {
        return NO;
    }
    return [version compare:DFDTUHIDFirstCoreSimulatorVersion options:NSNumericSearch] != NSOrderedAscending;
}

- (nullable instancetype)initWithDevice:(id)device error:(NSError **)error {
    self = [super init];
    if (self == nil) {
        return nil;
    }
    _queue = dispatch_queue_create("SimDeck.DTUHID", DISPATCH_QUEUE_SERIAL);

    NSError *lastError = nil;
    for (NSUInteger attempt = 1; attempt <= DFDTUHIDLivenessAttempts; attempt++) {
        xpc_connection_t connection = [self connectToDevice:device error:&lastError];
        if (connection != nil && [self confirmLiveness:connection error:&lastError]) {
            _connection = connection;
            usleep(DFDTUHIDDeviceOpenTailUs);
            return self;
        }
        if (connection != nil) {
            xpc_connection_cancel(connection);
        }
        if (attempt < DFDTUHIDLivenessAttempts) {
            usleep(DFDTUHIDLivenessBackoffUs);
        }
    }
    if (error != NULL) {
        *error = lastError ?: DFDTUHIDError(@"dtuhidd did not answer.");
    }
    return nil;
}

- (nullable xpc_connection_t)connectToDevice:(id)device error:(NSError **)error {
    DFXPCEndpointCreateMachPort4SimFn endpointFromPort =
        (DFXPCEndpointCreateMachPort4SimFn)dlsym(RTLD_DEFAULT, "xpc_endpoint_create_mach_port_4sim");
    DFXPCConnectionCreateFromEndpointFn connectionFromEndpoint =
        (DFXPCConnectionCreateFromEndpointFn)dlsym(RTLD_DEFAULT, "xpc_connection_create_from_endpoint");
    DFXPCConnectionEnableSim2Host4SimFn enableSim2Host =
        (DFXPCConnectionEnableSim2Host4SimFn)dlsym(RTLD_DEFAULT, "xpc_connection_enable_sim2host_4sim");
    if (endpointFromPort == NULL || connectionFromEndpoint == NULL || enableSim2Host == NULL) {
        if (error != NULL) {
            *error = DFDTUHIDError(@"libxpc does not export the simulator endpoint functions dtuhidd needs.");
        }
        return nil;
    }

    NSError *lookupError = nil;
    mach_port_t port = ((mach_port_t(*)(id, SEL, NSString *, NSError **))objc_msgSend)(
        device, sel_registerName("lookup:error:"), DFDTUHIDDigitizerService, &lookupError);
    if (port == MACH_PORT_NULL) {
        if (error != NULL) {
            *error = DFDTUHIDError([NSString stringWithFormat:@"The simulator does not vend %@: %@",
                                                             DFDTUHIDDigitizerService,
                                                             lookupError.localizedDescription ?: @"lookup failed"]);
        }
        return nil;
    }

    // The endpoint takes the lookup's send right.
    xpc_object_t endpoint = endpointFromPort(port, 0, 0);
    xpc_connection_t connection = endpoint != nil ? connectionFromEndpoint(endpoint) : nil;
    if (connection == nil) {
        if (error != NULL) {
            *error = DFDTUHIDError(@"Could not create an XPC connection to dtuhidd.");
        }
        return nil;
    }
    // Without this the guest sees the peer but never a payload.
    enableSim2Host(connection);

    __weak DFDTUHIDClient *weakSelf = self;
    xpc_connection_set_target_queue(connection, _queue);
    xpc_connection_set_event_handler(connection, ^(xpc_object_t event) {
        DFDTUHIDClient *strongSelf = weakSelf;
        if (strongSelf == nil || xpc_get_type(event) != XPC_TYPE_ERROR) {
            return;
        }
        if (event == XPC_ERROR_CONNECTION_INTERRUPTED) {
            strongSelf->_interruptions += 1;
        } else if (event == XPC_ERROR_CONNECTION_INVALID) {
            strongSelf->_invalidated = YES;
        }
    });
    xpc_connection_resume(connection);
    return connection;
}

/// A barrier carrying keyboard usage 0 ("no event"), so the daemon answers without the guest seeing a key.
- (BOOL)confirmLiveness:(xpc_connection_t)connection error:(NSError **)error {
    xpc_object_t probe = DFDTUHIDEnvelope("IndigoKeyboardButtonEvent", YES, DFDTUHIDKeyboardPayload(0, DFDTUHIDButtonUp));
    dispatch_semaphore_t answered = dispatch_semaphore_create(0);
    __block BOOL alive = NO;
    xpc_connection_send_message_with_reply(connection, probe, _queue, ^(xpc_object_t reply) {
        alive = xpc_get_type(reply) == XPC_TYPE_DICTIONARY;
        dispatch_semaphore_signal(answered);
    });
    if (dispatch_semaphore_wait(answered, dispatch_time(DISPATCH_TIME_NOW, DFDTUHIDLivenessTimeoutNs)) != 0 || !alive) {
        if (error != NULL) {
            *error = DFDTUHIDError(@"dtuhidd did not answer its liveness probe.");
        }
        return NO;
    }
    return YES;
}

- (BOOL)isValid {
    __block BOOL valid = NO;
    dispatch_sync(_queue, ^{
        valid = self->_connection != nil && !self->_invalidated;
    });
    return valid;
}

- (void)disconnect {
    dispatch_sync(_queue, ^{
        if (self->_connection != nil) {
            xpc_connection_cancel(self->_connection);
            self->_connection = nil;
        }
        self->_invalidated = YES;
    });
}

- (void)dealloc {
    if (_connection != nil) {
        xpc_connection_cancel(_connection);
    }
}

/// Sends `message` and waits until XPC has handed it to the guest, so a later event never overtakes it.
- (BOOL)send:(xpc_object_t)message error:(NSError **)error {
    __block xpc_connection_t connection = nil;
    dispatch_sync(_queue, ^{
        if (self->_connection != nil && !self->_invalidated) {
            connection = self->_connection;
            xpc_connection_send_message(connection, message);
        }
    });
    if (connection == nil) {
        if (error != NULL) {
            *error = DFDTUHIDError(@"The dtuhidd connection is closed.");
        }
        return NO;
    }
    // The barrier block runs on _queue, so this wait must not happen on it.
    dispatch_semaphore_t sent = dispatch_semaphore_create(0);
    xpc_connection_send_barrier(connection, ^{
        dispatch_semaphore_signal(sent);
    });
    if (dispatch_semaphore_wait(sent, dispatch_time(DISPATCH_TIME_NOW, DFDTUHIDSendBarrierTimeoutNs)) != 0) {
        if (error != NULL) {
            *error = DFDTUHIDError(@"Timed out sending to dtuhidd.");
        }
        return NO;
    }
    return YES;
}

/// Maps down/up onto dtuhidd's start/position/end. dtuhidd forgets a contact when the connection is interrupted, so a
/// down that would continue a contact from before an interruption starts a new one instead.
- (uint64_t)contactEventForPhase:(DFDTUHIDTouchPhase)phase twoFinger:(BOOL)twoFinger {
    __block uint64_t eventType = DFDTUHIDContactEnd;
    dispatch_sync(_queue, ^{
        BOOL *down = twoFinger ? &self->_twoFingerDown : &self->_contactDown;
        NSUInteger *startedAt = twoFinger ? &self->_twoFingerInterruptions : &self->_contactInterruptions;
        if (phase == DFDTUHIDTouchPhaseUp) {
            *down = NO;
            eventType = DFDTUHIDContactEnd;
        } else if (*down && *startedAt == self->_interruptions) {
            eventType = DFDTUHIDContactPosition;
        } else {
            *down = YES;
            *startedAt = self->_interruptions;
            eventType = DFDTUHIDContactStart;
        }
    });
    return eventType;
}

- (BOOL)sendTouchPhase:(DFDTUHIDTouchPhase)phase x:(double)x y:(double)y edge:(uint32_t)edge error:(NSError **)error {
    xpc_object_t payload = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_value(payload, "pointOne", DFDTUHIDPoint(x, y));
    xpc_dictionary_set_uint64(payload, "eventType", [self contactEventForPhase:phase twoFinger:NO]);
    xpc_dictionary_set_uint64(payload, "edge", edge);
    xpc_dictionary_set_uint64(payload, "target", 0);
    return [self send:DFDTUHIDEnvelope("IndigoDigitizerEvent", NO, payload) error:error];
}

- (BOOL)sendTwoFingerTouchPhase:(DFDTUHIDTouchPhase)phase
                             x1:(double)x1
                             y1:(double)y1
                             x2:(double)x2
                             y2:(double)y2
                          error:(NSError **)error {
    xpc_object_t payload = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_value(payload, "pointOne", DFDTUHIDPoint(x1, y1));
    xpc_dictionary_set_value(payload, "pointTwo", DFDTUHIDPoint(x2, y2));
    xpc_dictionary_set_uint64(payload, "eventType", [self contactEventForPhase:phase twoFinger:YES]);
    xpc_dictionary_set_uint64(payload, "edge", 0);
    xpc_dictionary_set_uint64(payload, "target", 0);
    return [self send:DFDTUHIDEnvelope("IndigoDigitizerEvent", NO, payload) error:error];
}

- (BOOL)sendButtonUsagePage:(uint32_t)usagePage usage:(uint32_t)usage down:(BOOL)down error:(NSError **)error {
    xpc_object_t payload = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_uint64(payload, "usagePage", usagePage);
    xpc_dictionary_set_uint64(payload, "usageCode", usage);
    xpc_dictionary_set_uint64(payload, "state", down ? DFDTUHIDButtonDown : DFDTUHIDButtonUp);
    return [self send:DFDTUHIDEnvelope("IndigoButtonEvent", NO, payload) error:error];
}

- (BOOL)sendKeyboardUsage:(uint32_t)usage down:(BOOL)down error:(NSError **)error {
    xpc_object_t payload = DFDTUHIDKeyboardPayload(usage, down ? DFDTUHIDButtonDown : DFDTUHIDButtonUp);
    return [self send:DFDTUHIDEnvelope("IndigoKeyboardButtonEvent", NO, payload) error:error];
}

@end
