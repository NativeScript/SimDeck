#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>

#import "XCWServerApplication.h"
#import "XCWSimctl.h"

static void XCWPrintUsage(void) {
    fprintf(stderr,
            "xcode-canvas-web\n"
            "\n"
            "Usage:\n"
            "  xcode-canvas-web serve [--port 4310] [--client-root /path/to/client/dist]\n"
            "  xcode-canvas-web list\n"
            "  xcode-canvas-web boot <udid>\n"
            "  xcode-canvas-web shutdown <udid>\n"
            "  xcode-canvas-web open-url <udid> <url>\n"
            "  xcode-canvas-web launch <udid> <bundle-id>\n");
}

static NSString *XCWDefaultClientRoot(void) {
    NSString *workingDirectory = [[NSFileManager defaultManager] currentDirectoryPath];
    return [[workingDirectory stringByAppendingPathComponent:@"client/dist"] stringByStandardizingPath];
}

static void XCWPrintJSON(id object) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:NSJSONWritingPrettyPrinted error:nil];
    NSString *string = [[NSString alloc] initWithData:data ?: [NSData data] encoding:NSUTF8StringEncoding] ?: @"{}";
    printf("%s\n", string.UTF8String);
}

int main(__unused int argc, __unused const char * argv[]) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];

        NSArray<NSString *> *arguments = [[NSProcessInfo processInfo] arguments];
        if (arguments.count < 2) {
            XCWPrintUsage();
            return 1;
        }

        NSString *command = arguments[1];
        XCWSimctl *simctl = [[XCWSimctl alloc] init];

        if ([command isEqualToString:@"serve"]) {
            uint16_t port = 4310;
            NSString *clientRoot = XCWDefaultClientRoot();

            for (NSUInteger index = 2; index < arguments.count; index++) {
                NSString *argument = arguments[index];
                if ([argument isEqualToString:@"--port"] && index + 1 < arguments.count) {
                    port = (uint16_t)[arguments[++index] integerValue];
                    continue;
                }
                if ([argument isEqualToString:@"--client-root"] && index + 1 < arguments.count) {
                    clientRoot = [arguments[++index] stringByStandardizingPath];
                    continue;
                }
            }

            XCWServerApplication *application = [[XCWServerApplication alloc] initWithPort:port clientRoot:clientRoot];
            NSError *error = nil;
            if (![application start:&error]) {
                fprintf(stderr, "Failed to start server: %s\n", error.localizedDescription.UTF8String);
                return 1;
            }

            printf("Xcode Canvas Web listening on http://127.0.0.1:%u\n", port);
            printf("Serving client from %s\n", clientRoot.UTF8String);
            dispatch_main();
        }

        if ([command isEqualToString:@"list"]) {
            NSError *error = nil;
            NSArray<NSDictionary *> *simulators = [simctl listSimulatorsWithError:&error];
            if (simulators == nil) {
                fprintf(stderr, "%s\n", error.localizedDescription.UTF8String);
                return 1;
            }
            XCWPrintJSON(@{ @"simulators": simulators });
            return 0;
        }

        if (([command isEqualToString:@"boot"] ||
             [command isEqualToString:@"shutdown"]) && arguments.count >= 3) {
            NSString *udid = arguments[2];
            NSError *error = nil;
            BOOL success = [command isEqualToString:@"boot"]
                ? [simctl bootSimulatorWithUDID:udid error:&error]
                : [simctl shutdownSimulatorWithUDID:udid error:&error];
            if (!success) {
                fprintf(stderr, "%s\n", error.localizedDescription.UTF8String);
                return 1;
            }
            XCWPrintJSON(@{ @"ok": @YES, @"udid": udid, @"action": command });
            return 0;
        }

        if ([command isEqualToString:@"open-url"] && arguments.count >= 4) {
            NSError *error = nil;
            if (![simctl openURL:arguments[3] simulatorUDID:arguments[2] error:&error]) {
                fprintf(stderr, "%s\n", error.localizedDescription.UTF8String);
                return 1;
            }
            XCWPrintJSON(@{ @"ok": @YES, @"udid": arguments[2], @"url": arguments[3] });
            return 0;
        }

        if ([command isEqualToString:@"launch"] && arguments.count >= 4) {
            NSError *error = nil;
            if (![simctl launchBundleID:arguments[3] simulatorUDID:arguments[2] error:&error]) {
                fprintf(stderr, "%s\n", error.localizedDescription.UTF8String);
                return 1;
            }
            XCWPrintJSON(@{ @"ok": @YES, @"udid": arguments[2], @"bundleId": arguments[3] });
            return 0;
        }

        XCWPrintUsage();
        return 1;
    }
}
