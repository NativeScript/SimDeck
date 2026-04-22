#import <CoreGraphics/CoreGraphics.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

typedef void (^XCWH264EncoderOutputHandler)(NSData *sampleData,
                                            uint64_t timestampUs,
                                            BOOL isKeyFrame,
                                            NSString * _Nullable codec,
                                            NSData * _Nullable decoderConfig,
                                            CGSize dimensions);

@interface XCWH264Encoder : NSObject

- (instancetype)init NS_UNAVAILABLE;
+ (instancetype)new NS_UNAVAILABLE;
- (instancetype)initWithOutputHandler:(XCWH264EncoderOutputHandler)outputHandler NS_DESIGNATED_INITIALIZER;

- (void)encodePixelBuffer:(CVPixelBufferRef)pixelBuffer;
- (void)requestKeyFrame;
- (void)invalidate;

@end

NS_ASSUME_NONNULL_END
