#import "XCWHTTPServer.h"

#import <Network/Network.h>

@class XCWHTTPConnectionContext;

@interface XCWHTTPServer ()

@property (nonatomic, readonly) uint16_t port;
@property (nonatomic, copy, readonly) XCWHTTPRequestHandler requestHandler;
@property (nonatomic, strong, readonly) NSMutableSet *activeConnections;

- (void)registerConnectionContext:(XCWHTTPConnectionContext *)context;
- (void)unregisterConnectionContext:(XCWHTTPConnectionContext *)context;

@end

@interface XCWHTTPConnectionContext : NSObject

- (instancetype)initWithConnection:(nw_connection_t)connection server:(XCWHTTPServer *)server;
- (void)start;

@end

@interface XCWHTTPConnectionWriter : NSObject <XCWHTTPResponseWriter>

- (instancetype)initWithConnectionContext:(XCWHTTPConnectionContext *)connectionContext
                    usesChunkedTransfer:(BOOL)usesChunkedTransfer;

@end

static dispatch_data_t XCWDispatchDataFromNSData(NSData *data) {
    NSData *retainedData = [data copy];
    return dispatch_data_create(retainedData.bytes, retainedData.length, dispatch_get_main_queue(), ^{
        (void)retainedData;
    });
}

@implementation XCWHTTPRequest

- (instancetype)initWithMethod:(NSString *)method
                        target:(NSString *)target
                       headers:(NSDictionary<NSString *,NSString *> *)headers
                          body:(NSData *)body {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _method = [method copy];
    _target = [target copy];
    _headers = [headers copy];
    _body = [body copy];

    NSURLComponents *components = [NSURLComponents componentsWithString:[NSString stringWithFormat:@"http://localhost%@", target]];
    _path = components.path ?: @"/";

    NSMutableDictionary<NSString *, NSString *> *query = [NSMutableDictionary dictionary];
    for (NSURLQueryItem *item in components.queryItems ?: @[]) {
        if (item.name.length > 0) {
            query[item.name] = item.value ?: @"";
        }
    }
    _query = query;
    return self;
}

- (nullable id)JSONObjectBody:(NSError * _Nullable __autoreleasing *)error {
    if (self.body.length == 0) {
        return nil;
    }
    return [NSJSONSerialization JSONObjectWithData:self.body options:0 error:error];
}

@end

@implementation XCWHTTPResponse

- (instancetype)initWithStatusCode:(NSInteger)statusCode
                           headers:(NSDictionary<NSString *,NSString *> *)headers
                              body:(NSData *)body {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _statusCode = statusCode;
    _headers = [headers copy];
    _body = [body copy];
    return self;
}

+ (instancetype)JSONResponseWithObject:(id)object statusCode:(NSInteger)statusCode {
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:0 error:nil] ?: [NSData data];
    return [[self alloc] initWithStatusCode:statusCode
                                    headers:@{ @"Content-Type": @"application/json; charset=utf-8" }
                                       body:data];
}

+ (instancetype)dataResponseWithData:(NSData *)data contentType:(NSString *)contentType statusCode:(NSInteger)statusCode {
    return [[self alloc] initWithStatusCode:statusCode
                                    headers:@{ @"Content-Type": contentType }
                                       body:data];
}

+ (instancetype)textResponseWithString:(NSString *)string statusCode:(NSInteger)statusCode {
    NSData *data = [string dataUsingEncoding:NSUTF8StringEncoding] ?: [NSData data];
    return [[self alloc] initWithStatusCode:statusCode
                                    headers:@{ @"Content-Type": @"text/plain; charset=utf-8" }
                                       body:data];
}

@end

@implementation XCWHTTPStreamResponse

- (instancetype)initWithStatusCode:(NSInteger)statusCode
                           headers:(NSDictionary<NSString *,NSString *> *)headers
                     streamHandler:(void (^)(id<XCWHTTPResponseWriter> writer))streamHandler {
    self = [super initWithStatusCode:statusCode headers:headers body:[NSData data]];
    if (self == nil) {
        return nil;
    }

    _streamHandler = [streamHandler copy];
    return self;
}

+ (instancetype)streamResponseWithStatusCode:(NSInteger)statusCode
                                     headers:(NSDictionary<NSString *,NSString *> *)headers
                               streamHandler:(void (^)(id<XCWHTTPResponseWriter> writer))streamHandler {
    return [[self alloc] initWithStatusCode:statusCode headers:headers streamHandler:streamHandler];
}

@end

@implementation XCWHTTPServer {
    nw_listener_t _listener;
}

- (instancetype)initWithPort:(uint16_t)port requestHandler:(XCWHTTPRequestHandler)requestHandler {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _port = port;
    _requestHandler = [requestHandler copy];
    _activeConnections = [NSMutableSet set];
    return self;
}

- (BOOL)start:(NSError * _Nullable __autoreleasing *)error {
    nw_parameters_t parameters = nw_parameters_create_secure_tcp(NW_PARAMETERS_DISABLE_PROTOCOL, NW_PARAMETERS_DEFAULT_CONFIGURATION);
    nw_protocol_stack_t stack = nw_parameters_copy_default_protocol_stack(parameters);
    nw_protocol_options_t tcpOptions = nw_protocol_stack_copy_transport_protocol(stack);
    nw_tcp_options_set_no_delay(tcpOptions, true);
    char portBuffer[16] = {0};
    snprintf(portBuffer, sizeof(portBuffer), "%u", self.port);
    _listener = nw_listener_create_with_port(portBuffer, parameters);
    if (_listener == nil) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:@"XcodeCanvasWeb.HTTPServer"
                                         code:1
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Unable to bind to port %u.", self.port],
            }];
        }
        return NO;
    }

    __weak typeof(self) weakSelf = self;
    nw_listener_set_queue(_listener, dispatch_get_main_queue());
    nw_listener_set_state_changed_handler(_listener, ^(nw_listener_state_t state, nw_error_t  _Nullable nwError) {
        if (state == nw_listener_state_failed && nwError != nil) {
            NSError *errorObject = CFBridgingRelease(nw_error_copy_cf_error(nwError));
            NSLog(@"[XCW] HTTP listener failed: %@", errorObject.localizedDescription ?: @"unknown error");
        }
    });
    nw_listener_set_new_connection_handler(_listener, ^(nw_connection_t  _Nonnull connection) {
        __strong typeof(weakSelf) strongSelf = weakSelf;
        if (strongSelf == nil) {
            nw_connection_cancel(connection);
            return;
        }
        XCWHTTPConnectionContext *context = [[XCWHTTPConnectionContext alloc] initWithConnection:connection server:strongSelf];
        [strongSelf registerConnectionContext:context];
        [context start];
    });
    nw_listener_start(_listener);
    return YES;
}

- (void)registerConnectionContext:(XCWHTTPConnectionContext *)context {
    [self.activeConnections addObject:context];
}

- (void)unregisterConnectionContext:(XCWHTTPConnectionContext *)context {
    [self.activeConnections removeObject:context];
}

@end

@interface XCWHTTPConnectionContext ()

@property (nonatomic, weak, readonly) XCWHTTPServer *server;
@property (nonatomic, readonly) nw_connection_t connection;
@property (nonatomic, strong, readonly) NSMutableData *buffer;
@property (nonatomic, strong, readonly) dispatch_queue_t queue;
@property (nonatomic, strong) XCWHTTPConnectionWriter *writer;

@end

@implementation XCWHTTPConnectionContext {
    BOOL _responseSent;
    BOOL _connectionClosed;
}

- (instancetype)initWithConnection:(nw_connection_t)connection server:(XCWHTTPServer *)server {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _connection = connection;
    _server = server;
    _buffer = [NSMutableData data];
    _queue = dispatch_queue_create("com.xcodecanvasweb.http.connection", DISPATCH_QUEUE_SERIAL);
    return self;
}

- (void)start {
    nw_connection_set_queue(self.connection, self.queue);
    nw_connection_start(self.connection);
    [self receiveMoreData];
}

- (void)receiveMoreData {
    if (_responseSent) {
        return;
    }

    __weak typeof(self) weakSelf = self;
    nw_connection_receive(self.connection, 1, 65536, ^(dispatch_data_t  _Nullable content, __unused nw_content_context_t  _Nullable context, bool isComplete, nw_error_t  _Nullable receiveError) {
        __strong typeof(weakSelf) strongSelf = weakSelf;
        if (strongSelf == nil) {
            return;
        }

        if (receiveError != nil) {
            [strongSelf cancelConnection];
            return;
        }

        if (content != nil) {
            NSMutableData *chunk = [NSMutableData data];
            dispatch_data_apply(content, ^bool(__unused dispatch_data_t region, __unused size_t offset, const void *buffer, size_t size) {
                [chunk appendBytes:buffer length:size];
                return true;
            });
            [strongSelf.buffer appendData:chunk];
        }

        XCWHTTPRequest *request = [strongSelf parsedRequestIfComplete];
        if (request != nil) {
            XCWHTTPResponse *response = strongSelf.server.requestHandler(request);
            [strongSelf sendResponse:response];
            return;
        }

        if (isComplete) {
            XCWHTTPResponse *response = [XCWHTTPResponse textResponseWithString:@"Bad Request" statusCode:400];
            [strongSelf sendResponse:response];
            return;
        }

        [strongSelf receiveMoreData];
    });
}

- (nullable XCWHTTPRequest *)parsedRequestIfComplete {
    NSData *delimiter = [@"\r\n\r\n" dataUsingEncoding:NSUTF8StringEncoding];
    NSRange headerRange = [self.buffer rangeOfData:delimiter options:0 range:NSMakeRange(0, self.buffer.length)];
    if (headerRange.location == NSNotFound) {
        return nil;
    }

    NSUInteger headerLength = NSMaxRange(headerRange);
    NSData *headerData = [self.buffer subdataWithRange:NSMakeRange(0, headerRange.location)];
    NSString *headerString = [[NSString alloc] initWithData:headerData encoding:NSUTF8StringEncoding];
    if (headerString.length == 0) {
        return nil;
    }

    NSArray<NSString *> *lines = [headerString componentsSeparatedByString:@"\r\n"];
    if (lines.count == 0) {
        return nil;
    }

    NSArray<NSString *> *requestLineParts = [lines.firstObject componentsSeparatedByString:@" "];
    if (requestLineParts.count < 2) {
        return nil;
    }

    NSMutableDictionary<NSString *, NSString *> *headers = [NSMutableDictionary dictionary];
    for (NSUInteger index = 1; index < lines.count; index++) {
        NSString *line = lines[index];
        NSRange separator = [line rangeOfString:@":"];
        if (separator.location == NSNotFound) {
            continue;
        }
        NSString *name = [[line substringToIndex:separator.location] lowercaseString];
        NSString *value = [[line substringFromIndex:separator.location + 1] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
        headers[name] = value;
    }

    NSUInteger contentLength = (NSUInteger)[headers[@"content-length"] integerValue];
    if (self.buffer.length < headerLength + contentLength) {
        return nil;
    }

    NSData *body = contentLength > 0 ? [self.buffer subdataWithRange:NSMakeRange(headerLength, contentLength)] : [NSData data];
    return [[XCWHTTPRequest alloc] initWithMethod:[requestLineParts[0] uppercaseString]
                                           target:requestLineParts[1]
                                          headers:headers
                                             body:body];
}

- (void)sendResponse:(XCWHTTPResponse *)response {
    _responseSent = YES;

    NSMutableDictionary<NSString *, NSString *> *headers = [response.headers mutableCopy];
    BOOL isStreamResponse = [response isKindOfClass:[XCWHTTPStreamResponse class]];
    if (!isStreamResponse) {
        headers[@"Content-Length"] = [NSString stringWithFormat:@"%lu", (unsigned long)response.body.length];
    } else {
        headers[@"Transfer-Encoding"] = @"chunked";
    }
    headers[@"Connection"] = isStreamResponse ? @"keep-alive" : @"close";
    headers[@"Access-Control-Allow-Origin"] = @"*";
    headers[@"Cache-Control"] = headers[@"Cache-Control"] ?: @"no-store";

    NSMutableString *headerString = [NSMutableString stringWithFormat:@"HTTP/1.1 %ld %@\r\n", (long)response.statusCode, [self.class reasonPhraseForStatusCode:response.statusCode]];
    [headers enumerateKeysAndObjectsUsingBlock:^(NSString *key, NSString *value, __unused BOOL *stop) {
        [headerString appendFormat:@"%@: %@\r\n", key, value];
    }];
    [headerString appendString:@"\r\n"];

    NSMutableData *payload = [NSMutableData data];
    [payload appendData:[headerString dataUsingEncoding:NSUTF8StringEncoding]];
    if (!isStreamResponse) {
        [payload appendData:response.body];
    }

    dispatch_data_t dispatchData = XCWDispatchDataFromNSData(payload);
    __weak typeof(self) weakSelf = self;
    nw_connection_send(self.connection, dispatchData, NW_CONNECTION_DEFAULT_MESSAGE_CONTEXT, !isStreamResponse, ^(nw_error_t  _Nullable sendError) {
        __strong typeof(weakSelf) strongSelf = weakSelf;
        if (sendError != nil) {
            [strongSelf cancelConnection];
            return;
        }

        if (isStreamResponse) {
            strongSelf.writer = [[XCWHTTPConnectionWriter alloc] initWithConnectionContext:strongSelf
                                                                        usesChunkedTransfer:YES];
            XCWHTTPStreamResponse *streamResponse = (XCWHTTPStreamResponse *)response;
            streamResponse.streamHandler(strongSelf.writer);
            return;
        }

        [strongSelf cancelConnection];
    });
}

- (void)cancelConnection {
    if (_connectionClosed) {
        return;
    }
    _connectionClosed = YES;
    [self.server unregisterConnectionContext:self];
    nw_connection_cancel(self.connection);
}

+ (NSString *)reasonPhraseForStatusCode:(NSInteger)statusCode {
    switch (statusCode) {
        case 200: return @"OK";
        case 201: return @"Created";
        case 204: return @"No Content";
        case 400: return @"Bad Request";
        case 404: return @"Not Found";
        case 405: return @"Method Not Allowed";
        case 500: return @"Internal Server Error";
        case 503: return @"Service Unavailable";
        default: return @"OK";
    }
}

@end

@interface XCWHTTPConnectionWriter ()

@property (nonatomic, weak, readonly) XCWHTTPConnectionContext *connectionContext;
@property (nonatomic, strong, readonly) dispatch_queue_t queue;

@end

@implementation XCWHTTPConnectionWriter {
    BOOL _closed;
    BOOL _writeInFlight;
    BOOL _closeAfterDraining;
    BOOL _usesChunkedTransfer;
    NSMutableArray<NSDictionary *> *_pendingWrites;
}

- (instancetype)initWithConnectionContext:(XCWHTTPConnectionContext *)connectionContext
                    usesChunkedTransfer:(BOOL)usesChunkedTransfer {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _connectionContext = connectionContext;
    _queue = dispatch_queue_create("com.xcodecanvasweb.http.writer", DISPATCH_QUEUE_SERIAL);
    _pendingWrites = [NSMutableArray array];
    _usesChunkedTransfer = usesChunkedTransfer;
    return self;
}

- (void)writeData:(NSData *)data completion:(void (^ _Nullable)(NSError * _Nullable error))completion {
    if (data.length == 0) {
        if (completion != nil) {
            completion(nil);
        }
        return;
    }

    dispatch_async(self.queue, ^{
        if (self->_closed) {
            if (completion != nil) {
                completion([NSError errorWithDomain:@"XcodeCanvasWeb.HTTPWriter"
                                               code:1
                                           userInfo:@{
                    NSLocalizedDescriptionKey: @"Connection already closed.",
                }]);
            }
            return;
        }

        NSData *payload = data;
        if (self->_usesChunkedTransfer) {
            NSMutableData *chunk = [NSMutableData data];
            NSString *lengthLine = [NSString stringWithFormat:@"%lx\r\n", (unsigned long)data.length];
            [chunk appendData:[lengthLine dataUsingEncoding:NSUTF8StringEncoding]];
            [chunk appendData:data];
            [chunk appendData:[@"\r\n" dataUsingEncoding:NSUTF8StringEncoding]];
            payload = chunk;
        }

        [self->_pendingWrites addObject:@{
            @"data": payload,
            @"completion": completion ?: ^(__unused NSError * _Nullable error) {}
        }];
        [self drainQueueIfNeeded];
    });
}

- (void)close {
    dispatch_async(self.queue, ^{
        if (self->_closed) {
            return;
        }
        self->_closed = YES;
        if (self->_usesChunkedTransfer) {
            self->_closeAfterDraining = YES;
            [self->_pendingWrites addObject:@{
                @"data": [@"0\r\n\r\n" dataUsingEncoding:NSUTF8StringEncoding],
                @"completion": ^(__unused NSError * _Nullable error) {}
            }];
            [self drainQueueIfNeeded];
            return;
        }

        dispatch_async(dispatch_get_main_queue(), ^{
            [self.connectionContext cancelConnection];
        });
    });
}

- (void)drainQueueIfNeeded {
    if (_writeInFlight || _closed || _pendingWrites.count == 0) {
        return;
    }

    _writeInFlight = YES;
    NSDictionary *entry = _pendingWrites.firstObject;
    [_pendingWrites removeObjectAtIndex:0];

    NSData *data = entry[@"data"];
    void (^completion)(NSError * _Nullable error) = entry[@"completion"];
    dispatch_data_t dispatchData = XCWDispatchDataFromNSData(data);

    __weak typeof(self) weakSelf = self;
    nw_connection_send(self.connectionContext.connection, dispatchData, NW_CONNECTION_DEFAULT_MESSAGE_CONTEXT, false, ^(nw_error_t  _Nullable sendError) {
        __strong typeof(weakSelf) strongSelf = weakSelf;
        NSError *error = sendError != nil ? CFBridgingRelease(nw_error_copy_cf_error(sendError)) : nil;
        if (completion != nil) {
            completion(error);
        }

        dispatch_async(strongSelf.queue, ^{
            strongSelf->_writeInFlight = NO;
            if (error != nil) {
                strongSelf->_closed = YES;
                [strongSelf->_pendingWrites removeAllObjects];
                dispatch_async(dispatch_get_main_queue(), ^{
                    [strongSelf.connectionContext cancelConnection];
                });
                return;
            }
            if (strongSelf->_closeAfterDraining && strongSelf->_pendingWrites.count == 0) {
                dispatch_async(dispatch_get_main_queue(), ^{
                    [strongSelf.connectionContext cancelConnection];
                });
                return;
            }
            [strongSelf drainQueueIfNeeded];
        });
    });
}

@end
