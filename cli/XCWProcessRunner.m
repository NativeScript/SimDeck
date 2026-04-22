#import "XCWProcessRunner.h"

static NSString * const XCWProcessRunnerErrorDomain = @"XcodeCanvasWeb.ProcessRunner";

@implementation XCWProcessResult

- (instancetype)initWithTerminationStatus:(int)terminationStatus
                               stdoutData:(NSData *)stdoutData
                               stderrData:(NSData *)stderrData {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _terminationStatus = terminationStatus;
    _stdoutData = [stdoutData copy];
    _stderrData = [stderrData copy];
    _stdoutString = [[NSString alloc] initWithData:_stdoutData encoding:NSUTF8StringEncoding] ?: @"";
    _stderrString = [[NSString alloc] initWithData:_stderrData encoding:NSUTF8StringEncoding] ?: @"";
    return self;
}

@end

@implementation XCWProcessRunner

+ (XCWProcessResult *)runLaunchPath:(NSString *)launchPath
                          arguments:(NSArray<NSString *> *)arguments
                          inputData:(NSData *)inputData
                              error:(NSError * _Nullable __autoreleasing *)error {
    NSTask *task = [[NSTask alloc] init];
    task.launchPath = launchPath;
    task.arguments = arguments;

    NSPipe *stdoutPipe = [NSPipe pipe];
    NSPipe *stderrPipe = [NSPipe pipe];
    task.standardOutput = stdoutPipe;
    task.standardError = stderrPipe;

    NSPipe *stdinPipe = nil;
    if (inputData != nil) {
        stdinPipe = [NSPipe pipe];
        task.standardInput = stdinPipe;
    }

    @try {
        [task launch];
    } @catch (NSException *exception) {
        if (error != NULL) {
            *error = [NSError errorWithDomain:XCWProcessRunnerErrorDomain
                                         code:1
                                     userInfo:@{
                NSLocalizedDescriptionKey: [NSString stringWithFormat:@"Failed to launch %@: %@", launchPath, exception.reason ?: @"unknown error"],
            }];
        }
        return nil;
    }

    if (inputData != nil) {
        [stdinPipe.fileHandleForWriting writeData:inputData];
        [stdinPipe.fileHandleForWriting closeFile];
    }

    __block NSData *stdoutData = [NSData data];
    __block NSData *stderrData = [NSData data];
    dispatch_group_t readGroup = dispatch_group_create();
    dispatch_queue_t readQueue = dispatch_get_global_queue(QOS_CLASS_UTILITY, 0);

    dispatch_group_async(readGroup, readQueue, ^{
        stdoutData = [stdoutPipe.fileHandleForReading readDataToEndOfFile] ?: [NSData data];
    });

    dispatch_group_async(readGroup, readQueue, ^{
        stderrData = [stderrPipe.fileHandleForReading readDataToEndOfFile] ?: [NSData data];
    });

    [task waitUntilExit];
    dispatch_group_wait(readGroup, DISPATCH_TIME_FOREVER);

    return [[XCWProcessResult alloc] initWithTerminationStatus:task.terminationStatus
                                                    stdoutData:stdoutData
                                                    stderrData:stderrData];
}

@end
