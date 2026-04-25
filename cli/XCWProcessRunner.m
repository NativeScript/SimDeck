#import "XCWProcessRunner.h"

#import <errno.h>
#import <fcntl.h>
#import <spawn.h>
#import <string.h>
#import <sys/wait.h>
#import <unistd.h>

extern char **environ;

static NSString * const XCWProcessRunnerErrorDomain = @"XcodeCanvasWeb.ProcessRunner";

static void XCWCloseFD(int *fd) {
    if (fd != NULL && *fd >= 0) {
        close(*fd);
        *fd = -1;
    }
}

static NSData *XCWReadAllAndCloseFD(int fd) {
    if (fd < 0) {
        return [NSData data];
    }

    NSMutableData *data = [NSMutableData data];
    uint8_t buffer[16384];
    for (;;) {
        ssize_t count = read(fd, buffer, sizeof(buffer));
        if (count > 0) {
            [data appendBytes:buffer length:(NSUInteger)count];
            continue;
        }
        if (count < 0 && errno == EINTR) {
            continue;
        }
        break;
    }
    close(fd);
    return data;
}

static void XCWWriteAllAndCloseFD(int fd, NSData *data) {
    if (fd < 0) {
        return;
    }

    const uint8_t *bytes = data.bytes;
    NSUInteger remaining = data.length;
    while (remaining > 0) {
        ssize_t written = write(fd, bytes, remaining);
        if (written > 0) {
            bytes += written;
            remaining -= (NSUInteger)written;
            continue;
        }
        if (written < 0 && errno == EINTR) {
            continue;
        }
        break;
    }
    close(fd);
}

static NSError *XCWProcessRunnerError(NSInteger code, NSString *description) {
    return [NSError errorWithDomain:XCWProcessRunnerErrorDomain
                               code:code
                           userInfo:@{ NSLocalizedDescriptionKey: description ?: @"Process failed." }];
}

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
    int stdoutPipe[2] = { -1, -1 };
    int stderrPipe[2] = { -1, -1 };
    int stdinPipe[2] = { -1, -1 };
    posix_spawn_file_actions_t fileActions;
    BOOL fileActionsInitialized = NO;
    char **argv = NULL;

    if (pipe(stdoutPipe) != 0 || pipe(stderrPipe) != 0 || (inputData != nil && pipe(stdinPipe) != 0)) {
        if (error != NULL) {
            *error = XCWProcessRunnerError(1, [NSString stringWithFormat:@"Failed to create process pipes: %s", strerror(errno)]);
        }
        XCWCloseFD(&stdoutPipe[0]);
        XCWCloseFD(&stdoutPipe[1]);
        XCWCloseFD(&stderrPipe[0]);
        XCWCloseFD(&stderrPipe[1]);
        XCWCloseFD(&stdinPipe[0]);
        XCWCloseFD(&stdinPipe[1]);
        return nil;
    }

    if (posix_spawn_file_actions_init(&fileActions) != 0) {
        if (error != NULL) {
            *error = XCWProcessRunnerError(2, [NSString stringWithFormat:@"Failed to initialize spawn actions: %s", strerror(errno)]);
        }
        XCWCloseFD(&stdoutPipe[0]);
        XCWCloseFD(&stdoutPipe[1]);
        XCWCloseFD(&stderrPipe[0]);
        XCWCloseFD(&stderrPipe[1]);
        XCWCloseFD(&stdinPipe[0]);
        XCWCloseFD(&stdinPipe[1]);
        return nil;
    }
    fileActionsInitialized = YES;

    posix_spawn_file_actions_adddup2(&fileActions, stdoutPipe[1], STDOUT_FILENO);
    posix_spawn_file_actions_adddup2(&fileActions, stderrPipe[1], STDERR_FILENO);
    if (inputData != nil) {
        posix_spawn_file_actions_adddup2(&fileActions, stdinPipe[0], STDIN_FILENO);
    } else {
        posix_spawn_file_actions_addopen(&fileActions, STDIN_FILENO, "/dev/null", O_RDONLY, 0);
    }
    posix_spawn_file_actions_addclose(&fileActions, stdoutPipe[0]);
    posix_spawn_file_actions_addclose(&fileActions, stdoutPipe[1]);
    posix_spawn_file_actions_addclose(&fileActions, stderrPipe[0]);
    posix_spawn_file_actions_addclose(&fileActions, stderrPipe[1]);
    if (inputData != nil) {
        posix_spawn_file_actions_addclose(&fileActions, stdinPipe[0]);
        posix_spawn_file_actions_addclose(&fileActions, stdinPipe[1]);
    }

    NSUInteger argc = arguments.count + 2;
    argv = calloc(argc, sizeof(char *));
    if (argv == NULL) {
        if (error != NULL) {
            *error = XCWProcessRunnerError(3, @"Failed to allocate process arguments.");
        }
        posix_spawn_file_actions_destroy(&fileActions);
        XCWCloseFD(&stdoutPipe[0]);
        XCWCloseFD(&stdoutPipe[1]);
        XCWCloseFD(&stderrPipe[0]);
        XCWCloseFD(&stderrPipe[1]);
        XCWCloseFD(&stdinPipe[0]);
        XCWCloseFD(&stdinPipe[1]);
        return nil;
    }
    argv[0] = (char *)launchPath.fileSystemRepresentation;
    for (NSUInteger index = 0; index < arguments.count; index += 1) {
        argv[index + 1] = (char *)arguments[index].UTF8String;
    }
    argv[argc - 1] = NULL;

    pid_t pid = 0;
    int spawnResult = posix_spawn(&pid, launchPath.fileSystemRepresentation, &fileActions, NULL, argv, environ);
    if (spawnResult != 0) {
        if (error != NULL) {
            *error = XCWProcessRunnerError(4, [NSString stringWithFormat:@"Failed to launch %@: %s", launchPath, strerror(spawnResult)]);
        }
        posix_spawn_file_actions_destroy(&fileActions);
        free(argv);
        XCWCloseFD(&stdoutPipe[0]);
        XCWCloseFD(&stdoutPipe[1]);
        XCWCloseFD(&stderrPipe[0]);
        XCWCloseFD(&stderrPipe[1]);
        XCWCloseFD(&stdinPipe[0]);
        XCWCloseFD(&stdinPipe[1]);
        return nil;
    }

    __block NSData *stdoutData = [NSData data];
    __block NSData *stderrData = [NSData data];
    dispatch_group_t readGroup = dispatch_group_create();
    dispatch_queue_t readQueue = dispatch_get_global_queue(QOS_CLASS_UTILITY, 0);
    int stdoutReadFD = stdoutPipe[0];
    int stderrReadFD = stderrPipe[0];
    stdoutPipe[0] = -1;
    stderrPipe[0] = -1;

    dispatch_group_async(readGroup, readQueue, ^{
        stdoutData = XCWReadAllAndCloseFD(stdoutReadFD);
    });

    dispatch_group_async(readGroup, readQueue, ^{
        stderrData = XCWReadAllAndCloseFD(stderrReadFD);
    });

    XCWCloseFD(&stdoutPipe[1]);
    XCWCloseFD(&stderrPipe[1]);
    if (inputData != nil) {
        XCWCloseFD(&stdinPipe[0]);
        int stdinWriteFD = stdinPipe[1];
        stdinPipe[1] = -1;
        dispatch_group_async(readGroup, readQueue, ^{
            XCWWriteAllAndCloseFD(stdinWriteFD, inputData);
        });
    }

    int waitStatus = 0;
    pid_t waitResult = -1;
    do {
        waitResult = waitpid(pid, &waitStatus, 0);
    } while (waitResult < 0 && errno == EINTR);
    dispatch_group_wait(readGroup, DISPATCH_TIME_FOREVER);
    int terminationStatus = 1;
    if (waitResult < 0) {
        if (error != NULL) {
            *error = XCWProcessRunnerError(5, [NSString stringWithFormat:@"Failed to wait for %@: %s", launchPath, strerror(errno)]);
        }
    } else if (WIFEXITED(waitStatus)) {
        terminationStatus = WEXITSTATUS(waitStatus);
    } else if (WIFSIGNALED(waitStatus)) {
        terminationStatus = 128 + WTERMSIG(waitStatus);
    }

    if (fileActionsInitialized) {
        posix_spawn_file_actions_destroy(&fileActions);
    }
    free(argv);

    return [[XCWProcessResult alloc] initWithTerminationStatus:terminationStatus
                                                    stdoutData:stdoutData
                                                    stderrData:stderrData];
}

@end
