#import "XCWH264Encoder.h"

#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#import <QuartzCore/QuartzCore.h>
#include <math.h>
#include <stdlib.h>
#include <unistd.h>

static uint64_t NowUs(void) {
    return (uint64_t)llround(CACurrentMediaTime() * 1000000.0);
}

static void FillPixelBuffer(CVPixelBufferRef pixelBuffer, int frameIndex) {
    CVPixelBufferLockBaseAddress(pixelBuffer, 0);
    uint8_t *base = (uint8_t *)CVPixelBufferGetBaseAddress(pixelBuffer);
    size_t width = CVPixelBufferGetWidth(pixelBuffer);
    size_t height = CVPixelBufferGetHeight(pixelBuffer);
    size_t stride = CVPixelBufferGetBytesPerRow(pixelBuffer);
    for (size_t y = 0; y < height; y++) {
        uint8_t *row = base + (y * stride);
        for (size_t x = 0; x < width; x++) {
            uint8_t v = (uint8_t)((x + y + (size_t)(frameIndex * 7)) & 0xff);
            row[x * 4 + 0] = (uint8_t)(v ^ 0x55);
            row[x * 4 + 1] = (uint8_t)((v + frameIndex) & 0xff);
            row[x * 4 + 2] = v;
            row[x * 4 + 3] = 0xff;
        }
    }
    CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);
}

static CVPixelBufferRef CreatePixelBuffer(size_t width, size_t height, int frameIndex) {
    CVPixelBufferRef pixelBuffer = NULL;
    NSDictionary *attrs = @{
        (__bridge NSString *)kCVPixelBufferCGImageCompatibilityKey: @YES,
        (__bridge NSString *)kCVPixelBufferCGBitmapContextCompatibilityKey: @YES,
    };
    CVReturn status = CVPixelBufferCreate(kCFAllocatorDefault,
                                          width,
                                          height,
                                          kCVPixelFormatType_32BGRA,
                                          (__bridge CFDictionaryRef)attrs,
                                          &pixelBuffer);
    if (status != kCVReturnSuccess || pixelBuffer == NULL) {
        return NULL;
    }
    FillPixelBuffer(pixelBuffer, frameIndex);
    return pixelBuffer;
}

static double Percentile(NSArray<NSNumber *> *values, double percentile) {
    if (values.count == 0) {
        return 0.0;
    }
    NSArray<NSNumber *> *sorted = [values sortedArrayUsingSelector:@selector(compare:)];
    double index = ((double)sorted.count - 1.0) * percentile;
    NSUInteger lower = (NSUInteger)floor(index);
    NSUInteger upper = (NSUInteger)ceil(index);
    if (lower == upper) {
        return sorted[lower].doubleValue;
    }
    double fraction = index - (double)lower;
    return sorted[lower].doubleValue * (1.0 - fraction) + sorted[upper].doubleValue * fraction;
}

static void PrintUsage(const char *program) {
    fprintf(stderr, "usage: %s <software|hardware|auto> <width> <height> <fps> <seconds>\n", program);
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 6) {
            PrintUsage(argv[0]);
            return 2;
        }

        NSString *mode = [NSString stringWithUTF8String:argv[1]];
        int width = atoi(argv[2]);
        int height = atoi(argv[3]);
        int fps = atoi(argv[4]);
        double seconds = atof(argv[5]);
        if (width <= 0 || height <= 0 || fps <= 0 || seconds <= 0) {
            PrintUsage(argv[0]);
            return 2;
        }

        setenv("SIMDECK_VIDEO_CODEC", mode.UTF8String, 1);
        setenv("SIMDECK_REALTIME_STREAM", "0", 1);
        setenv("SIMDECK_LOW_LATENCY", "0", 1);

        NSMutableArray<NSNumber *> *outputGapsMs = [NSMutableArray array];
        __block NSUInteger outputFrames = 0;
        __block NSUInteger keyframes = 0;
        __block NSUInteger bytes = 0;
        __block uint64_t firstOutputUs = 0;
        __block uint64_t lastOutputUs = 0;

        XCWH264Encoder *encoder = [[XCWH264Encoder alloc] initWithOutputHandler:^(NSData *sampleData,
                                                                                  uint64_t timestampUs,
                                                                                  BOOL isKeyFrame,
                                                                                  NSString *codec,
                                                                                  NSData *decoderConfig,
                                                                                  CGSize dimensions) {
            (void)timestampUs;
            (void)codec;
            (void)decoderConfig;
            (void)dimensions;
            @synchronized (outputGapsMs) {
                uint64_t now = NowUs();
                if (firstOutputUs == 0) {
                    firstOutputUs = now;
                }
                if (lastOutputUs > 0) {
                    [outputGapsMs addObject:@(((double)(now - lastOutputUs)) / 1000.0)];
                }
                lastOutputUs = now;
                outputFrames += 1;
                keyframes += isKeyFrame ? 1 : 0;
                bytes += sampleData.length;
            }
        }];

        NSMutableArray *buffers = [NSMutableArray array];
        for (int i = 0; i < 4; i++) {
            CVPixelBufferRef pixelBuffer = CreatePixelBuffer((size_t)width, (size_t)height, i);
            if (pixelBuffer == NULL) {
                fprintf(stderr, "failed to create pixel buffer\n");
                return 1;
            }
            [buffers addObject:(__bridge id)pixelBuffer];
            CVPixelBufferRelease(pixelBuffer);
        }

        NSUInteger inputFrames = (NSUInteger)llround(seconds * (double)fps);
        uint64_t intervalUs = (uint64_t)llround(1000000.0 / (double)fps);
        uint64_t startUs = NowUs();
        for (NSUInteger i = 0; i < inputFrames; i++) {
            uint64_t targetUs = startUs + (uint64_t)i * intervalUs;
            while (NowUs() < targetUs) {
                usleep(100);
            }
            CVPixelBufferRef pixelBuffer = (__bridge CVPixelBufferRef)buffers[i % buffers.count];
            [encoder encodePixelBuffer:pixelBuffer];
        }

        uint64_t drainDeadlineUs = NowUs() + 3000000;
        while (NowUs() < drainDeadlineUs) {
            NSDictionary *stats = [encoder statsRepresentation];
            NSNumber *inFlight = stats[@"inFlightFrames"];
            NSNumber *submitted = stats[@"submittedFrames"];
            NSNumber *outputs = stats[@"outputFrames"];
            if (inFlight.unsignedIntegerValue == 0 &&
                outputs.unsignedIntegerValue >= submitted.unsignedIntegerValue) {
                break;
            }
            usleep(1000);
        }

        NSDictionary *stats = [encoder statsRepresentation];
        [encoder invalidate];

        NSArray<NSNumber *> *gaps = nil;
        NSUInteger localOutputs = 0;
        NSUInteger localKeyframes = 0;
        NSUInteger localBytes = 0;
        uint64_t localFirstOutputUs = 0;
        uint64_t localLastOutputUs = 0;
        @synchronized (outputGapsMs) {
            gaps = [outputGapsMs copy];
            localOutputs = outputFrames;
            localKeyframes = keyframes;
            localBytes = bytes;
            localFirstOutputUs = firstOutputUs;
            localLastOutputUs = lastOutputUs;
        }

        double outputDuration = localLastOutputUs > localFirstOutputUs
            ? ((double)(localLastOutputUs - localFirstOutputUs)) / 1000000.0
            : 0.0;
        double outputFps = outputDuration > 0 ? ((double)MAX(localOutputs, 1) / outputDuration) : 0.0;
        double mbps = outputDuration > 0 ? (((double)localBytes * 8.0) / outputDuration / 1000000.0) : 0.0;

        NSDictionary *result = @{
            @"mode": mode,
            @"sourceWidth": @(width),
            @"sourceHeight": @(height),
            @"targetFps": @(fps),
            @"seconds": @(seconds),
            @"inputFrames": @(inputFrames),
            @"outputFrames": @(localOutputs),
            @"keyframes": @(localKeyframes),
            @"outputFps": @(outputFps),
            @"mbps": @(mbps),
            @"gapAvgMs": @([gaps valueForKeyPath:@"@avg.self"] ? [[gaps valueForKeyPath:@"@avg.self"] doubleValue] : 0.0),
            @"gapP50Ms": @(Percentile(gaps, 0.50)),
            @"gapP95Ms": @(Percentile(gaps, 0.95)),
            @"gapP99Ms": @(Percentile(gaps, 0.99)),
            @"gapMaxMs": @([gaps valueForKeyPath:@"@max.self"] ? [[gaps valueForKeyPath:@"@max.self"] doubleValue] : 0.0),
            @"encoderStats": stats,
        };
        NSData *json = [NSJSONSerialization dataWithJSONObject:result options:NSJSONWritingPrettyPrinted error:nil];
        NSString *jsonString = json ? [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding] : @"{}";
        printf("%s\n", jsonString.UTF8String);
    }
    return 0;
}
