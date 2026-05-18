#import "XCWChromeRenderer.h"

#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>

static NSString * const XCWChromeRendererErrorDomain = @"SimDeck.ChromeRenderer";

@interface XCWChromeRenderer ()
+ (NSArray<NSDictionary<NSString *, id> *> *)buttonProfilesForChromeInfo:(NSDictionary *)chromeInfo
                                                               chromeSize:(CGSize)chromeSize
                                                             chromeOffset:(CGPoint)chromeOffset;
+ (nullable NSDictionary *)inputNamed:(NSString *)buttonName
                         chromeInfo:(NSDictionary *)chromeInfo
                              error:(NSError * _Nullable __autoreleasing *)error;
+ (NSEdgeInsets)devicePaddingForChromeInfo:(NSDictionary *)chromeInfo
                                chromeSize:(CGSize)chromeSize;
+ (CGFloat)inputScaleForChromeInfo:(NSDictionary *)chromeInfo
                         chromeSize:(CGSize)chromeSize;
+ (CGSize)scaledInputAssetSize:(CGSize)assetSize scale:(CGFloat)scale;
+ (BOOL)input:(NSDictionary *)input shouldDrawOnTopForChromeInfo:(NSDictionary *)chromeInfo;
@end

@implementation XCWChromeRenderer

+ (nullable NSDictionary<NSString *, id> *)profileForDeviceName:(NSString *)deviceName
                                                          error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *chromeInfo = [self chromeInfoForDeviceName:deviceName error:error];
    if (chromeInfo == nil) {
        return nil;
    }
    return [self profileForChromeInfo:chromeInfo error:error];
}

+ (nullable NSData *)PNGDataForDeviceName:(NSString *)deviceName
                                    error:(NSError * _Nullable __autoreleasing *)error {
    return [self PNGDataForDeviceName:deviceName includeButtons:YES error:error];
}

+ (nullable NSData *)PNGDataForDeviceName:(NSString *)deviceName
                            includeButtons:(BOOL)includeButtons
                                      error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *chromeInfo = [self chromeInfoForDeviceName:deviceName error:error];
    if (chromeInfo == nil) {
        return nil;
    }

    NSString *compositePath = [self compositeAssetPathForChromeInfo:chromeInfo];
    CGSize chromeSize = [self compositeSizeForChromeInfo:chromeInfo error:error];
    if (CGSizeEqualToSize(chromeSize, CGSizeZero)) {
        return nil;
    }

    NSDictionary *profile = [self profileForChromeInfo:chromeInfo error:error];
    if (profile == nil) {
        return nil;
    }
    CGSize renderSize = CGSizeMake([self numberValue:profile[@"totalWidth"]],
                                   [self numberValue:profile[@"totalHeight"]]);
    CGFloat chromeX = [self numberValue:profile[@"chromeX"]];
    CGFloat chromeY = [self numberValue:profile[@"chromeY"]];
    BOOL drawNonTopInputsBeforeBody = includeButtons;

    CGFloat scale = 3.0;
    NSInteger pixelWidth = MAX((NSInteger)ceil(renderSize.width * scale), 1);
    NSInteger pixelHeight = MAX((NSInteger)ceil(renderSize.height * scale), 1);

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
    CGContextTranslateCTM(context, chromeX, chromeY);
    if (drawNonTopInputsBeforeBody) {
        if (![self drawInputImagesForChromeInfo:chromeInfo
                                         inSize:chromeSize
                                        context:context
                                      onlyOnTop:NO
                                         error:error]) {
            CGContextRestoreGState(context);
            CGContextRelease(context);
            return nil;
        }
    }
    BOOL rendered = NO;
    if (compositePath.length > 0) {
        rendered = [self drawPDFAtPath:compositePath
                               inRect:CGRectMake(0, 0, chromeSize.width, chromeSize.height)
                              context:context
                                 error:error];
    } else {
        rendered = [self drawSlicedChromeInfo:chromeInfo
                                      inSize:chromeSize
                                     context:context
                                       error:error];
    }
    if (!rendered) {
        CGContextRestoreGState(context);
        CGContextRelease(context);
        return nil;
    }
    if (includeButtons && !drawNonTopInputsBeforeBody) {
        if (![self drawInputImagesForChromeInfo:chromeInfo
                                         inSize:chromeSize
                                        context:context
                                      onlyOnTop:NO
                                         error:error]) {
            CGContextRestoreGState(context);
            CGContextRelease(context);
            return nil;
        }
    }
    CGContextTranslateCTM(context, -chromeX, -chromeY);
    if (![self clearScreenAreaForChromeInfo:chromeInfo profile:profile context:context error:error]) {
        CGContextRestoreGState(context);
        CGContextRelease(context);
        return nil;
    }
    if (![self drawSensorBarForChromeInfo:chromeInfo profile:profile context:context error:error]) {
        CGContextRestoreGState(context);
        CGContextRelease(context);
        return nil;
    }
    CGContextTranslateCTM(context, chromeX, chromeY);
    if (includeButtons) {
        if (![self drawInputImagesForChromeInfo:chromeInfo
                                         inSize:chromeSize
                                       context:context
                                      onlyOnTop:YES
                                          error:error]) {
            CGContextRestoreGState(context);
            CGContextRelease(context);
            return nil;
        }
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

+ (nullable NSData *)buttonPNGDataForDeviceName:(NSString *)deviceName
                                     buttonName:(NSString *)buttonName
                                        pressed:(BOOL)pressed
                                          error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *chromeInfo = [self chromeInfoForDeviceName:deviceName error:error];
    if (chromeInfo == nil) {
        return nil;
    }
    NSDictionary *input = [self inputNamed:buttonName chromeInfo:chromeInfo error:error];
    if (input == nil) {
        return nil;
    }

    NSString *assetName = pressed && [input[@"imageDown"] isKindOfClass:[NSString class]]
        ? input[@"imageDown"]
        : input[@"image"];
    if (assetName.length == 0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:14
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"The chrome button `%@` did not specify a renderable image.", buttonName ?: @""],
            }];
        }
        return nil;
    }

    NSString *assetPath = [self resolvedChromeAssetPathForName:assetName chromePath:chromeInfo[@"chromePath"]];
    return [self PNGDataForPDFAtPath:assetPath scale:3.0 error:error];
}

+ (nullable NSData *)screenMaskPNGDataForDeviceName:(NSString *)deviceName
                                              error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *chromeInfo = [self chromeInfoForDeviceName:deviceName error:error];
    if (chromeInfo == nil) {
        return nil;
    }

    NSString *maskPath = [self screenMaskPathForChromeInfo:chromeInfo];
    if (maskPath.length == 0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:13
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"The device profile for %@ did not specify a framebuffer mask.", deviceName],
            }];
        }
        return nil;
    }

    return [self PNGDataForPDFAtPath:maskPath scale:1.0 error:error];
}

+ (nullable NSData *)screenshotPNGDataForDeviceName:(NSString *)deviceName
                                      screenPNGData:(NSData *)screenPNGData
                                              error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *profile = [self profileForDeviceName:deviceName error:error];
    if (profile == nil) {
        return nil;
    }

    NSData *chromePNGData = [self PNGDataForDeviceName:deviceName includeButtons:YES error:error];
    if (chromePNGData == nil) {
        return nil;
    }

    NSImage *screenImage = [[NSImage alloc] initWithData:screenPNGData];
    NSImage *chromeImage = [[NSImage alloc] initWithData:chromePNGData];
    if (screenImage == nil || chromeImage == nil) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:15
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to decode simulator screenshot or chrome PNG data.",
            }];
        }
        return nil;
    }

    CGFloat scale = 3.0;
    CGFloat totalWidth = [self numberValue:profile[@"totalWidth"]];
    CGFloat totalHeight = [self numberValue:profile[@"totalHeight"]];
    CGFloat screenX = [self numberValue:profile[@"screenX"]];
    CGFloat screenY = [self numberValue:profile[@"screenY"]];
    CGFloat screenWidth = [self numberValue:profile[@"screenWidth"]];
    CGFloat screenHeight = [self numberValue:profile[@"screenHeight"]];
    NSInteger pixelWidth = MAX((NSInteger)ceil(totalWidth * scale), 1);
    NSInteger pixelHeight = MAX((NSInteger)ceil(totalHeight * scale), 1);
    if (totalWidth <= 0.0 || totalHeight <= 0.0 || screenWidth <= 0.0 || screenHeight <= 0.0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:16
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Device chrome profile did not include usable screenshot geometry.",
            }];
        }
        return nil;
    }

    NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL
                                                                       pixelsWide:pixelWidth
                                                                       pixelsHigh:pixelHeight
                                                                    bitsPerSample:8
                                                                  samplesPerPixel:4
                                                                         hasAlpha:YES
                                                                         isPlanar:NO
                                                                   colorSpaceName:NSDeviceRGBColorSpace
                                                                      bytesPerRow:0
                                                                     bitsPerPixel:32];
    if (bitmap == nil) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:17
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to create a bitmap for bezeled screenshot rendering.",
            }];
        }
        return nil;
    }

    NSGraphicsContext *graphicsContext = [NSGraphicsContext graphicsContextWithBitmapImageRep:bitmap];
    [NSGraphicsContext saveGraphicsState];
    [NSGraphicsContext setCurrentContext:graphicsContext];
    graphicsContext.imageInterpolation = NSImageInterpolationHigh;
    NSRect outputRect = NSMakeRect(0.0, 0.0, pixelWidth, pixelHeight);
    [[NSColor clearColor] setFill];
    NSRectFillUsingOperation(outputRect, NSCompositingOperationClear);

    NSRect screenRect = NSMakeRect(screenX * scale,
                                   pixelHeight - ((screenY + screenHeight) * scale),
                                   screenWidth * scale,
                                   screenHeight * scale);
    NSDictionary *hints = @{ NSImageHintInterpolation: @(NSImageInterpolationHigh) };
    [screenImage drawInRect:screenRect
                   fromRect:NSZeroRect
                  operation:NSCompositingOperationSourceOver
                   fraction:1.0
             respectFlipped:NO
                      hints:hints];
    [chromeImage drawInRect:outputRect
                   fromRect:NSZeroRect
                  operation:NSCompositingOperationSourceOver
                   fraction:1.0
             respectFlipped:NO
                      hints:hints];
    [NSGraphicsContext restoreGraphicsState];

    NSData *pngData = [bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
    if (pngData.length == 0 && error != NULL) {
        *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                     code:18
                                 userInfo:@{
            NSLocalizedDescriptionKey: @"Unable to encode bezeled simulator screenshot PNG.",
        }];
    }
    return pngData.length > 0 ? pngData : nil;
}

+ (nullable NSDictionary<NSString *, id> *)profileForChromeInfo:(NSDictionary *)chromeInfo
                                                          error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *plist = chromeInfo[@"plist"];
    NSDictionary *json = chromeInfo[@"json"];
    NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
    NSDictionary *sizing = [images[@"sizing"] isKindOfClass:[NSDictionary class]] ? images[@"sizing"] : @{};
    NSDictionary *stand = [images[@"stand"] isKindOfClass:[NSDictionary class]] ? images[@"stand"] : @{};

    CGFloat sizingTop = [self numberValue:sizing[@"topHeight"]];
    CGFloat sizingLeft = [self numberValue:sizing[@"leftWidth"]];
    CGFloat sizingBottom = [self numberValue:sizing[@"bottomHeight"]];
    CGFloat sizingRight = [self numberValue:sizing[@"rightWidth"]];
    CGFloat standHeight = [self numberValue:stand[@"height"]];

    CGSize compositeSize = [self compositeSizeForChromeInfo:chromeInfo error:error];
    if (CGSizeEqualToSize(compositeSize, CGSizeZero)) {
        return nil;
    }

    NSDictionary *paths = [json[@"paths"] isKindOfClass:[NSDictionary class]] ? json[@"paths"] : @{};
    NSDictionary *border = [paths[@"simpleOutsideBorder"] isKindOfClass:[NSDictionary class]] ? paths[@"simpleOutsideBorder"] : @{};
    NSDictionary *borderInsets = [border[@"insets"] isKindOfClass:[NSDictionary class]] ? border[@"insets"] : @{};
    CGFloat rawCornerRadius = [self numberValue:border[@"cornerRadiusX"]];

    CGFloat borderTop = [self numberValue:borderInsets[@"top"]];
    CGFloat borderLeft = [self numberValue:borderInsets[@"left"]];
    CGFloat borderBottom = [self numberValue:borderInsets[@"bottom"]];
    CGFloat borderRight = [self numberValue:borderInsets[@"right"]];

    CGFloat bezelTop = sizingTop + borderTop;
    CGFloat bezelLeft = sizingLeft + borderLeft;
    CGFloat bezelBottom = sizingBottom + borderBottom;
    CGFloat bezelRight = sizingRight + borderRight;

    BOOL watchProfile = [self isWatchProfile:plist];
    BOOL phoneProfile = [self isPhoneProfile:plist];
    NSString *sensorName = [plist[@"sensorBarImage"] isKindOfClass:[NSString class]] ? plist[@"sensorBarImage"] : @"";
    BOOL hasModernPhoneSensor = [self shouldRenderPhoneChromeFromSlices:plist sensorName:sensorName];
    BOOL hasComposite = !hasModernPhoneSensor && [self compositeAssetPathForChromeInfo:chromeInfo].length > 0;
    CGFloat screenScale = MAX([self numberValue:plist[@"mainScreenScale"]], 1.0);
    CGSize profileScreenSize = [self screenSizeForChromeInfo:chromeInfo
                                                   chromeSize:compositeSize
                                                  screenScale:screenScale];
    CGFloat pointScreenWidth = profileScreenSize.width;
    CGFloat pointScreenHeight = profileScreenSize.height;

    CGFloat screenWidth;
    CGFloat screenHeight;
    CGFloat screenX;
    CGFloat screenY;
    if (watchProfile) {
        CGFloat usableHeight = MAX(compositeSize.height - standHeight, 1.0);
        screenWidth = pointScreenWidth > 0.0
            ? MIN(pointScreenWidth, compositeSize.width)
            : MAX(compositeSize.width - sizingLeft - sizingRight, 1.0);
        screenHeight = pointScreenHeight > 0.0
            ? MIN(pointScreenHeight, usableHeight)
            : MAX(usableHeight - sizingTop - sizingBottom, 1.0);
        screenX = MAX((compositeSize.width - screenWidth) / 2.0, 0.0);
        screenY = MAX((usableHeight - screenHeight) / 2.0, 0.0);
    } else if (hasComposite && pointScreenWidth > 0.0 && pointScreenHeight > 0.0) {
        screenWidth = pointScreenWidth;
        screenHeight = pointScreenHeight;
        screenX = MAX((compositeSize.width - screenWidth) / 2.0, 0.0);
        CGFloat usableHeight = compositeSize.height - standHeight;
        screenY = MAX((usableHeight - screenHeight) / 2.0, bezelTop);
    } else {
        screenX = bezelLeft;
        screenY = bezelTop;
        screenWidth = MAX(compositeSize.width - bezelLeft - bezelRight, 1.0);
        screenHeight = MAX(compositeSize.height - standHeight - bezelTop - bezelBottom, 1.0);
    }

    CGFloat innerRadius = MAX(rawCornerRadius - MAX(screenX, screenY), 0.0);
    CGFloat radiusScale = !watchProfile && pointScreenWidth > 0.0 ? screenWidth / pointScreenWidth : 1.0;
    CGFloat chromeCornerRadius = innerRadius * radiusScale;
    CGFloat cornerRadius = chromeCornerRadius;

    CGRect fullFrame = [self fullFrameForChromeInfo:chromeInfo chromeSize:compositeSize];
    CGFloat chromeX = -CGRectGetMinX(fullFrame);
    CGFloat chromeY = -CGRectGetMinY(fullFrame);
    BOOL hasScreenMask = !phoneProfile && [self screenMaskPathForChromeInfo:chromeInfo].length > 0;
    NSArray<NSDictionary<NSString *, id> *> *buttons = [self buttonProfilesForChromeInfo:chromeInfo
                                                                              chromeSize:compositeSize
                                                                            chromeOffset:CGPointMake(chromeX, chromeY)];

    return @{
        @"totalWidth": @(CGRectGetWidth(fullFrame)),
        @"totalHeight": @(CGRectGetHeight(fullFrame)),
        @"chromeX": @(chromeX),
        @"chromeY": @(chromeY),
        @"chromeWidth": @(compositeSize.width),
        @"chromeHeight": @(compositeSize.height),
        @"screenX": @(screenX + chromeX),
        @"screenY": @(screenY + chromeY),
        @"screenWidth": @(screenWidth),
        @"screenHeight": @(screenHeight),
        @"cornerRadius": @(cornerRadius),
        @"chromeCornerRadius": @(chromeCornerRadius),
        @"hasScreenMask": @(hasScreenMask),
        @"buttons": buttons,
    };
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
        @"profileResourcesPath": profilePath.stringByDeletingLastPathComponent,
    };
}

+ (CGSize)compositeSizeForChromeInfo:(NSDictionary *)chromeInfo
                               error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *plist = chromeInfo[@"plist"];
    NSString *sensorName = [plist[@"sensorBarImage"] isKindOfClass:[NSString class]] ? plist[@"sensorBarImage"] : @"";
    BOOL hasModernPhoneSensor = [self shouldRenderPhoneChromeFromSlices:plist sensorName:sensorName];
    NSString *compositePath = hasModernPhoneSensor ? @"" : [self compositeAssetPathForChromeInfo:chromeInfo];
    if (compositePath.length == 0) {
        NSDictionary *json = chromeInfo[@"json"];
        NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
        NSDictionary *sizing = [images[@"sizing"] isKindOfClass:[NSDictionary class]] ? images[@"sizing"] : @{};
        NSDictionary *paths = [json[@"paths"] isKindOfClass:[NSDictionary class]] ? json[@"paths"] : @{};
        NSDictionary *bord = [paths[@"simpleOutsideBorder"] isKindOfClass:[NSDictionary class]] ? paths[@"simpleOutsideBorder"] : @{};
        NSDictionary *bordI = [bord[@"insets"] isKindOfClass:[NSDictionary class]] ? bord[@"insets"] : @{};
        CGFloat screenScale = MAX([self numberValue:plist[@"mainScreenScale"]], 1.0);
        CGSize screenSize = [self screenSizeForChromeInfo:chromeInfo
                                               chromeSize:CGSizeZero
                                              screenScale:screenScale];
        CGFloat screenWidth = screenSize.width;
        CGFloat screenHeight = screenSize.height;
        CGFloat bezelLeft = [self numberValue:sizing[@"leftWidth"]] + [self numberValue:bordI[@"left"]];
        CGFloat bezelRight = [self numberValue:sizing[@"rightWidth"]] + [self numberValue:bordI[@"right"]];
        CGFloat bezelTop = [self numberValue:sizing[@"topHeight"]] + [self numberValue:bordI[@"top"]];
        CGFloat bezelBottom = [self numberValue:sizing[@"bottomHeight"]] + [self numberValue:bordI[@"bottom"]];
        NSDictionary *stand = [images[@"stand"] isKindOfClass:[NSDictionary class]] ? images[@"stand"] : @{};
        CGFloat standHeight = [self numberValue:stand[@"height"]];
        CGFloat totalWidth = screenWidth + bezelLeft + bezelRight;
        CGFloat totalHeight = screenHeight + bezelTop + bezelBottom + standHeight;
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
    NSDictionary *stand = [images[@"stand"] isKindOfClass:[NSDictionary class]] ? images[@"stand"] : @{};
    CGFloat standHeight = [self numberValue:stand[@"height"]];
    CGFloat chromeHeight = MAX(size.height - standHeight, 1.0);
    CGFloat middleHeight = MAX(chromeHeight - topHeight - bottomHeight, 1.0);

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
        if ([self drawRasterizedPDFAtPath:assetPath inRect:NSRectToCGRect(nsRect) context:context error:error]) {
            drewAny = YES;
        } else {
            return NO;
        }
    }
    if (standHeight > 0.0 && ![self drawStandImagesForChromeInfo:chromeInfo
                                                           inSize:size
                                                        chromeYMax:chromeHeight
                                                          context:context
                                                            error:error]) {
        return NO;
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

+ (BOOL)drawStandImagesForChromeInfo:(NSDictionary *)chromeInfo
                               inSize:(CGSize)size
                            chromeYMax:(CGFloat)chromeYMax
                              context:(CGContextRef)context
                                error:(NSError * _Nullable __autoreleasing *)error {
    NSDictionary *json = chromeInfo[@"json"];
    NSString *chromePath = chromeInfo[@"chromePath"];
    NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
    NSDictionary *stand = [images[@"stand"] isKindOfClass:[NSDictionary class]] ? images[@"stand"] : @{};
    CGFloat standWidth = [self numberValue:stand[@"width"]];
    CGFloat standHeight = [self numberValue:stand[@"height"]];
    if (standWidth <= 0.0 || standHeight <= 0.0) {
        return YES;
    }

    NSString *leftName = [stand[@"left"] isKindOfClass:[NSString class]] ? stand[@"left"] : @"";
    NSString *centerName = [stand[@"center"] isKindOfClass:[NSString class]] ? stand[@"center"] : @"";
    NSString *rightName = [stand[@"right"] isKindOfClass:[NSString class]] ? stand[@"right"] : @"";
    NSString *leftPath = leftName.length > 0 ? [self resolvedChromeAssetPathForName:leftName chromePath:chromePath] : @"";
    NSString *centerPath = centerName.length > 0 ? [self resolvedChromeAssetPathForName:centerName chromePath:chromePath] : @"";
    NSString *rightPath = rightName.length > 0 ? [self resolvedChromeAssetPathForName:rightName chromePath:chromePath] : @"";
    CGSize leftSize = [self PDFPageSizeAtPath:leftPath];
    CGSize rightSize = [self PDFPageSizeAtPath:rightPath];
    CGFloat leftWidth = MAX(leftSize.width, 0.0);
    CGFloat rightWidth = MAX(rightSize.width, 0.0);
    CGFloat centerWidth = MAX(standWidth - leftWidth - rightWidth, 1.0);
    CGFloat x = MAX((size.width - standWidth) / 2.0, 0.0);
    CGFloat y = chromeYMax;

    if (leftPath.length > 0 && leftWidth > 0.0) {
        if (![self drawRasterizedPDFAtPath:leftPath
                                     inRect:CGRectMake(x, y, leftWidth, standHeight)
                                    context:context
                                      error:error]) {
            return NO;
        }
    }
    if (centerPath.length > 0) {
        if (![self drawRasterizedPDFAtPath:centerPath
                                     inRect:CGRectMake(x + leftWidth, y, centerWidth, standHeight)
                                    context:context
                                      error:error]) {
            return NO;
        }
    }
    if (rightPath.length > 0 && rightWidth > 0.0) {
        if (![self drawRasterizedPDFAtPath:rightPath
                                     inRect:CGRectMake(x + leftWidth + centerWidth, y, rightWidth, standHeight)
                                    context:context
                                      error:error]) {
            return NO;
        }
    }
    return YES;
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
        BOOL onTop = [self input:input shouldDrawOnTopForChromeInfo:chromeInfo];
        if (onTop != onlyOnTop) {
            continue;
        }
        NSString *assetName = [input[@"image"] isKindOfClass:[NSString class]] ? input[@"image"] : @"";
        if (assetName.length == 0) {
            continue;
        }
        NSString *assetPath = [self resolvedChromeAssetPathForName:assetName chromePath:chromePath];
        CGFloat inputScale = [self inputScaleForChromeInfo:chromeInfo chromeSize:size];
        CGSize assetSize = [self scaledInputAssetSize:[self PDFPageSizeAtPath:assetPath]
                                                scale:inputScale];
        if (assetSize.width <= 0.0 || assetSize.height <= 0.0) {
            continue;
        }

        CGRect rect = [self inputFrameForInput:input
                                     assetSize:assetSize
                                        inSize:size
                                         scale:inputScale];
        if (![self drawPDFAtPath:assetPath inRect:rect context:context error:error]) {
            return NO;
        }
    }
    return YES;
}

+ (CGRect)fullFrameForChromeInfo:(NSDictionary *)chromeInfo
                       chromeSize:(CGSize)chromeSize {
    NSEdgeInsets padding = [self devicePaddingForChromeInfo:chromeInfo chromeSize:chromeSize];
    if (padding.top != 0.0 || padding.left != 0.0 || padding.bottom != 0.0 || padding.right != 0.0) {
        return CGRectMake(-padding.left,
                          -padding.top,
                          chromeSize.width + padding.left + padding.right,
                          chromeSize.height + padding.top + padding.bottom);
    }

    CGRect bounds = CGRectMake(0.0, 0.0, chromeSize.width, chromeSize.height);
    NSDictionary *json = chromeInfo[@"json"];
    NSString *chromePath = chromeInfo[@"chromePath"];
    BOOL hasComposite = [self compositeAssetPathForChromeInfo:chromeInfo].length > 0;
    BOOL watchProfile = [self isWatchProfile:chromeInfo[@"plist"]];
    NSArray *inputs = [json[@"inputs"] isKindOfClass:[NSArray class]] ? json[@"inputs"] : @[];
    for (id inputValue in inputs) {
        if (![inputValue isKindOfClass:[NSDictionary class]]) {
            continue;
        }
        NSDictionary *input = inputValue;
        BOOL onTop = [self input:input shouldDrawOnTopForChromeInfo:chromeInfo];
        if (hasComposite && watchProfile && !onTop) {
            continue;
        }
        NSString *assetName = [input[@"image"] isKindOfClass:[NSString class]] ? input[@"image"] : @"";
        if (assetName.length == 0) {
            continue;
        }
        NSString *assetPath = [self resolvedChromeAssetPathForName:assetName chromePath:chromePath];
        CGFloat inputScale = [self inputScaleForChromeInfo:chromeInfo chromeSize:chromeSize];
        CGSize assetSize = [self scaledInputAssetSize:[self PDFPageSizeAtPath:assetPath]
                                                scale:inputScale];
        if (assetSize.width <= 0.0 || assetSize.height <= 0.0) {
            continue;
        }
        bounds = CGRectUnion(bounds, [self inputFrameForInput:input
                                                    assetSize:assetSize
                                                       inSize:chromeSize
                                                        scale:inputScale
                                                   offsetName:@"normal"]);
        bounds = CGRectUnion(bounds, [self inputFrameForInput:input
                                                    assetSize:assetSize
                                                       inSize:chromeSize
                                                        scale:inputScale
                                                   offsetName:@"rollover"]);
    }
    return CGRectIntegral(bounds);
}

+ (CGRect)inputFrameForInput:(NSDictionary *)input
                   assetSize:(CGSize)assetSize
                      inSize:(CGSize)size {
    return [self inputFrameForInput:input assetSize:assetSize inSize:size scale:1.0 offsetName:@"normal"];
}

+ (CGRect)inputFrameForInput:(NSDictionary *)input
                   assetSize:(CGSize)assetSize
                      inSize:(CGSize)size
                       scale:(CGFloat)scale {
    return [self inputFrameForInput:input assetSize:assetSize inSize:size scale:scale offsetName:@"normal"];
}

+ (CGRect)inputFrameForInput:(NSDictionary *)input
                   assetSize:(CGSize)assetSize
                      inSize:(CGSize)size
                  offsetName:(NSString *)offsetName {
    return [self inputFrameForInput:input
                          assetSize:assetSize
                             inSize:size
                              scale:1.0
                         offsetName:offsetName];
}

+ (CGRect)inputFrameForInput:(NSDictionary *)input
                   assetSize:(CGSize)assetSize
                      inSize:(CGSize)size
                       scale:(CGFloat)scale
                  offsetName:(NSString *)offsetName {
    NSDictionary *offsets = [input[@"offsets"] isKindOfClass:[NSDictionary class]] ? input[@"offsets"] : @{};
    NSDictionary *normalOffset = [offsets[@"normal"] isKindOfClass:[NSDictionary class]] ? offsets[@"normal"] : nil;
    NSDictionary *rolloverOffset = [offsets[@"rollover"] isKindOfClass:[NSDictionary class]] ? offsets[@"rollover"] : nil;
    NSDictionary *requestedOffset = [offsets[offsetName] isKindOfClass:[NSDictionary class]] ? offsets[offsetName] : nil;
    NSDictionary *primaryOffset = normalOffset ?: rolloverOffset ?: requestedOffset ?: @{};
    NSDictionary *secondaryOffset = rolloverOffset ?: normalOffset ?: requestedOffset ?: @{};
    CGFloat coordinateScale = MAX(scale, 1.0);
    CGFloat normalX = [self numberValue:primaryOffset[@"x"]] * coordinateScale;
    CGFloat normalY = [self numberValue:primaryOffset[@"y"]] * coordinateScale;
    CGFloat rolloverX = [self numberValue:secondaryOffset[@"x"]] * coordinateScale;
    CGFloat rolloverY = [self numberValue:secondaryOffset[@"y"]] * coordinateScale;
    CGFloat restX = normalX;
    CGFloat restY = normalY;
    NSString *anchor = [input[@"anchor"] isKindOfClass:[NSString class]] ? input[@"anchor"] : @"";
    NSString *align = [input[@"align"] isKindOfClass:[NSString class]] ? input[@"align"] : @"";

    if ([offsetName isEqualToString:@"rollover"]) {
        restX = rolloverX;
        restY = rolloverY;
    } else if ([anchor isEqualToString:@"left"]) {
        restX = rolloverX;
        restY = rolloverY;
    } else if ([anchor isEqualToString:@"right"] ||
               [anchor isEqualToString:@"top"] ||
               [anchor isEqualToString:@"bottom"]) {
        restX = (2.0 * normalX) - rolloverX;
        restY = (2.0 * normalY) - rolloverY;
    }

    CGFloat x = restX - (assetSize.width / 2.0);
    CGFloat y = restY;
    if ([anchor isEqualToString:@"left"]) {
        x = restX - (assetSize.width / 2.0);
    } else if ([anchor isEqualToString:@"right"]) {
        x = size.width + restX;
        y = restY;
    } else if ([anchor isEqualToString:@"top"]) {
        if ([align isEqualToString:@"trailing"]) {
            x = size.width + restX - assetSize.width;
        } else {
            x = restX;
        }
        y = restY - assetSize.height;
    } else if ([anchor isEqualToString:@"bottom"]) {
        if ([align isEqualToString:@"trailing"]) {
            x = size.width + restX - assetSize.width;
        } else {
            x = restX;
        }
        y = size.height + restY;
    }

    if ([anchor isEqualToString:@"left"] || [anchor isEqualToString:@"right"]) {
        if ([align isEqualToString:@"center"]) {
            y = (size.height - assetSize.height) / 2.0 + restY;
        } else if ([align isEqualToString:@"trailing"]) {
            y = size.height - assetSize.height + restY;
        }
    } else if ([anchor isEqualToString:@"top"] || [anchor isEqualToString:@"bottom"]) {
        if ([align isEqualToString:@"center"]) {
            x = (size.width / 2.0) + restX - (assetSize.width / 2.0);
        }
    } else if ([align isEqualToString:@"center"]) {
        x = (size.width - assetSize.width) / 2.0 + restX;
    } else if ([align isEqualToString:@"trailing"]) {
        x = size.width - assetSize.width + restX;
    }

    return CGRectMake(x, y, assetSize.width, assetSize.height);
}

+ (NSEdgeInsets)devicePaddingForChromeInfo:(NSDictionary *)chromeInfo
                                chromeSize:(CGSize)chromeSize {
    NSDictionary *json = chromeInfo[@"json"];
    NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
    NSDictionary *padding = [images[@"devicePadding"] isKindOfClass:[NSDictionary class]] ? images[@"devicePadding"] : @{};
    CGFloat inputScale = [self inputScaleForChromeInfo:chromeInfo chromeSize:chromeSize];
    return NSEdgeInsetsMake([self numberValue:padding[@"top"]] * inputScale,
                            [self numberValue:padding[@"left"]] * inputScale,
                            [self numberValue:padding[@"bottom"]] * inputScale,
                            [self numberValue:padding[@"right"]] * inputScale);
}

+ (NSArray<NSDictionary<NSString *, id> *> *)buttonProfilesForChromeInfo:(NSDictionary *)chromeInfo
                                                               chromeSize:(CGSize)chromeSize
                                                             chromeOffset:(CGPoint)chromeOffset {
    NSDictionary *json = chromeInfo[@"json"];
    NSString *chromePath = chromeInfo[@"chromePath"];
    NSArray *inputs = [json[@"inputs"] isKindOfClass:[NSArray class]] ? json[@"inputs"] : @[];
    NSMutableArray<NSDictionary<NSString *, id> *> *buttons = [NSMutableArray arrayWithCapacity:inputs.count];

    for (id inputValue in inputs) {
        if (![inputValue isKindOfClass:[NSDictionary class]]) {
            continue;
        }
        NSDictionary *input = inputValue;
        NSString *name = [input[@"name"] isKindOfClass:[NSString class]] ? input[@"name"] : @"";
        NSString *assetName = [input[@"image"] isKindOfClass:[NSString class]] ? input[@"image"] : @"";
        if (name.length == 0 || assetName.length == 0) {
            continue;
        }

        NSString *assetPath = [self resolvedChromeAssetPathForName:assetName chromePath:chromePath];
        CGFloat inputScale = [self inputScaleForChromeInfo:chromeInfo chromeSize:chromeSize];
        CGSize assetSize = [self scaledInputAssetSize:[self PDFPageSizeAtPath:assetPath]
                                                scale:inputScale];
        if (assetSize.width <= 0.0 || assetSize.height <= 0.0) {
            continue;
        }

        CGRect rect = [self inputFrameForInput:input
                                     assetSize:assetSize
                                        inSize:chromeSize
                                         scale:inputScale];
        rect = CGRectOffset(rect, chromeOffset.x, chromeOffset.y);
        if (CGRectGetWidth(rect) <= 0.0 || CGRectGetHeight(rect) <= 0.0) {
            continue;
        }

        NSDictionary *offsets = [input[@"offsets"] isKindOfClass:[NSDictionary class]] ? input[@"offsets"] : @{};
        NSDictionary *normalOffset = [offsets[@"normal"] isKindOfClass:[NSDictionary class]] ? offsets[@"normal"] : @{};
        NSDictionary *rolloverOffset = [offsets[@"rollover"] isKindOfClass:[NSDictionary class]] ? offsets[@"rollover"] : normalOffset;
        NSString *label = [input[@"accessibilityTitle"] isKindOfClass:[NSString class]] ? input[@"accessibilityTitle"] : name;
        NSString *type = [input[@"type"] isKindOfClass:[NSString class]] ? input[@"type"] : @"";
        NSString *anchor = [input[@"anchor"] isKindOfClass:[NSString class]] ? input[@"anchor"] : @"";
        NSString *align = [input[@"align"] isKindOfClass:[NSString class]] ? input[@"align"] : @"";
        NSString *imageDownName = [input[@"imageDown"] isKindOfClass:[NSString class]] ? input[@"imageDown"] : @"";
        NSString *imageDownDrawMode = [input[@"imageDownDrawMode"] isKindOfClass:[NSString class]] ? input[@"imageDownDrawMode"] : @"";
        BOOL onTop = [self input:input shouldDrawOnTopForChromeInfo:chromeInfo];

        NSMutableDictionary<NSString *, id> *button = [@{
            @"name": name,
            @"label": label,
            @"type": type,
            @"imageName": assetName,
            @"x": @(CGRectGetMinX(rect)),
            @"y": @(CGRectGetMinY(rect)),
            @"width": @(CGRectGetWidth(rect)),
            @"height": @(CGRectGetHeight(rect)),
            @"anchor": anchor,
            @"align": align,
            @"onTop": @(onTop),
            @"normalOffset": @{
                @"x": @([self numberValue:normalOffset[@"x"]] * inputScale),
                @"y": @([self numberValue:normalOffset[@"y"]] * inputScale),
            },
            @"rolloverOffset": @{
                @"x": @([self numberValue:rolloverOffset[@"x"]] * inputScale),
                @"y": @([self numberValue:rolloverOffset[@"y"]] * inputScale),
            },
        } mutableCopy];

        if (imageDownName.length > 0) {
            button[@"imageDownName"] = imageDownName;
        }
        if (imageDownDrawMode.length > 0) {
            button[@"imageDownDrawMode"] = imageDownDrawMode;
        }

        if ([input[@"usagePage"] respondsToSelector:@selector(unsignedIntValue)]) {
            button[@"usagePage"] = @([input[@"usagePage"] unsignedIntValue]);
        }
        if ([input[@"usage"] respondsToSelector:@selector(unsignedIntValue)]) {
            button[@"usage"] = @([input[@"usage"] unsignedIntValue]);
        }

        [buttons addObject:button];
    }

    return buttons;
}

+ (nullable NSDictionary *)inputNamed:(NSString *)buttonName
                            chromeInfo:(NSDictionary *)chromeInfo
                                 error:(NSError * _Nullable __autoreleasing *)error {
    NSString *normalizedName = [[buttonName ?: @"" stringByDeletingPathExtension] lowercaseString];
    NSDictionary *json = chromeInfo[@"json"];
    NSArray *inputs = [json[@"inputs"] isKindOfClass:[NSArray class]] ? json[@"inputs"] : @[];
    for (id inputValue in inputs) {
        if (![inputValue isKindOfClass:[NSDictionary class]]) {
            continue;
        }
        NSDictionary *input = inputValue;
        NSString *name = [input[@"name"] isKindOfClass:[NSString class]] ? input[@"name"] : @"";
        if ([name.lowercaseString isEqualToString:normalizedName]) {
            return input;
        }
    }

    if (error != NULL) {
        *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                     code:15
                                 userInfo:@{
            NSLocalizedDescriptionKey: [NSString stringWithFormat:@"The device chrome did not expose a button named `%@`.", buttonName ?: @""],
        }];
    }
    return nil;
}

+ (CGSize)screenSizeForChromeInfo:(NSDictionary *)chromeInfo
                        chromeSize:(CGSize)chromeSize
                       screenScale:(CGFloat)screenScale {
    NSDictionary *plist = chromeInfo[@"plist"];
    CGFloat rawWidth = [self numberValue:plist[@"mainScreenWidth"]];
    CGFloat rawHeight = [self numberValue:plist[@"mainScreenHeight"]];
    CGFloat scale = MAX(screenScale, 1.0);
    if (![self isWatchProfile:plist]) {
        return CGSizeMake(rawWidth / scale, rawHeight / scale);
    }

    return CGSizeMake(rawWidth, rawHeight);
}

+ (CGFloat)inputScaleForChromeInfo:(NSDictionary *)chromeInfo
                         chromeSize:(CGSize)chromeSize {
    NSDictionary *plist = chromeInfo[@"plist"];
    if (![self isWatchProfile:plist]) {
        return 1.0;
    }
    // watchOS profiles expose screen geometry in raw pixels, while input PDFs
    // and chrome offsets are authored against DeviceKit's nominal screen slot.
    CGFloat screenScale = MAX([self numberValue:plist[@"mainScreenScale"]], 1.0);
    NSDictionary *json = chromeInfo[@"json"];
    NSDictionary *images = [json[@"images"] isKindOfClass:[NSDictionary class]] ? json[@"images"] : @{};
    NSDictionary *sizing = [images[@"sizing"] isKindOfClass:[NSDictionary class]] ? images[@"sizing"] : @{};
    NSDictionary *stand = [images[@"stand"] isKindOfClass:[NSDictionary class]] ? images[@"stand"] : @{};
    CGFloat nominalWidth = chromeSize.width -
        [self numberValue:sizing[@"leftWidth"]] -
        [self numberValue:sizing[@"rightWidth"]];
    CGFloat nominalHeight = chromeSize.height -
        [self numberValue:stand[@"height"]] -
        [self numberValue:sizing[@"topHeight"]] -
        [self numberValue:sizing[@"bottomHeight"]];
    CGFloat screenWidth = [self numberValue:plist[@"mainScreenWidth"]];
    CGFloat screenHeight = [self numberValue:plist[@"mainScreenHeight"]];
    if (nominalWidth <= 0.0 || nominalHeight <= 0.0 || screenWidth <= 0.0 || screenHeight <= 0.0) {
        return screenScale;
    }
    CGFloat fitScale = MIN(screenWidth / nominalWidth, screenHeight / nominalHeight);
    if (!isfinite(fitScale) || fitScale <= 0.0) {
        return screenScale;
    }
    return screenScale * MIN(fitScale, 1.0);
}

+ (CGSize)scaledInputAssetSize:(CGSize)assetSize scale:(CGFloat)scale {
    CGFloat inputScale = MAX(scale, 1.0);
    return CGSizeMake(assetSize.width * inputScale, assetSize.height * inputScale);
}

+ (BOOL)input:(NSDictionary *)input shouldDrawOnTopForChromeInfo:(NSDictionary *)chromeInfo {
    if ([input[@"onTop"] respondsToSelector:@selector(boolValue)] && [input[@"onTop"] boolValue]) {
        return YES;
    }
    NSString *type = [input[@"type"] isKindOfClass:[NSString class]] ? input[@"type"] : @"";
    NSString *name = [input[@"name"] isKindOfClass:[NSString class]] ? input[@"name"] : @"";
    return [self isWatchProfile:chromeInfo[@"plist"]] &&
        ([type isEqualToString:@"crown"] || [name isEqualToString:@"digital-crown"]);
}

+ (BOOL)drawRasterizedPDFAtPath:(NSString *)path
                         inRect:(CGRect)rect
                        context:(CGContextRef)context
                          error:(NSError * _Nullable __autoreleasing *)error {
    CGImageRef image = [self newImageForPDFAtPath:path error:error];
    if (image == NULL) {
        return NO;
    }

    CGFloat imageWidth = MAX((CGFloat)CGImageGetWidth(image), 1.0);
    CGFloat imageHeight = MAX((CGFloat)CGImageGetHeight(image), 1.0);
    CGContextSaveGState(context);
    CGContextClipToRect(context, rect);
    CGContextTranslateCTM(context, rect.origin.x, rect.origin.y + rect.size.height);
    CGContextScaleCTM(context, rect.size.width / imageWidth, -rect.size.height / imageHeight);
    CGContextDrawImage(context, CGRectMake(0, 0, imageWidth, imageHeight), image);
    CGContextRestoreGState(context);
    CGImageRelease(image);
    return YES;
}

+ (nullable CGImageRef)newImageForPDFAtPath:(NSString *)path
                                      error:(NSError * _Nullable __autoreleasing *)error CF_RETURNS_RETAINED {
    if (path.length == 0) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:14
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"DeviceKit chrome asset path was empty.",
            }];
        }
        return NULL;
    }

    CGPDFDocumentRef document = CGPDFDocumentCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path]);
    if (document == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:7
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Unable to open DeviceKit chrome PDF %@.", path.lastPathComponent],
            }];
        }
        return NULL;
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
        return NULL;
    }

    CGRect mediaBox = CGPDFPageGetBoxRect(page, kCGPDFCropBox);
    if (CGRectIsEmpty(mediaBox)) {
        mediaBox = CGPDFPageGetBoxRect(page, kCGPDFMediaBox);
    }
    NSInteger width = MAX((NSInteger)ceil(mediaBox.size.width), 1);
    NSInteger height = MAX((NSInteger)ceil(mediaBox.size.height), 1);
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(NULL,
                                                 width,
                                                 height,
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
                NSLocalizedDescriptionKey: @"Unable to create a CoreGraphics bitmap context for DeviceKit chrome asset rendering.",
            }];
        }
        return NULL;
    }

    CGContextClearRect(context, CGRectMake(0, 0, width, height));
    CGContextTranslateCTM(context, -mediaBox.origin.x, -mediaBox.origin.y);
    CGContextDrawPDFPage(context, page);
    CGImageRef image = CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    CGPDFDocumentRelease(document);
    if (image == NULL && error != NULL) {
        *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                     code:10
                                 userInfo:@{
            NSLocalizedDescriptionKey: @"Unable to create a CGImage from the DeviceKit chrome asset.",
        }];
    }
    return image;
}

+ (BOOL)clearScreenAreaForChromeInfo:(NSDictionary *)chromeInfo
                              profile:(NSDictionary *)profile
                              context:(CGContextRef)context
                                error:(NSError * _Nullable __autoreleasing *)error {
    CGFloat x = [self numberValue:profile[@"screenX"]];
    CGFloat y = [self numberValue:profile[@"screenY"]];
    CGFloat width = [self numberValue:profile[@"screenWidth"]];
    CGFloat height = [self numberValue:profile[@"screenHeight"]];
    if (width <= 0.0 || height <= 0.0) {
        return YES;
    }

    CGRect rect = CGRectMake(x, y, width, height);
    BOOL hasScreenMask = [profile[@"hasScreenMask"] respondsToSelector:@selector(boolValue)] && [profile[@"hasScreenMask"] boolValue];
    if (hasScreenMask) {
        NSString *maskPath = [self screenMaskPathForChromeInfo:chromeInfo];
        if (maskPath.length > 0) {
            CGImageRef maskImage = [self newImageForPDFAtPath:maskPath error:error];
            if (maskImage == NULL) {
                return NO;
            }
            CGContextSaveGState(context);
            CGContextClipToMask(context, rect, maskImage);
            CGContextSetBlendMode(context, kCGBlendModeClear);
            CGContextFillRect(context, rect);
            CGContextRestoreGState(context);
            CGImageRelease(maskImage);
            return YES;
        }
    }

    CGFloat radius = [self numberValue:profile[@"cornerRadius"]];
    if (radius <= 0.0) {
        radius = [self numberValue:profile[@"chromeCornerRadius"]];
    }
    CGFloat clampedRadius = MIN(MAX(radius, 0.0), MIN(width, height) / 2.0);

    CGContextSaveGState(context);
    CGContextSetBlendMode(context, kCGBlendModeClear);
    if (clampedRadius <= 0.0) {
        CGContextFillRect(context, rect);
    } else {
        CGMutablePathRef path = CGPathCreateMutable();
        CGPathMoveToPoint(path, NULL, CGRectGetMinX(rect) + clampedRadius, CGRectGetMinY(rect));
        CGPathAddLineToPoint(path, NULL, CGRectGetMaxX(rect) - clampedRadius, CGRectGetMinY(rect));
        CGPathAddArcToPoint(path, NULL, CGRectGetMaxX(rect), CGRectGetMinY(rect), CGRectGetMaxX(rect), CGRectGetMinY(rect) + clampedRadius, clampedRadius);
        CGPathAddLineToPoint(path, NULL, CGRectGetMaxX(rect), CGRectGetMaxY(rect) - clampedRadius);
        CGPathAddArcToPoint(path, NULL, CGRectGetMaxX(rect), CGRectGetMaxY(rect), CGRectGetMaxX(rect) - clampedRadius, CGRectGetMaxY(rect), clampedRadius);
        CGPathAddLineToPoint(path, NULL, CGRectGetMinX(rect) + clampedRadius, CGRectGetMaxY(rect));
        CGPathAddArcToPoint(path, NULL, CGRectGetMinX(rect), CGRectGetMaxY(rect), CGRectGetMinX(rect), CGRectGetMaxY(rect) - clampedRadius, clampedRadius);
        CGPathAddLineToPoint(path, NULL, CGRectGetMinX(rect), CGRectGetMinY(rect) + clampedRadius);
        CGPathAddArcToPoint(path, NULL, CGRectGetMinX(rect), CGRectGetMinY(rect), CGRectGetMinX(rect) + clampedRadius, CGRectGetMinY(rect), clampedRadius);
        CGPathCloseSubpath(path);
        CGContextAddPath(context, path);
        CGContextFillPath(context);
        CGPathRelease(path);
    }
    CGContextRestoreGState(context);
    return YES;
}

+ (BOOL)drawSensorBarForChromeInfo:(NSDictionary *)chromeInfo
                            profile:(NSDictionary *)profile
                            context:(CGContextRef)context
                              error:(NSError * _Nullable __autoreleasing *)error {
    NSString *sensorPath = [self sensorBarPathForChromeInfo:chromeInfo];
    if (sensorPath.length == 0) {
        return YES;
    }

    CGSize sensorSize = [self PDFPageSizeAtPath:sensorPath];
    if (sensorSize.width <= 0.0 || sensorSize.height <= 0.0) {
        return YES;
    }

    CGFloat screenX = [self numberValue:profile[@"screenX"]];
    CGFloat screenY = [self numberValue:profile[@"screenY"]];
    CGFloat screenWidth = [self numberValue:profile[@"screenWidth"]];
    if (screenWidth <= 0.0) {
        return YES;
    }

    CGRect rect = CGRectMake(screenX + ((screenWidth - sensorSize.width) / 2.0),
                             screenY,
                             sensorSize.width,
                             sensorSize.height);
    return [self drawPDFAtPath:sensorPath inRect:rect context:context error:error];
}

+ (NSString *)sensorBarPathForChromeInfo:(NSDictionary *)chromeInfo {
    NSDictionary *plist = chromeInfo[@"plist"];
    NSString *resourcesPath = [chromeInfo[@"profileResourcesPath"] isKindOfClass:[NSString class]] ? chromeInfo[@"profileResourcesPath"] : @"";
    NSString *sensorName = [plist[@"sensorBarImage"] isKindOfClass:[NSString class]] ? plist[@"sensorBarImage"] : @"";
    if (resourcesPath.length == 0 || sensorName.length == 0) {
        return @"";
    }

    NSString *sensorPath = [resourcesPath stringByAppendingPathComponent:[sensorName stringByAppendingPathExtension:@"pdf"]];
    return [[NSFileManager defaultManager] fileExistsAtPath:sensorPath] ? sensorPath : @"";
}

+ (NSString *)screenMaskPathForChromeInfo:(NSDictionary *)chromeInfo {
    NSDictionary *plist = chromeInfo[@"plist"];
    NSString *resourcesPath = [chromeInfo[@"profileResourcesPath"] isKindOfClass:[NSString class]] ? chromeInfo[@"profileResourcesPath"] : @"";
    NSString *maskName = [plist[@"framebufferMask"] isKindOfClass:[NSString class]] ? plist[@"framebufferMask"] : @"";
    if (resourcesPath.length == 0 || maskName.length == 0) {
        return @"";
    }

    NSString *maskPath = [resourcesPath stringByAppendingPathComponent:[maskName stringByAppendingPathExtension:@"pdf"]];
    return [[NSFileManager defaultManager] fileExistsAtPath:maskPath] ? maskPath : @"";
}

+ (CGFloat)framebufferMaskCornerRadiusForChromeInfo:(NSDictionary *)chromeInfo
                                   pointScreenWidth:(CGFloat)pointScreenWidth {
    NSString *maskPath = [self screenMaskPathForChromeInfo:chromeInfo];
    if (maskPath.length == 0 || pointScreenWidth <= 0.0) {
        return 0.0;
    }

    CGPDFDocumentRef document = CGPDFDocumentCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:maskPath]);
    if (document == NULL) {
        return 0.0;
    }
    CGPDFPageRef page = CGPDFDocumentGetPage(document, 1);
    if (page == NULL) {
        CGPDFDocumentRelease(document);
        return 0.0;
    }

    CGRect mediaBox = CGPDFPageGetBoxRect(page, kCGPDFMediaBox);
    NSInteger width = MAX((NSInteger)ceil(mediaBox.size.width), 1);
    NSInteger height = MAX((NSInteger)ceil(mediaBox.size.height), 1);
    if (width <= 1 || height <= 1 || width > 4096 || height > 4096) {
        CGPDFDocumentRelease(document);
        return 0.0;
    }

    size_t bytesPerRow = (size_t)width * 4;
    NSMutableData *pixels = [NSMutableData dataWithLength:(size_t)height * bytesPerRow];
    CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(pixels.mutableBytes,
                                                 (size_t)width,
                                                 (size_t)height,
                                                 8,
                                                 bytesPerRow,
                                                 colorSpace,
                                                 kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(colorSpace);
    if (context == NULL) {
        CGPDFDocumentRelease(document);
        return 0.0;
    }

    CGContextClearRect(context, CGRectMake(0, 0, width, height));
    CGContextSaveGState(context);
    CGContextTranslateCTM(context, 0, height);
    CGContextScaleCTM(context, (CGFloat)width / MAX(mediaBox.size.width, 1.0), -(CGFloat)height / MAX(mediaBox.size.height, 1.0));
    CGContextTranslateCTM(context, -mediaBox.origin.x, -mediaBox.origin.y);
    CGContextDrawPDFPage(context, page);
    CGContextRestoreGState(context);
    CGContextRelease(context);
    CGPDFDocumentRelease(document);

    const unsigned char *bytes = pixels.bytes;
    NSInteger topInset = -1;
    for (NSInteger x = 0; x < width; x++) {
        if (bytes[(size_t)x * 4 + 3] > 127) {
            topInset = x;
            break;
        }
    }
    NSInteger leftInset = -1;
    for (NSInteger y = 0; y < height; y++) {
        if (bytes[(size_t)y * bytesPerRow + 3] > 127) {
            leftInset = y;
            break;
        }
    }
    if (topInset < 0 || leftInset < 0) {
        return 0.0;
    }

    CGFloat maskRadius = MAX((CGFloat)topInset, (CGFloat)leftInset);
    CGFloat maskWidth = MAX(mediaBox.size.width, 1.0);
    return maskRadius * pointScreenWidth / maskWidth;
}

+ (nullable NSData *)PNGDataForPDFAtPath:(NSString *)path
                                    scale:(CGFloat)scale
                                    error:(NSError * _Nullable __autoreleasing *)error {
    if (path.length == 0) {
        return nil;
    }

    CGPDFDocumentRef document = CGPDFDocumentCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path]);
    if (document == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:7
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Unable to open PDF %@.", path.lastPathComponent],
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
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"PDF %@ did not contain a renderable page.", path.lastPathComponent],
            }];
        }
        return nil;
    }

    CGRect mediaBox = CGPDFPageGetBoxRect(page, kCGPDFMediaBox);
    CGFloat renderScale = MAX(scale, 1.0);
    NSInteger pixelWidth = MAX((NSInteger)ceil(mediaBox.size.width * renderScale), 1);
    NSInteger pixelHeight = MAX((NSInteger)ceil(mediaBox.size.height * renderScale), 1);

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
                NSLocalizedDescriptionKey: @"Unable to create a CoreGraphics bitmap context for PDF rendering.",
            }];
        }
        return nil;
    }

    CGContextClearRect(context, CGRectMake(0, 0, pixelWidth, pixelHeight));
    CGContextSaveGState(context);
    CGContextTranslateCTM(context, 0, pixelHeight);
    CGContextScaleCTM(context,
                      (CGFloat)pixelWidth / MAX(mediaBox.size.width, 1.0),
                      -((CGFloat)pixelHeight / MAX(mediaBox.size.height, 1.0)));
    CGContextTranslateCTM(context, -mediaBox.origin.x, -mediaBox.origin.y);
    CGContextDrawPDFPage(context, page);
    CGContextRestoreGState(context);
    CGPDFDocumentRelease(document);

    CGImageRef image = CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    if (image == NULL) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWChromeRendererErrorDomain
                                         code:10
                                     userInfo:@{
                NSLocalizedDescriptionKey: @"Unable to create a CGImage from the PDF bitmap.",
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
                NSLocalizedDescriptionKey: @"Unable to create a PNG encoder for PDF rendering.",
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
                NSLocalizedDescriptionKey: @"Unable to encode PDF render as PNG.",
            }];
        }
        return nil;
    }
    return data;
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

+ (BOOL)isPhoneProfile:(NSDictionary *)plist {
    NSString *chromeIdentifier = [plist[@"chromeIdentifier"] isKindOfClass:[NSString class]] ? plist[@"chromeIdentifier"] : @"";
    if ([chromeIdentifier containsString:@".phone"]) {
        return YES;
    }

    NSArray *families = [plist[@"supportedProductFamilyIDs"] isKindOfClass:[NSArray class]] ? plist[@"supportedProductFamilyIDs"] : @[];
    for (id family in families) {
        if ([family respondsToSelector:@selector(integerValue)] && [family integerValue] == 1) {
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
    CGContextClipToRect(context, rect);
    CGContextTranslateCTM(context, rect.origin.x, rect.origin.y + rect.size.height);
    CGContextScaleCTM(context, rect.size.width / MAX(mediaBox.size.width, 1.0), -rect.size.height / MAX(mediaBox.size.height, 1.0));
    CGContextTranslateCTM(context, -mediaBox.origin.x, -mediaBox.origin.y);
    CGContextDrawPDFPage(context, page);
    CGContextRestoreGState(context);
    CGPDFDocumentRelease(document);
    return YES;
}

+ (NSString *)compositeAssetPathForChromeInfo:(NSDictionary *)chromeInfo {
    NSDictionary *plist = chromeInfo[@"plist"];
    NSString *sensorName = [plist[@"sensorBarImage"] isKindOfClass:[NSString class]] ? plist[@"sensorBarImage"] : @"";
    if ([self shouldRenderPhoneChromeFromSlices:plist sensorName:sensorName]) {
        return @"";
    }

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

+ (BOOL)shouldRenderPhoneChromeFromSlices:(NSDictionary *)plist sensorName:(NSString *)sensorName {
    if (![self isPhoneProfile:plist]) {
        return NO;
    }
    if (sensorName.length > 0) {
        return YES;
    }

    NSString *chromeIdentifier = [plist[@"chromeIdentifier"] isKindOfClass:[NSString class]] ? plist[@"chromeIdentifier"] : @"";
    return [chromeIdentifier hasSuffix:@".phone11"] ||
        [chromeIdentifier hasSuffix:@".phone12"] ||
        [chromeIdentifier hasSuffix:@".phone13"];
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
