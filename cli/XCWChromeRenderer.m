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

    BOOL watchProfile = [self isWatchProfile:plist];
    CGFloat screenScale = MAX([self numberValue:plist[@"mainScreenScale"]], 1.0);
    CGFloat profileScreenWidth = [self numberValue:plist[@"mainScreenWidth"]];
    CGFloat profileScreenHeight = [self numberValue:plist[@"mainScreenHeight"]];
    CGFloat pointScreenWidth = watchProfile ? profileScreenWidth : profileScreenWidth / screenScale;
    CGFloat screenWidth = watchProfile ? profileScreenWidth : MAX(compositeSize.width - insetLeft - insetRight, 1.0);
    CGFloat screenHeight = watchProfile ? profileScreenHeight : MAX(compositeSize.height - insetTop - insetBottom, 1.0);
    CGFloat screenX = watchProfile ? MAX((compositeSize.width - screenWidth) / 2.0, 0.0) : insetLeft;
    CGFloat screenY = watchProfile ? MAX((compositeSize.height - screenHeight) / 2.0, 0.0) : insetTop;
    CGFloat bezelWidth = MAX(insetLeft, insetTop);
    CGFloat innerRadius = MAX(rawCornerRadius - bezelWidth, 0.0);
    CGFloat radiusScale = pointScreenWidth > 0.0 ? screenWidth / pointScreenWidth : 1.0;
    CGFloat cornerRadius = watchProfile ? rawCornerRadius : innerRadius * radiusScale;

    return @{
        @"totalWidth": @(compositeSize.width),
        @"totalHeight": @(compositeSize.height),
        @"screenX": @(screenX),
        @"screenY": @(screenY),
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
    CGSize compositeSize = [self compositeSizeForChromeInfo:chromeInfo error:error];
    if (CGSizeEqualToSize(compositeSize, CGSizeZero)) {
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
    if (![self drawInputImagesForChromeInfo:chromeInfo
                                      inSize:compositeSize
                                     context:context
                                   onlyOnTop:NO
                                      error:error]) {
        CGContextRestoreGState(context);
        CGContextRelease(context);
        return nil;
    }
    BOOL rendered = NO;
    if (compositePath.length > 0) {
        rendered = [self drawPDFAtPath:compositePath
                               inRect:CGRectMake(0, 0, compositeSize.width, compositeSize.height)
                              context:context
                                 error:error];
    } else {
        rendered = [self drawSlicedChromeInfo:chromeInfo
                                      inSize:compositeSize
                                     context:context
                                       error:error];
    }
    if (!rendered) {
        CGContextRestoreGState(context);
        CGContextRelease(context);
        return nil;
    }
    if (![self drawInputImagesForChromeInfo:chromeInfo
                                      inSize:compositeSize
                                     context:context
                                   onlyOnTop:YES
                                      error:error]) {
        CGContextRestoreGState(context);
        CGContextRelease(context);
        return nil;
    }
    CGContextRestoreGState(context);

    CGImageRef image = CGBitmapContextCreateImage(context);
    CGContextRelease(context);

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
        NSDictionary *plist = chromeInfo[@"plist"];
        NSDictionary *json = chromeInfo[@"json"];
        NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
        NSDictionary *sizing = [images[@"sizing"] isKindOfClass:[NSDictionary class]] ? images[@"sizing"] : @{};
        CGFloat screenScale = MAX([self numberValue:plist[@"mainScreenScale"]], 1.0);
        BOOL watchProfile = [self isWatchProfile:plist];
        CGFloat screenWidth = [self numberValue:plist[@"mainScreenWidth"]];
        CGFloat screenHeight = [self numberValue:plist[@"mainScreenHeight"]];
        if (!watchProfile) {
            screenWidth /= screenScale;
            screenHeight /= screenScale;
        }
        CGFloat totalWidth = screenWidth + [self numberValue:sizing[@"leftWidth"]] + [self numberValue:sizing[@"rightWidth"]];
        CGFloat totalHeight = screenHeight + [self numberValue:sizing[@"topHeight"]] + [self numberValue:sizing[@"bottomHeight"]];
        if (totalWidth > 0.0 && totalHeight > 0.0) {
            return CGSizeMake(totalWidth, totalHeight);
        }
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:11
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"The DeviceKit chrome metadata did not specify enough sizing data.",
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

+ (BOOL)drawSlicedChromeInfo:(NSDictionary *)chromeInfo
                      inSize:(CGSize)size
                     context:(CGContextRef)context
                       error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *json = chromeInfo[@"json"];
    NSString *chromePath = chromeInfo[@"chromePath"];
    NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
    NSDictionary *sizing = [images[@"sizing"] isKindOfClass:[NSDictionary class]] ? images[@"sizing"] : @{};
    CGFloat top = [self numberValue:sizing[@"topHeight"]];
    CGFloat left = [self numberValue:sizing[@"leftWidth"]];
    CGFloat bottom = [self numberValue:sizing[@"bottomHeight"]];
    CGFloat right = [self numberValue:sizing[@"rightWidth"]];

    NSString *topLeftPath = [self resolvedChromeAssetPathForName:[images[@"topLeft"] isKindOfClass:[NSString class]] ? images[@"topLeft"] : @"" chromePath:chromePath];
    NSString *topPath = [self resolvedChromeAssetPathForName:[images[@"top"] isKindOfClass:[NSString class]] ? images[@"top"] : @"" chromePath:chromePath];
    NSString *topRightPath = [self resolvedChromeAssetPathForName:[images[@"topRight"] isKindOfClass:[NSString class]] ? images[@"topRight"] : @"" chromePath:chromePath];
    NSString *leftPath = [self resolvedChromeAssetPathForName:[images[@"left"] isKindOfClass:[NSString class]] ? images[@"left"] : @"" chromePath:chromePath];
    NSString *rightPath = [self resolvedChromeAssetPathForName:[images[@"right"] isKindOfClass:[NSString class]] ? images[@"right"] : @"" chromePath:chromePath];
    NSString *bottomLeftPath = [self resolvedChromeAssetPathForName:[images[@"bottomLeft"] isKindOfClass:[NSString class]] ? images[@"bottomLeft"] : @"" chromePath:chromePath];
    NSString *bottomPath = [self resolvedChromeAssetPathForName:[images[@"bottom"] isKindOfClass:[NSString class]] ? images[@"bottom"] : @"" chromePath:chromePath];
    NSString *bottomRightPath = [self resolvedChromeAssetPathForName:[images[@"bottomRight"] isKindOfClass:[NSString class]] ? images[@"bottomRight"] : @"" chromePath:chromePath];

    CGSize topLeftSize = [self PDFPageSizeAtPath:topLeftPath];
    CGSize topSize = [self PDFPageSizeAtPath:topPath];
    CGSize topRightSize = [self PDFPageSizeAtPath:topRightPath];
    CGSize leftSize = [self PDFPageSizeAtPath:leftPath];
    CGSize rightSize = [self PDFPageSizeAtPath:rightPath];
    CGSize bottomLeftSize = [self PDFPageSizeAtPath:bottomLeftPath];
    CGSize bottomSize = [self PDFPageSizeAtPath:bottomPath];
    CGSize bottomRightSize = [self PDFPageSizeAtPath:bottomRightPath];

    CGFloat topHeight = MAX(MAX(MAX(top, topSize.height), topLeftSize.height), topRightSize.height);
    CGFloat leftWidth = MAX(MAX(MAX(left, leftSize.width), topLeftSize.width), bottomLeftSize.width);
    CGFloat bottomHeight = MAX(MAX(MAX(bottom, bottomSize.height), bottomLeftSize.height), bottomRightSize.height);
    CGFloat rightWidth = MAX(MAX(MAX(right, rightSize.width), topRightSize.width), bottomRightSize.width);
    CGFloat middleWidth = MAX(size.width - leftWidth - rightWidth, 1.0);
    CGFloat middleHeight = MAX(size.height - topHeight - bottomHeight, 1.0);

    NSArray<NSDictionary *> *pieces = @[
        @{ @"path": topLeftPath, @"rect": [NSValue valueWithRect:NSMakeRect(0, 0, leftWidth, topHeight)] },
        @{ @"path": topPath, @"rect": [NSValue valueWithRect:NSMakeRect(leftWidth, 0, middleWidth, topHeight)] },
        @{ @"path": topRightPath, @"rect": [NSValue valueWithRect:NSMakeRect(leftWidth + middleWidth, 0, rightWidth, topHeight)] },
        @{ @"path": leftPath, @"rect": [NSValue valueWithRect:NSMakeRect(0, topHeight, leftWidth, middleHeight)] },
        @{ @"path": rightPath, @"rect": [NSValue valueWithRect:NSMakeRect(leftWidth + middleWidth, topHeight, rightWidth, middleHeight)] },
        @{ @"path": bottomLeftPath, @"rect": [NSValue valueWithRect:NSMakeRect(0, topHeight + middleHeight, leftWidth, bottomHeight)] },
        @{ @"path": bottomPath, @"rect": [NSValue valueWithRect:NSMakeRect(leftWidth, topHeight + middleHeight, middleWidth, bottomHeight)] },
        @{ @"path": bottomRightPath, @"rect": [NSValue valueWithRect:NSMakeRect(leftWidth + middleWidth, topHeight + middleHeight, rightWidth, bottomHeight)] },
    ];

    BOOL drewAny = NO;
    for (NSDictionary *piece in pieces) {
        NSString *assetPath = piece[@"path"];
        if (assetPath.length == 0) {
            continue;
        }
        NSRect nsRect = [piece[@"rect"] rectValue];
        if (NSWidth(nsRect) <= 0.0 || NSHeight(nsRect) <= 0.0) {
            continue;
        }
        if ([self drawPDFAtPath:assetPath inRect:NSRectToCGRect(nsRect) context:context error:error]) {
            drewAny = YES;
        } else {
            return NO;
        }
    }
    if (!drewAny && error != NULL) {
        *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                     code:13
                                 userInfo:@{
            NSLocalizedDescriptionKey: @"The DeviceKit chrome did not expose renderable composite or sliced PDF assets.",
        }];
    }
    return drewAny;
}

+ (BOOL)drawInputImagesForChromeInfo:(NSDictionary *)chromeInfo
                               inSize:(CGSize)size
                              context:(CGContextRef)context
                            onlyOnTop:(BOOL)onlyOnTop
                                error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *json = chromeInfo[@"json"];
    NSString *chromePath = chromeInfo[@"chromePath"];
    NSArray *inputs = [json[@"inputs"] isKindOfClass:[NSArray class]] ? json[@"inputs"] : @[];
    for (id inputValue in inputs) {
        if (![inputValue isKindOfClass:[NSDictionary class]]) {
            continue;
        }
        NSDictionary *input = inputValue;
        BOOL onTop = [input[@"onTop"] respondsToSelector:@selector(boolValue)] && [input[@"onTop"] boolValue];
        if (onTop != onlyOnTop) {
            continue;
        }
        NSString *assetName = [input[@"image"] isKindOfClass:[NSString class]] ? input[@"image"] : @"";
        if (assetName.length == 0) {
            continue;
        }
        NSString *assetPath = [self resolvedChromeAssetPathForName:assetName chromePath:chromePath];
        CGSize assetSize = [self PDFPageSizeAtPath:assetPath];
        if (assetSize.width <= 0.0 || assetSize.height <= 0.0) {
            continue;
        }

        NSDictionary *offsets = [input[@"offsets"] isKindOfClass:[NSDictionary class]] ? input[@"offsets"] : @{};
        NSDictionary *normalOffset = [offsets[@"normal"] isKindOfClass:[NSDictionary class]] ? offsets[@"normal"] : @{};
        CGFloat offsetX = [self numberValue:normalOffset[@"x"]];
        CGFloat offsetY = [self numberValue:normalOffset[@"y"]];
        NSString *anchor = [input[@"anchor"] isKindOfClass:[NSString class]] ? input[@"anchor"] : @"";
        NSString *align = [input[@"align"] isKindOfClass:[NSString class]] ? input[@"align"] : @"";

        CGFloat x = offsetX;
        CGFloat y = offsetY;
        if ([anchor isEqualToString:@"right"]) {
            x = size.width + offsetX;
        } else if ([anchor isEqualToString:@"bottom"]) {
            y = size.height + offsetY;
        } else if ([anchor isEqualToString:@"left"]) {
            x = offsetX;
        } else if ([anchor isEqualToString:@"top"]) {
            y = offsetY;
            if ([align isEqualToString:@"trailing"]) {
                x = size.width + offsetX;
            } else if ([align isEqualToString:@"center"]) {
                x = (size.width - assetSize.width) / 2.0 + offsetX;
            }
        }

        CGRect rect = CGRectMake(x, y, assetSize.width, assetSize.height);
        if (![self drawPDFAtPath:assetPath inRect:rect context:context error:error]) {
            return NO;
        }
    }
    return YES;
}

+ (BOOL)isWatchProfile:(NSDictionary *)plist {
    NSString *chromeIdentifier = [plist[@"chromeIdentifier"] isKindOfClass:[NSString class]] ? plist[@"chromeIdentifier"] : @"";
    if ([chromeIdentifier containsString:@".watch"]) {
        return YES;
    }

    NSArray *families = [plist[@"supportedProductFamilyIDs"] isKindOfClass:[NSArray class]] ? plist[@"supportedProductFamilyIDs"] : @[];
    for (id family in families) {
        if ([family respondsToSelector:@selector(integerValue)] && [family integerValue] == 4) {
            return YES;
        }
    }
    return NO;
}

+ (CGSize)PDFPageSizeAtPath:(NSString *)path {
    if (path.length == 0) {
        return CGSizeZero;
    }
    CGPDFDocumentRef document = CGPDFDocumentCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path]);
    if (document == NULL) {
        return CGSizeZero;
    }
    CGPDFPageRef page = CGPDFDocumentGetPage(document, 1);
    CGRect mediaBox = page != NULL ? CGPDFPageGetBoxRect(page, kCGPDFMediaBox) : CGRectZero;
    CGPDFDocumentRelease(document);
    return mediaBox.size;
}

+ (BOOL)drawPDFAtPath:(NSString *)path
               inRect:(CGRect)rect
              context:(CGContextRef)context
                 error:(NSError * _Nullable __autoreleasing *)error {
    CGPDFDocumentRef document = CGPDFDocumentCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path]);
    if (document == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:7
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Unable to open DeviceKit chrome PDF %@.", path.lastPathComponent],
            }];
        }
        return NO;
    }
    CGPDFPageRef page = CGPDFDocumentGetPage(document, 1);
    if (page == NULL) {
        CGPDFDocumentRelease(document);
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:8
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"DeviceKit chrome PDF %@ did not contain a renderable page.", path.lastPathComponent],
            }];
        }
        return NO;
    }
    CGRect mediaBox = CGPDFPageGetBoxRect(page, kCGPDFMediaBox);
    CGContextSaveGState(context);
    CGContextTranslateCTM(context, rect.origin.x, rect.origin.y + rect.size.height);
    CGContextScaleCTM(context, rect.size.width / MAX(mediaBox.size.width, 1.0), -rect.size.height / MAX(mediaBox.size.height, 1.0));
    CGContextTranslateCTM(context, -mediaBox.origin.x, -mediaBox.origin.y);
    CGContextDrawPDFPage(context, page);
    CGContextRestoreGState(context);
    CGPDFDocumentRelease(document);
    return YES;
}

+ (NSString *)compositeAssetPathForChromeInfo:(NSDictionary *)chromeInfo {
    NSDictionary *json = chromeInfo[@"json"];
    NSString *chromePath = chromeInfo[@"chromePath"];
    NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
    NSString *name = [images[@"composite"] isKindOfClass:[NSString class]] ? images[@"composite"] : @"";
    if (name.length == 0) {
        name = [images[@"simpleComposite"] isKindOfClass:[NSString class]] ? images[@"simpleComposite"] : @"";
    }
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
