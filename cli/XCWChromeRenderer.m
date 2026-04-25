#import "XCWChromeRenderer.h"

#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>

static NSString * const XCWChromeRendererErrorDomain = @"SimDeck.ChromeRenderer";

@implementation XCWChromeRenderer

+ (nullable NSDictionary<NSString *, id> *)profileForDeviceName:(NSString *)deviceName
                                                          error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *chromeInfo = [self chromeInfoForDeviceName:deviceName error:error];
    if (chromeInfo == nil) {
        return nil;
    }

    NSDictionary *plist = chromeInfo[@"plist"];
    NSDictionary *json = chromeInfo[@"json"];
    NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
    NSDictionary *sizing = [images[@"sizing"] isKindOfClass:[NSDictionary class]] ? images[@"sizing"] : @{};

    CGFloat insetTop = [self numberValue:sizing[@"topHeight"]];
    CGFloat insetLeft = [self numberValue:sizing[@"leftWidth"]];
    CGFloat insetBottom = [self numberValue:sizing[@"bottomHeight"]];
    CGFloat insetRight = [self numberValue:sizing[@"rightWidth"]];

    CGSize compositeSize = [self compositeSizeForChromeInfo:chromeInfo error:error];
    if (CGSizeEqualToSize(compositeSize, CGSizeZero)) {
        return nil;
    }

    NSDictionary *paths = [json[@"paths"] isKindOfClass:[NSDictionary class]] ? json[@"paths"] : @{};
    NSDictionary *border = [paths[@"simpleOutsideBorder"] isKindOfClass:[NSDictionary class]] ? paths[@"simpleOutsideBorder"] : @{};
    CGFloat rawCornerRadius = [self numberValue:border[@"cornerRadiusX"]];

    CGFloat screenScale = MAX([self numberValue:plist[@"mainScreenScale"]], 1.0);
    CGFloat pointScreenWidth = [self numberValue:plist[@"mainScreenWidth"]] / screenScale;
    CGFloat screenWidth = MAX(compositeSize.width - insetLeft - insetRight, 1.0);
    CGFloat screenHeight = MAX(compositeSize.height - insetTop - insetBottom, 1.0);
    CGFloat bezelWidth = MAX(insetLeft, insetTop);
    CGFloat innerRadius = MAX(rawCornerRadius - bezelWidth, 0.0);
    CGFloat radiusScale = pointScreenWidth > 0.0 ? screenWidth / pointScreenWidth : 1.0;
    CGFloat cornerRadius = innerRadius * radiusScale;

    return @{
        @"totalWidth": @(compositeSize.width),
        @"totalHeight": @(compositeSize.height),
        @"screenX": @(insetLeft),
        @"screenY": @(insetTop),
        @"screenWidth": @(screenWidth),
        @"screenHeight": @(screenHeight),
        @"cornerRadius": @(cornerRadius),
    };
}

+ (nullable NSData *)PNGDataForDeviceName:(NSString *)deviceName
                                    error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *chromeInfo = [self chromeInfoForDeviceName:deviceName error:error];
    if (chromeInfo == nil) {
        return nil;
    }

    NSString *compositePath = [self compositeAssetPathForChromeInfo:chromeInfo];
    if (compositePath.length == 0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:6
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"The DeviceKit chrome did not expose a composite PDF asset.",
            }];
        }
        return nil;
    }

    CGSize compositeSize = [self compositeSizeForChromeInfo:chromeInfo error:error];
    if (CGSizeEqualToSize(compositeSize, CGSizeZero)) {
        return nil;
    }

    CGPDFDocumentRef document = CGPDFDocumentCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:compositePath]);
    if (document == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:7
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to open the DeviceKit chrome composite PDF.",
            }];
        }
        return nil;
    }

    CGPDFPageRef page = CGPDFDocumentGetPage(document, 1);
    if (page == NULL) {
        CGPDFDocumentRelease(document);
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:8
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"The DeviceKit chrome composite PDF did not contain a renderable page.",
            }];
        }
        return nil;
    }

    CGFloat scale = 3.0;
    NSInteger pixelWidth = MAX((NSInteger)ceil(compositeSize.width * scale), 1);
    NSInteger pixelHeight = MAX((NSInteger)ceil(compositeSize.height * scale), 1);

    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(NULL,
                                                 pixelWidth,
                                                 pixelHeight,
                                                 8,
                                                 0,
                                                 colorSpace,
                                                 kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(colorSpace);
    if (context == NULL) {
        CGPDFDocumentRelease(document);
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:9
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to create a CoreGraphics bitmap context for simulator chrome rendering.",
            }];
        }
        return nil;
    }
    CGContextClearRect(context, CGRectMake(0, 0, pixelWidth, pixelHeight));
    CGContextSaveGState(context);
    CGContextTranslateCTM(context, 0, pixelHeight);
    CGContextScaleCTM(context, scale, -scale);
    CGContextDrawPDFPage(context, page);
    CGContextRestoreGState(context);

    CGImageRef image = CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    CGPDFDocumentRelease(document);

    if (image == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:10
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to create a CGImage from the simulator chrome bitmap.",
            }];
        }
        return nil;
    }

    NSMutableData *data = [NSMutableData data];
    CGImageDestinationRef destination = CGImageDestinationCreateWithData((__bridge CFMutableDataRef)data,
                                                                         CFSTR("public.png"),
                                                                         1,
                                                                         NULL);
    if (destination == NULL) {
        CGImageRelease(image);
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:11
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to create a PNG encoder for simulator chrome output.",
            }];
        }
        return nil;
    }

    CGImageDestinationAddImage(destination, image, NULL);
    BOOL finalized = CGImageDestinationFinalize(destination);
    CFRelease(destination);
    CGImageRelease(image);

    if (!finalized) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:12
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to encode simulator chrome PNG.",
            }];
        }
        return nil;
    }
    return data;
}

+ (nullable NSDictionary *)chromeInfoForDeviceName:(NSString *)deviceName
                                             error:(NSError * _Nullable __autoreleasing *)error {
    NSString *profilePath = [NSString stringWithFormat:@"/Library/Developer/CoreSimulator/Profiles/DeviceTypes/%@.simdevicetype/Contents/Resources/profile.plist", deviceName];
    NSData *profileData = [NSData dataWithContentsOfFile:profilePath];
    if (profileData == nil) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:1
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Unable to open %@.", profilePath.lastPathComponent],
            }];
        }
        return nil;
    }

    NSError *plistError = nil;
    NSDictionary *plist = [NSPropertyListSerialization propertyListWithData:profileData
                                                                    options:NSPropertyListImmutable
                                                                     format:nil
                                                                      error:&plistError];
    if (![plist isKindOfClass:[NSDictionary class]]) {
        if (error != NULL) {
            *error = plistError ?: [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                                       code:2
                                                   userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to decode the CoreSimulator device profile.",
            }];
        }
        return nil;
    }

    NSString *chromeIdentifier = [plist[@"chromeIdentifier"] isKindOfClass:[NSString class]] ? plist[@"chromeIdentifier"] : @"";
    NSString *chromeName = [chromeIdentifier stringByReplacingOccurrencesOfString:@"com.apple.dt.devicekit.chrome."
                                                                       withString:@""];
    if (chromeName.length == 0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:3
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"The device profile for %@ did not specify a DeviceKit chrome identifier.", deviceName],
            }];
        }
        return nil;
    }

    NSString *chromePath = [NSString stringWithFormat:@"/Library/Developer/DeviceKit/Chrome/%@.devicechrome/Contents/Resources", chromeName];
    NSString *jsonPath = [chromePath stringByAppendingPathComponent:@"chrome.json"];
    NSData *jsonData = [NSData dataWithContentsOfFile:jsonPath];
    if (jsonData == nil) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:4
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Unable to locate DeviceKit chrome metadata for %@.", deviceName],
            }];
        }
        return nil;
    }

    NSError *jsonError = nil;
    NSDictionary *json = [NSJSONSerialization JSONObjectWithData:jsonData options:0 error:&jsonError];
    if (![json isKindOfClass:[NSDictionary class]]) {
        if (error != NULL) {
            *error = jsonError ?: [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                                      code:5
                                                  userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to decode DeviceKit chrome metadata.",
            }];
        }
        return nil;
    }

    return @{
        @"plist": plist,
        @"json": json,
        @"chromePath": chromePath,
    };
}

+ (CGSize)compositeSizeForChromeInfo:(NSDictionary *)chromeInfo
                               error:(NSError * _Nullable __autoreleasing *)error {
    NSString *compositePath = [self compositeAssetPathForChromeInfo:chromeInfo];
    if (compositePath.length == 0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:11
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"The DeviceKit chrome metadata did not specify a composite PDF asset.",
            }];
        }
        return CGSizeZero;
    }

    CGPDFDocumentRef document = CGPDFDocumentCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:compositePath]);
    if (document == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:12
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to open the DeviceKit chrome composite PDF.",
            }];
        }
        return CGSizeZero;
    }

    CGPDFPageRef page = CGPDFDocumentGetPage(document, 1);
    CGRect pageRect = page != NULL ? CGPDFPageGetBoxRect(page, kCGPDFMediaBox) : CGRectZero;
    CGPDFDocumentRelease(document);
    return pageRect.size;
}

+ (NSString *)compositeAssetPathForChromeInfo:(NSDictionary *)chromeInfo {
    NSDictionary *json = chromeInfo[@"json"];
    NSString *chromePath = chromeInfo[@"chromePath"];
    NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
    NSString *name = [images[@"composite"] isKindOfClass:[NSString class]] ? images[@"composite"] : @"";
    if (name.length == 0) {
        return @"";
    }
    return [self resolvedChromeAssetPathForName:name chromePath:chromePath];
}

+ (CGFloat)numberValue:(id)value {
    if ([value respondsToSelector:@selector(doubleValue)]) {
        return (CGFloat)[value doubleValue];
    }
    return 0.0;
}

+ (NSString *)resolvedChromeAssetPathForName:(NSString *)name chromePath:(NSString *)chromePath {
    NSString *candidate = [chromePath stringByAppendingPathComponent:name];
    if ([[NSFileManager defaultManager] fileExistsAtPath:candidate]) {
        return candidate;
    }
    if (name.pathExtension.length == 0) {
        NSString *pdfPath = [candidate stringByAppendingPathExtension:@"pdf"];
        if ([[NSFileManager defaultManager] fileExistsAtPath:pdfPath]) {
            return pdfPath;
        }
    }
    return candidate;
}

@end
