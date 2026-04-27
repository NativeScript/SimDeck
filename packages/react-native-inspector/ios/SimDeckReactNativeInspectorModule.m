#import "SimDeckReactNativeInspectorModule.h"

#import <UIKit/UIKit.h>

@implementation SimDeckReactNativeInspectorModule

RCT_EXPORT_MODULE(SimDeckReactNativeInspector)

RCT_REMAP_METHOD(getInfo,
                 getInfoWithResolver:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    NSBundle *bundle = NSBundle.mainBundle;
    UIScreen *screen = UIScreen.mainScreen;
    CGRect bounds = screen.bounds;
    NSString *bundleName = [bundle objectForInfoDictionaryKey:@"CFBundleName"] ?: @"";
    resolve(@{
      @"processIdentifier": @(NSProcessInfo.processInfo.processIdentifier),
      @"bundleIdentifier": bundle.bundleIdentifier ?: @"",
      @"bundleName": bundleName,
      @"displayScale": @(screen.scale),
      @"screenBounds": @{
        @"x": @(bounds.origin.x),
        @"y": @(bounds.origin.y),
        @"width": @(bounds.size.width),
        @"height": @(bounds.size.height)
      }
    });
  });
}

@end
