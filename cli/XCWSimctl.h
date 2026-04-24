#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface XCWSimctl : NSObject

- (nullable NSArray<NSDictionary *> *)listSimulatorsWithError:(NSError * _Nullable * _Nullable)error;
- (BOOL)bootSimulatorWithUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (BOOL)shutdownSimulatorWithUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (BOOL)toggleAppearanceForSimulatorUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (BOOL)openURL:(NSString *)urlString simulatorUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (BOOL)launchBundleID:(NSString *)bundleID simulatorUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (nullable NSData *)screenshotPNGForSimulatorUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (BOOL)eraseSimulatorWithUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (BOOL)installAppAtPath:(NSString *)appPath simulatorUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (BOOL)uninstallBundleID:(NSString *)bundleID simulatorUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (BOOL)setPasteboardText:(NSString *)text simulatorUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (nullable NSString *)pasteboardTextForSimulatorUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;
- (nullable NSArray<NSDictionary *> *)recentLogEntriesForSimulatorUDID:(NSString *)udid seconds:(NSTimeInterval)seconds error:(NSError * _Nullable * _Nullable)error;
- (nullable NSDictionary *)simulatorWithUDID:(NSString *)udid error:(NSError * _Nullable * _Nullable)error;

@end

NS_ASSUME_NONNULL_END
