#import "XCWServerApplication.h"

#import <AppKit/AppKit.h>

#import "XCWChromeRenderer.h"
#import "XCWHTTPServer.h"
#import "XCWPrivateSimulatorSession.h"
#import "XCWSimctl.h"

@interface XCWServerApplication ()

@property (nonatomic, readonly) uint16_t port;
@property (nonatomic, copy, readonly) NSString *clientRoot;
@property (nonatomic, strong, readonly) XCWSimctl *simctl;
@property (nonatomic, strong) XCWHTTPServer *httpServer;
@property (nonatomic, strong, readonly) NSMutableDictionary<NSString *, XCWPrivateSimulatorSession *> *sessionsByUDID;

@end

@implementation XCWServerApplication

- (instancetype)initWithPort:(uint16_t)port clientRoot:(NSString *)clientRoot {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _port = port;
    _clientRoot = [clientRoot stringByStandardizingPath];
    _simctl = [[XCWSimctl alloc] init];
    _sessionsByUDID = [NSMutableDictionary dictionary];
    return self;
}

- (BOOL)start:(NSError * _Nullable __autoreleasing *)error {
    __weak typeof(self) weakSelf = self;
    self.httpServer = [[XCWHTTPServer alloc] initWithPort:self.port requestHandler:^XCWHTTPResponse * _Nonnull(XCWHTTPRequest *request) {
        __strong typeof(weakSelf) strongSelf = weakSelf;
        return [strongSelf responseForRequest:request];
    }];
    return [self.httpServer start:error];
}

- (XCWHTTPResponse *)responseForRequest:(XCWHTTPRequest *)request {
    if ([request.method isEqualToString:@"OPTIONS"]) {
        return [[XCWHTTPResponse alloc] initWithStatusCode:204 headers:@{} body:[NSData data]];
    }

    if ([request.path hasPrefix:@"/api/"]) {
        return [self apiResponseForRequest:request];
    }

    return [self staticResponseForRequest:request];
}

- (XCWHTTPResponse *)apiResponseForRequest:(XCWHTTPRequest *)request {
    if ([request.path isEqualToString:@"/api/health"]) {
        return [XCWHTTPResponse JSONResponseWithObject:@{
            @"ok": @YES,
            @"port": @(self.port),
            @"timestamp": @([[NSDate date] timeIntervalSince1970]),
        } statusCode:200];
    }

    NSArray<NSString *> *components = [[request.path componentsSeparatedByString:@"/"] filteredArrayUsingPredicate:[NSPredicate predicateWithFormat:@"length > 0"]];
    if (components.count == 2 &&
        [components[0] isEqualToString:@"api"] &&
        [components[1] isEqualToString:@"simulators"] &&
        [request.method isEqualToString:@"GET"]) {
        NSError *error = nil;
        NSArray<NSDictionary *> *simulators = [self.simctl listSimulatorsWithError:&error];
        if (simulators == nil) {
            return [self errorResponse:error statusCode:500];
        }
        [self warmPrivateSessionsForBootedSimulators:simulators];
        return [XCWHTTPResponse JSONResponseWithObject:@{ @"simulators": [self enrichedSimulators:simulators] } statusCode:200];
    }

    if (components.count >= 4 &&
        [components[0] isEqualToString:@"api"] &&
        [components[1] isEqualToString:@"simulators"]) {
        NSString *udid = components[2];
        NSString *action = components[3];

        if ([request.method isEqualToString:@"POST"] && [action isEqualToString:@"boot"]) {
            NSError *error = nil;
            if (![self.simctl bootSimulatorWithUDID:udid error:&error]) {
                return [self errorResponse:error statusCode:500];
            }
            XCWPrivateSimulatorSession *session = [self ensureSessionForUDID:udid waitForReady:NO error:nil];
            if (session != nil) {
                [session waitUntilReadyWithTimeout:5.0];
            }
            return [self simulatorPayloadResponseForUDID:udid];
        }

        if ([request.method isEqualToString:@"POST"] && [action isEqualToString:@"shutdown"]) {
            [self removeSessionForUDID:udid];
            NSError *error = nil;
            if (![self.simctl shutdownSimulatorWithUDID:udid error:&error]) {
                return [self errorResponse:error statusCode:500];
            }
            return [self simulatorPayloadResponseForUDID:udid];
        }

        if ([request.method isEqualToString:@"POST"] && [action isEqualToString:@"open-url"]) {
            NSDictionary *payload = [self validatedJSONObjectBodyForRequest:request errorResponse:nil];
            NSString *urlString = [payload isKindOfClass:[NSDictionary class]] ? payload[@"url"] : nil;
            if (urlString.length == 0) {
                return [self errorResponse:[NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                               code:1
                                                           userInfo:@{ NSLocalizedDescriptionKey: @"Request body must include `url`." }]
                                 statusCode:400];
            }

            NSError *error = nil;
            if (![self.simctl openURL:urlString simulatorUDID:udid error:&error]) {
                return [self errorResponse:error statusCode:500];
            }
            return [self simulatorPayloadResponseForUDID:udid];
        }

        if ([request.method isEqualToString:@"POST"] && [action isEqualToString:@"launch"]) {
            NSDictionary *payload = [self validatedJSONObjectBodyForRequest:request errorResponse:nil];
            NSString *bundleID = [payload isKindOfClass:[NSDictionary class]] ? payload[@"bundleId"] : nil;
            if (bundleID.length == 0) {
                return [self errorResponse:[NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                               code:2
                                                           userInfo:@{ NSLocalizedDescriptionKey: @"Request body must include `bundleId`." }]
                                 statusCode:400];
            }

            NSError *error = nil;
            if (![self.simctl launchBundleID:bundleID simulatorUDID:udid error:&error]) {
                return [self errorResponse:error statusCode:500];
            }
            return [self simulatorPayloadResponseForUDID:udid];
        }

        if ([request.method isEqualToString:@"POST"] && [action isEqualToString:@"touch"]) {
            XCWPrivateSimulatorSession *session = [self ensureSessionForUDID:udid waitForReady:YES error:nil];
            if (session == nil) {
                return [self errorResponse:[NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                               code:3
                                                           userInfo:@{ NSLocalizedDescriptionKey: @"Unable to attach to the private simulator display." }]
                                 statusCode:500];
            }

            NSDictionary *payload = [self validatedJSONObjectBodyForRequest:request errorResponse:nil];
            NSNumber *x = payload[@"x"];
            NSNumber *y = payload[@"y"];
            NSString *phase = payload[@"phase"];
            if (x == nil || y == nil || phase.length == 0) {
                return [self errorResponse:[NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                               code:4
                                                           userInfo:@{ NSLocalizedDescriptionKey: @"Touch payload must include numeric `x`, `y`, and string `phase`." }]
                                 statusCode:400];
            }

            NSError *error = nil;
            if (![session sendTouchWithNormalizedX:x.doubleValue normalizedY:y.doubleValue phase:phase error:&error]) {
                return [self errorResponse:error statusCode:500];
            }
            return [XCWHTTPResponse JSONResponseWithObject:@{ @"ok": @YES } statusCode:200];
        }

        if ([request.method isEqualToString:@"POST"] && [action isEqualToString:@"key"]) {
            XCWPrivateSimulatorSession *session = [self ensureSessionForUDID:udid waitForReady:YES error:nil];
            if (session == nil) {
                return [self errorResponse:[NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                               code:5
                                                           userInfo:@{ NSLocalizedDescriptionKey: @"Unable to attach to the private simulator display." }]
                                 statusCode:500];
            }

            NSDictionary *payload = [self validatedJSONObjectBodyForRequest:request errorResponse:nil];
            NSNumber *keyCode = payload[@"keyCode"];
            NSNumber *modifiers = payload[@"modifiers"];
            if (keyCode == nil) {
                return [self errorResponse:[NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                               code:6
                                                           userInfo:@{ NSLocalizedDescriptionKey: @"Keyboard payload must include `keyCode`." }]
                                 statusCode:400];
            }

            NSError *error = nil;
            if (![session sendKeyCode:keyCode.unsignedShortValue modifiers:modifiers.unsignedIntegerValue error:&error]) {
                return [self errorResponse:error statusCode:500];
            }
            return [XCWHTTPResponse JSONResponseWithObject:@{ @"ok": @YES } statusCode:200];
        }

        if ([request.method isEqualToString:@"POST"] && [action isEqualToString:@"home"]) {
            XCWPrivateSimulatorSession *session = [self ensureSessionForUDID:udid waitForReady:YES error:nil];
            if (session == nil) {
                return [self errorResponse:[NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                               code:7
                                                           userInfo:@{ NSLocalizedDescriptionKey: @"Unable to attach to the private simulator display." }]
                                 statusCode:500];
            }

            NSError *error = nil;
            if (![session pressHomeButton:&error]) {
                return [self errorResponse:error statusCode:500];
            }
            return [XCWHTTPResponse JSONResponseWithObject:@{ @"ok": @YES } statusCode:200];
        }

        if ([request.method isEqualToString:@"POST"] && [action isEqualToString:@"rotate-right"]) {
            XCWPrivateSimulatorSession *session = [self ensureSessionForUDID:udid waitForReady:YES error:nil];
            if (session == nil) {
                return [self errorResponse:[NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                               code:8
                                                           userInfo:@{ NSLocalizedDescriptionKey: @"Unable to attach to the private simulator display." }]
                                 statusCode:500];
            }

            NSError *error = nil;
            if (![session rotateRight:&error]) {
                return [self errorResponse:error statusCode:500];
            }
            return [XCWHTTPResponse JSONResponseWithObject:@{ @"ok": @YES } statusCode:200];
        }

        if ([request.method isEqualToString:@"GET"] && [action isEqualToString:@"chrome-profile"]) {
            NSError *lookupError = nil;
            NSDictionary *simulator = [self.simctl simulatorWithUDID:udid error:&lookupError];
            if (simulator == nil) {
                return [self errorResponse:lookupError statusCode:404];
            }

            NSError *profileError = nil;
            NSDictionary *profile = [XCWChromeRenderer profileForDeviceName:simulator[@"name"] ?: @""
                                                                       error:&profileError];
            if (profile == nil) {
                return [self errorResponse:profileError statusCode:500];
            }
            return [XCWHTTPResponse JSONResponseWithObject:profile statusCode:200];
        }

        if ([request.method isEqualToString:@"GET"] && [action isEqualToString:@"chrome.png"]) {
            NSError *lookupError = nil;
            NSDictionary *simulator = [self.simctl simulatorWithUDID:udid error:&lookupError];
            if (simulator == nil) {
                return [self errorResponse:lookupError statusCode:404];
            }

            NSError *renderError = nil;
            NSData *pngData = [XCWChromeRenderer PNGDataForDeviceName:simulator[@"name"] ?: @""
                                                                error:&renderError];
            if (pngData == nil) {
                return [self errorResponse:renderError statusCode:500];
            }
            return [[XCWHTTPResponse alloc] initWithStatusCode:200
                                                       headers:@{
                @"Content-Type": @"image/png",
                @"Cache-Control": @"no-cache, no-store, must-revalidate",
            }
                                                          body:pngData];
        }
    }

    return [XCWHTTPResponse JSONResponseWithObject:@{ @"error": @"Not Found" } statusCode:404];
}

- (XCWHTTPResponse *)simulatorPayloadResponseForUDID:(NSString *)udid {
    NSError *error = nil;
    NSDictionary *simulator = [self.simctl simulatorWithUDID:udid error:&error];
    if (simulator == nil) {
        return [self errorResponse:error statusCode:404];
    }

    NSDictionary *enriched = [self enrichedSimulators:@[simulator]].firstObject ?: simulator;
    return [XCWHTTPResponse JSONResponseWithObject:@{ @"simulator": enriched } statusCode:200];
}

- (NSArray<NSDictionary *> *)enrichedSimulators:(NSArray<NSDictionary *> *)simulators {
    NSMutableArray<NSDictionary *> *enriched = [NSMutableArray arrayWithCapacity:simulators.count];
    NSDictionary<NSString *, NSDictionary *> *sessionInfoByUDID = [self privateSessionInfoByUDID];

    for (NSDictionary *simulator in simulators) {
        NSMutableDictionary *copy = [simulator mutableCopy];
        NSDictionary *sessionInfo = sessionInfoByUDID[simulator[@"udid"]];
        if (sessionInfo != nil) {
            copy[@"privateDisplay"] = sessionInfo;
        } else {
            copy[@"privateDisplay"] = @{
                @"displayReady": @NO,
                @"displayStatus": [simulator[@"isBooted"] boolValue] ? @"Not attached" : @"Boot required",
                @"displayWidth": @0,
                @"displayHeight": @0,
                @"frameSequence": @0,
            };
        }
        [enriched addObject:copy];
    }

    return enriched;
}

- (void)warmPrivateSessionsForBootedSimulators:(NSArray<NSDictionary *> *)simulators {
    for (NSDictionary *simulator in simulators) {
        if (![simulator[@"isBooted"] boolValue]) {
            continue;
        }

        XCWPrivateSimulatorSession *session = [self ensureSessionForUDID:simulator[@"udid"] waitForReady:NO error:nil];
        if (session != nil) {
            [session waitForFirstEncodedFrameWithTimeout:2.0];
        }
    }
}

- (NSDictionary<NSString *, NSDictionary *> *)privateSessionInfoByUDID {
    @synchronized (self) {
        NSMutableDictionary<NSString *, NSDictionary *> *snapshot = [NSMutableDictionary dictionaryWithCapacity:self.sessionsByUDID.count];
        [self.sessionsByUDID enumerateKeysAndObjectsUsingBlock:^(NSString *key, XCWPrivateSimulatorSession *session, __unused BOOL *stop) {
            snapshot[key] = [session sessionInfoRepresentation];
        }];
        return snapshot;
    }
}

- (nullable XCWPrivateSimulatorSession *)ensureSessionForUDID:(NSString *)udid
                                                 waitForReady:(BOOL)waitForReady
                                                        error:(NSError * _Nullable * _Nullable)error {
    XCWPrivateSimulatorSession *existingSession = nil;
    @synchronized (self) {
        existingSession = self.sessionsByUDID[udid];
    }
    if (existingSession != nil) {
        if (waitForReady) {
            [existingSession waitUntilReadyWithTimeout:10.0];
        }
        return existingSession;
    }

    NSError *lookupError = nil;
    NSDictionary *simulator = [self.simctl simulatorWithUDID:udid error:&lookupError];
    if (simulator == nil) {
        if (error != NULL) {
            *error = lookupError;
        }
        return nil;
    }

    if (![simulator[@"isBooted"] boolValue]) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"XcodeCanvasWeb.PrivateDisplay"
                                         code:1
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"The simulator must be booted before attaching the private display bridge.",
            }];
        }
        return nil;
    }

    NSError *sessionError = nil;
    XCWPrivateSimulatorSession *session = [[XCWPrivateSimulatorSession alloc] initWithUDID:udid
                                                                             simulatorName:simulator[@"name"] ?: udid
                                                                                     error:&sessionError];
    if (session == nil) {
        if (error != NULL) {
            *error = sessionError;
        }
        return nil;
    }

    @synchronized (self) {
        XCWPrivateSimulatorSession *racedSession = self.sessionsByUDID[udid];
        if (racedSession != nil) {
            [session disconnect];
            session = racedSession;
        } else {
            self.sessionsByUDID[udid] = session;
        }
    }

    if (waitForReady) {
        [session waitUntilReadyWithTimeout:10.0];
    }
    return session;
}

- (void)removeSessionForUDID:(NSString *)udid {
    XCWPrivateSimulatorSession *session = nil;
    @synchronized (self) {
        session = self.sessionsByUDID[udid];
        [self.sessionsByUDID removeObjectForKey:udid];
    }
    [session disconnect];
}

- (nullable NSDictionary *)validatedJSONObjectBodyForRequest:(XCWHTTPRequest *)request
                                               errorResponse:(XCWHTTPResponse * _Nullable __autoreleasing * _Nullable)errorResponse {
    NSError *error = nil;
    id payload = [request JSONObjectBody:&error];
    if (payload == nil && request.body.length == 0) {
        return @{};
    }
    if (![payload isKindOfClass:[NSDictionary class]]) {
        if (errorResponse != NULL) {
            *errorResponse = [self errorResponse:error ?: [NSError errorWithDomain:@"XcodeCanvasWeb.API"
                                                                              code:20
                                                                          userInfo:@{
                NSLocalizedDescriptionKey: @"Request body must be a JSON object.",
            }]
                                    statusCode:400];
        }
        return nil;
    }
    return payload;
}

- (XCWHTTPResponse *)staticResponseForRequest:(XCWHTTPRequest *)request {
    NSString *relativePath = request.path;
    if ([relativePath isEqualToString:@"/"]) {
        relativePath = @"/index.html";
    }

    NSString *candidatePath = [[self.clientRoot stringByAppendingPathComponent:[relativePath substringFromIndex:1]] stringByStandardizingPath];
    if (![candidatePath hasPrefix:self.clientRoot]) {
        return [XCWHTTPResponse textResponseWithString:@"Forbidden" statusCode:400];
    }

    BOOL isDirectory = NO;
    if (![[NSFileManager defaultManager] fileExistsAtPath:candidatePath isDirectory:&isDirectory] || isDirectory) {
        candidatePath = [self.clientRoot stringByAppendingPathComponent:@"index.html"];
    }

    NSData *data = [NSData dataWithContentsOfFile:candidatePath];
    if (data.length == 0 && ![[NSFileManager defaultManager] fileExistsAtPath:candidatePath]) {
        NSString *message = @"Client bundle missing. Run `npm install && npm run build` inside client/ first.";
        return [XCWHTTPResponse textResponseWithString:message statusCode:404];
    }

    NSString *extension = candidatePath.pathExtension.lowercaseString;
    NSString *contentType = [self.class contentTypeForExtension:extension];
    return [[XCWHTTPResponse alloc] initWithStatusCode:200
                                               headers:@{
        @"Content-Type": contentType,
        @"Cache-Control": [extension isEqualToString:@"html"] ? @"no-cache" : @"public, max-age=300",
    }
                                                  body:data ?: [NSData data]];
}

- (XCWHTTPResponse *)errorResponse:(NSError *)error statusCode:(NSInteger)statusCode {
    return [XCWHTTPResponse JSONResponseWithObject:@{
        @"error": error.localizedDescription ?: @"Unknown error",
    } statusCode:statusCode];
}

+ (NSString *)contentTypeForExtension:(NSString *)extension {
    static NSDictionary<NSString *, NSString *> *types = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        types = @{
            @"html": @"text/html; charset=utf-8",
            @"js": @"text/javascript; charset=utf-8",
            @"css": @"text/css; charset=utf-8",
            @"json": @"application/json; charset=utf-8",
            @"png": @"image/png",
            @"jpg": @"image/jpeg",
            @"jpeg": @"image/jpeg",
            @"svg": @"image/svg+xml",
            @"ico": @"image/x-icon",
        };
    });
    return types[extension] ?: @"application/octet-stream";
}

@end
