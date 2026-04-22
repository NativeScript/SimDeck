#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@class XCWHTTPRequest;
@class XCWHTTPResponse;
@class XCWHTTPStreamResponse;

typedef XCWHTTPResponse * _Nonnull (^XCWHTTPRequestHandler)(XCWHTTPRequest *request);

@protocol XCWHTTPResponseWriter <NSObject>

- (void)writeData:(NSData *)data completion:(void (^ _Nullable)(NSError * _Nullable error))completion;
- (void)close;

@end

@interface XCWHTTPRequest : NSObject

@property (nonatomic, copy, readonly) NSString *method;
@property (nonatomic, copy, readonly) NSString *target;
@property (nonatomic, copy, readonly) NSString *path;
@property (nonatomic, copy, readonly) NSDictionary<NSString *, NSString *> *headers;
@property (nonatomic, copy, readonly) NSDictionary<NSString *, NSString *> *query;
@property (nonatomic, copy, readonly) NSData *body;

- (instancetype)initWithMethod:(NSString *)method
                        target:(NSString *)target
                       headers:(NSDictionary<NSString *, NSString *> *)headers
                          body:(NSData *)body NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;

- (nullable id)JSONObjectBody:(NSError * _Nullable * _Nullable)error;

@end

@interface XCWHTTPResponse : NSObject

@property (nonatomic, readonly) NSInteger statusCode;
@property (nonatomic, copy, readonly) NSDictionary<NSString *, NSString *> *headers;
@property (nonatomic, copy, readonly) NSData *body;

- (instancetype)initWithStatusCode:(NSInteger)statusCode
                           headers:(NSDictionary<NSString *, NSString *> *)headers
                              body:(NSData *)body NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;

+ (instancetype)JSONResponseWithObject:(id)object statusCode:(NSInteger)statusCode;
+ (instancetype)dataResponseWithData:(NSData *)data contentType:(NSString *)contentType statusCode:(NSInteger)statusCode;
+ (instancetype)textResponseWithString:(NSString *)string statusCode:(NSInteger)statusCode;

@end

@interface XCWHTTPStreamResponse : XCWHTTPResponse

@property (nonatomic, copy, readonly) void (^streamHandler)(id<XCWHTTPResponseWriter> writer);

- (instancetype)initWithStatusCode:(NSInteger)statusCode
                           headers:(NSDictionary<NSString *, NSString *> *)headers
                     streamHandler:(void (^)(id<XCWHTTPResponseWriter> writer))streamHandler;
- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;

+ (instancetype)streamResponseWithStatusCode:(NSInteger)statusCode
                                     headers:(NSDictionary<NSString *, NSString *> *)headers
                               streamHandler:(void (^)(id<XCWHTTPResponseWriter> writer))streamHandler;

@end

@interface XCWHTTPServer : NSObject

- (instancetype)initWithPort:(uint16_t)port
              requestHandler:(XCWHTTPRequestHandler)requestHandler NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;

- (BOOL)start:(NSError * _Nullable * _Nullable)error;

@end

NS_ASSUME_NONNULL_END
