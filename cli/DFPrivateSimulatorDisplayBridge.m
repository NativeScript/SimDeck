#import "DFPrivateSimulatorDisplayBridge.h"

#import <CoreVideo/CoreVideo.h>
#import <dlfcn.h>
#import <mach/mach.h>
#import <mach/mach_time.h>
#import <mach-o/dyld.h>
#import <mach-o/loader.h>
#import <mach-o/nlist.h>
#import <math.h>
#import <objc/message.h>
#import <objc/runtime.h>
#import <stdarg.h>

// PurpleWorkspacePort mach message IDs / GSEvent type constants.
// Reverse-engineered from Simulator.app ARM64 (Xcode 26.2) — see idb's
// PrivateHeaders/SimulatorApp/GSEvent.h for the complete wire format.
#define DFGSEventMachMessageID 0x7B
#define DFGSEventHostFlag 0x20000
#define DFGSEventTypeDeviceOrientationChanged 50

static NSString * const DFPrivateSimulatorErrorDomain = @"SimDeck.PrivateSimulator";
static NSString * const DFSimulatorKitPath = @"/Applications/Xcode.app/Contents/Developer/Library/PrivateFrameworks/SimulatorKit.framework/SimulatorKit";
static NSString * const DFCoreSimulatorPath = @"/Library/Developer/PrivateFrameworks/CoreSimulator.framework/CoreSimulator";
static NSString * const DFPrivateSimulatorLogPath = @"/tmp/simdeck-private-bridge.log";
static const void *DFPrivateSimulatorCallbackQueueKey = &DFPrivateSimulatorCallbackQueueKey;
static const void *DFDigitizerDelegateAssociationKey = &DFDigitizerDelegateAssociationKey;
static const void *DFDigitizerWakeDelegateAssociationKey = &DFDigitizerWakeDelegateAssociationKey;

typedef struct IndigoHIDMessageStruct IndigoHIDMessage;
typedef uint32_t IndigoHIDEdge;

typedef IndigoHIDMessage *(*DFIndigoHIDMessageForMouseNSEventFn)(CGPoint *location, CGPoint *windowLocation, uint32_t target, NSEventType type, NSSize displaySize, IndigoHIDEdge edge);
typedef IndigoHIDMessage *(*DFIndigoHIDMessageForKeyboardArbitraryFn)(int keyCode, int op);
typedef IndigoHIDMessage *(*DFIndigoHIDMessageForKeyboardNSEventFn)(NSEvent *event);
typedef IndigoHIDMessage *(*DFIndigoHIDMessageForButtonFn)(uint32_t buttonCode, uint32_t operation, uint32_t target);
typedef IndigoHIDMessage *(*DFIndigoHIDMessageForHIDArbitraryFn)(uint32_t target, uint32_t page, uint32_t usage, uint32_t operation);

#pragma pack(push, 4)
typedef struct {
    uint32_t msgh_bits;
    uint32_t msgh_size;
    uint32_t msgh_remote_port;
    uint32_t msgh_local_port;
    uint32_t msgh_voucher_port;
    int32_t msgh_id;
} DFMachMessageHeader;

typedef struct {
    uint32_t field1;
    uint32_t field2;
    uint32_t field3;
    double xRatio;
    double yRatio;
    double field6;
    double field7;
    double field8;
    uint32_t field9;
    uint32_t field10;
    uint32_t field11;
    uint32_t field12;
    uint32_t field13;
    double field14;
    double field15;
    double field16;
    double field17;
    double field18;
} DFIndigoTouch;

typedef union {
    DFIndigoTouch touch;
} DFIndigoEvent;

typedef struct {
    uint32_t field1;
    uint64_t timestamp;
    uint32_t field3;
    DFIndigoEvent event;
} DFIndigoPayload;

typedef struct {
    DFMachMessageHeader header;
    uint32_t innerSize;
    uint8_t eventType;
    uint8_t reserved[3];
    DFIndigoPayload payload;
} DFIndigoMessage;
#pragma pack(pop)

static const uint32_t DFIndigoTouchTarget = 0x32;
static const uint8_t DFIndigoEventTypeTouch = 0x02;
static const uint32_t DFIndigoTouchEventKind = 0x0b;
static const int DFKeyboardDirectionDown = 1;
static const int DFKeyboardDirectionUp = 2;
static const uint32_t DFButtonDirectionDown = 1;
static const uint32_t DFButtonDirectionUp = 2;
static const uint32_t DFConsumerControlUsagePage = 0x0c;
static const uint32_t DFHomeConsumerUsage = 0x65;
// Apple Simulator sends Indigo button code 0x191 for Home; 1 is Lock.
static const uint32_t DFHomeButtonCode = 0x191;
static const NSUInteger DFKeyboardModifierShift = 1 << 0;
static const NSUInteger DFKeyboardModifierControl = 1 << 1;
static const NSUInteger DFKeyboardModifierOption = 1 << 2;
static const NSUInteger DFKeyboardModifierCommand = 1 << 3;
static const NSUInteger DFKeyboardModifierCapsLock = 1 << 4;

typedef struct {
    __unsafe_unretained id unit;
    double value;
} DFUnitAngleMeasurement;

static DFIndigoMessage *DFCreateIndigoMultiTouchMessage(CGPoint normalizedPoint1, CGPoint normalizedPoint2, NSSize displaySize, BOOL touchDown, NSError **error);

typedef NS_ENUM(NSInteger, DFPrivateSimulatorErrorCode) {
    DFPrivateSimulatorErrorCodeFrameworkLoadFailed = 1,
    DFPrivateSimulatorErrorCodeServiceContextFailed = 2,
    DFPrivateSimulatorErrorCodeDeviceLookupFailed = 3,
    DFPrivateSimulatorErrorCodeDisplayAttachFailed = 4,
    DFPrivateSimulatorErrorCodeTouchDispatchFailed = 5,
};

static NSError * DFMakeError(DFPrivateSimulatorErrorCode code, NSString *description) {
    return [NSError errorWithDomain:DFPrivateSimulatorErrorDomain code:code userInfo:@{
        NSLocalizedDescriptionKey: description,
    }];
}

static void DFLog(NSString *format, ...) {
    va_list arguments;
    va_start(arguments, format);
    NSString *message = [[NSString alloc] initWithFormat:format arguments:arguments];
    va_end(arguments);
    NSString *line = [NSString stringWithFormat:@"%@ [XCW][PrivateSim] %@\n", [NSDate date], message];
    NSFileManager *fileManager = [NSFileManager defaultManager];
    if (![fileManager fileExistsAtPath:DFPrivateSimulatorLogPath]) {
        [line writeToFile:DFPrivateSimulatorLogPath atomically:YES encoding:NSUTF8StringEncoding error:nil];
        return;
    }

    NSFileHandle *handle = [NSFileHandle fileHandleForWritingAtPath:DFPrivateSimulatorLogPath];
    if (handle == nil) {
        [line writeToFile:DFPrivateSimulatorLogPath atomically:YES encoding:NSUTF8StringEncoding error:nil];
        return;
    }

    @try {
        [handle seekToEndOfFile];
        [handle writeData:[line dataUsingEncoding:NSUTF8StringEncoding]];
    } @catch (__unused NSException *exception) {
    }
    [handle closeFile];
}

#pragma mark - SimulatorKit Swift symbol resolver
//
// We call into SimulatorKit's private Swift API by dlsym'ing mangled symbol
// names. The leading bytes of a Swift mangling encode `module + class + member
// name` and are stable across Xcode releases; the trailing bytes encode the
// type signature, which Apple periodically reshapes (e.g. Xcode 26.4 retyped
// `SimDeviceScreenAdapter.screens` from `[UInt32: SimScreen]` (ObjC) to
// `[UInt32: SimDeviceScreen]` (Swift), invalidating the old mangled tail).
//
// Instead of hardcoding the full mangled name, we walk SimulatorKit's
// `LC_SYMTAB` once and find the first symbol whose name matches a stable
// (prefix, suffix) pair — typically `…member_name` + `vg`/`vs`/`F`. Resolved
// pointers are cached per (prefix, suffix). When a lookup fails we log loudly
// so the next breaking change shows up as a clear "missing: X" instead of a
// silent timeout downstream.

static const struct mach_header_64 *gSimulatorKitImage = NULL;
static intptr_t gSimulatorKitSlide = 0;

static void DFLocateSimulatorKitImageOnce(void) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        Class probe = NSClassFromString(@"SimulatorKit.SimDeviceScreenAdapter");
        if (probe == Nil) {
            probe = NSClassFromString(@"SimulatorKit.SimDeviceScreen");
        }
        if (probe == Nil) {
            DFLog(@"warning: SimulatorKit not yet loaded — cannot locate Mach-O image for symbol resolution");
            return;
        }
        Dl_info info = {0};
        if (dladdr((__bridge const void *)probe, &info) == 0 || info.dli_fbase == NULL) {
            DFLog(@"warning: dladdr failed on SimulatorKit class — symbol prefix lookups disabled");
            return;
        }
        uint32_t count = _dyld_image_count();
        for (uint32_t i = 0; i < count; i++) {
            const struct mach_header *header = _dyld_get_image_header(i);
            if ((const void *)header != info.dli_fbase) {
                continue;
            }
            if (header->magic != MH_MAGIC_64 && header->magic != MH_CIGAM_64) {
                DFLog(@"warning: SimulatorKit Mach-O is not 64-bit (magic=0x%x)", header->magic);
                return;
            }
            gSimulatorKitImage = (const struct mach_header_64 *)header;
            gSimulatorKitSlide = _dyld_get_image_vmaddr_slide(i);
            return;
        }
        DFLog(@"warning: SimulatorKit image not found in dyld image list");
    });
}

// Read a uleb128-encoded integer, advancing *p.
static uint64_t DFReadULEB(const uint8_t **p, const uint8_t *end) {
    uint64_t result = 0;
    int shift = 0;
    while (*p < end) {
        uint8_t byte = *(*p)++;
        result |= ((uint64_t)(byte & 0x7f)) << shift;
        if ((byte & 0x80) == 0) break;
        shift += 7;
        if (shift >= 64) break;
    }
    return result;
}

// DFS the dyld exports trie, returning the first terminal whose full mangled
// name starts with `prefix` and ends with `suffix`. Trie pruning means we
// visit only paths consistent with the prefix; effectively O(prefix length +
// matching subtree size).
typedef struct {
    const uint8_t *trie;
    const uint8_t *trieEnd;
    const char *prefix;
    size_t prefixLen;
    const char *suffix;
    size_t suffixLen;
    uint64_t address;     // resolved symbol address (image-relative); 0 if none
    BOOL found;
    char nameBuf[1024];
} DFTrieContext;

static void DFTrieDescend(DFTrieContext *ctx, const uint8_t *node, size_t nameLen) {
    if (ctx->found || node == NULL || node >= ctx->trieEnd) return;

    const uint8_t *p = node;
    uint64_t termSize = DFReadULEB(&p, ctx->trieEnd);

    if (termSize > 0) {
        // Path so far == terminal symbol's full name.
        if (nameLen >= ctx->prefixLen + ctx->suffixLen &&
            memcmp(ctx->nameBuf, ctx->prefix, ctx->prefixLen) == 0 &&
            (ctx->suffixLen == 0 ||
             memcmp(ctx->nameBuf + nameLen - ctx->suffixLen, ctx->suffix, ctx->suffixLen) == 0)) {
            const uint8_t *info = p;
            uint64_t flags = DFReadULEB(&info, ctx->trieEnd);
            uint64_t address = DFReadULEB(&info, ctx->trieEnd);
            // Skip re-exports (REEXPORT=0x08) and resolver stubs (STUB_AND_RESOLVER=0x10).
            if ((flags & 0x08) == 0 && (flags & 0x10) == 0) {
                ctx->address = address;
                ctx->found = YES;
                return;
            }
        }
        p += termSize;
    }

    if (p >= ctx->trieEnd) return;
    uint8_t childCount = *p++;

    for (uint8_t i = 0; i < childCount; i++) {
        if (p >= ctx->trieEnd) return;
        const char *edgeLabel = (const char *)p;
        size_t labelLen = strnlen(edgeLabel, (size_t)(ctx->trieEnd - p));
        p += labelLen + 1;
        if (p > ctx->trieEnd) return;
        uint64_t childOffset = DFReadULEB(&p, ctx->trieEnd);

        size_t newLen = nameLen + labelLen;
        if (newLen >= sizeof(ctx->nameBuf)) continue;

        // Prune: the appended label must keep us on a path consistent with prefix.
        size_t cmpLen = newLen < ctx->prefixLen ? newLen : ctx->prefixLen;
        size_t cmpStart = nameLen < ctx->prefixLen ? nameLen : ctx->prefixLen;
        if (cmpLen > cmpStart) {
            if (memcmp(edgeLabel, ctx->prefix + cmpStart, cmpLen - cmpStart) != 0) {
                continue;
            }
        }

        memcpy(ctx->nameBuf + nameLen, edgeLabel, labelLen);
        DFTrieDescend(ctx, ctx->trie + childOffset, newLen);
        if (ctx->found) return;
    }
}

// Resolves a Swift symbol exported from SimulatorKit by stable mangled prefix
// + suffix, walking the dyld exports trie (where Swift exports live in modern
// Mach-O builds — LC_SYMTAB only carries local/debug symbols).
static void *DFFindSwiftSymbol(const char *prefix, const char *suffix) {
    DFLocateSimulatorKitImageOnce();
    if (gSimulatorKitImage == NULL || prefix == NULL || prefix[0] == '\0') {
        return NULL;
    }

    const struct linkedit_data_command *exportsTrie = NULL;
    const struct segment_command_64 *linkedit = NULL;
    uint64_t dyldInfoExportOff = 0;
    uint64_t dyldInfoExportSize = 0;

    const struct load_command *lc = (const struct load_command *)((const char *)gSimulatorKitImage + sizeof(struct mach_header_64));
    for (uint32_t i = 0; i < gSimulatorKitImage->ncmds; i++) {
        switch (lc->cmd) {
            case LC_DYLD_EXPORTS_TRIE:
                exportsTrie = (const struct linkedit_data_command *)lc;
                break;
            case LC_DYLD_INFO:
            case LC_DYLD_INFO_ONLY: {
                const struct dyld_info_command *info = (const struct dyld_info_command *)lc;
                dyldInfoExportOff = info->export_off;
                dyldInfoExportSize = info->export_size;
                break;
            }
            case LC_SEGMENT_64: {
                const struct segment_command_64 *seg = (const struct segment_command_64 *)lc;
                if (strcmp(seg->segname, "__LINKEDIT") == 0) {
                    linkedit = seg;
                }
                break;
            }
        }
        lc = (const struct load_command *)((const char *)lc + lc->cmdsize);
    }

    if (linkedit == NULL) {
        DFLog(@"warning: SimulatorKit Mach-O has no __LINKEDIT segment");
        return NULL;
    }

    uint64_t trieFileOff = 0;
    uint64_t trieSize = 0;
    if (exportsTrie != NULL && exportsTrie->datasize > 0) {
        trieFileOff = exportsTrie->dataoff;
        trieSize = exportsTrie->datasize;
    } else if (dyldInfoExportSize > 0) {
        trieFileOff = dyldInfoExportOff;
        trieSize = dyldInfoExportSize;
    } else {
        DFLog(@"warning: SimulatorKit has no LC_DYLD_EXPORTS_TRIE / LC_DYLD_INFO export trie");
        return NULL;
    }

    uintptr_t linkeditMapped = (uintptr_t)linkedit->vmaddr + (uintptr_t)gSimulatorKitSlide - (uintptr_t)linkedit->fileoff;
    const uint8_t *trie = (const uint8_t *)(linkeditMapped + (uintptr_t)trieFileOff);

    // The exports trie stores symbol names with the leading `_` that dlsym
    // strips by convention. Prepend it to the search prefix so the trie's
    // edge labels (which start with `_`) match our prefix from the root.
    char prefixedBuf[1024];
    int written = snprintf(prefixedBuf, sizeof(prefixedBuf), "_%s", prefix);
    if (written < 0 || (size_t)written >= sizeof(prefixedBuf)) {
        return NULL;
    }

    DFTrieContext ctx = {0};
    ctx.trie = trie;
    ctx.trieEnd = trie + trieSize;
    ctx.prefix = prefixedBuf;
    ctx.prefixLen = (size_t)written;
    ctx.suffix = suffix ?: "";
    ctx.suffixLen = suffix ? strlen(suffix) : 0;

    DFTrieDescend(&ctx, trie, 0);

    if (!ctx.found) {
        return NULL;
    }
    return (void *)((uintptr_t)gSimulatorKitImage + (uintptr_t)ctx.address);
}

// Cache resolved function pointers per (prefix, suffix). Logs once per missing
// symbol so the first run after a breaking Xcode update names exactly what
// went away.
static void *DFResolveSwiftSymbol(const char *prefix, const char *suffix, const char *role) {
    static NSLock *lock;
    static NSMutableDictionary<NSString *, NSValue *> *cache;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        lock = [NSLock new];
        cache = [NSMutableDictionary new];
    });

    NSString *key = [NSString stringWithFormat:@"%s\x01%s", prefix ?: "", suffix ?: ""];
    [lock lock];
    NSValue *cached = cache[key];
    [lock unlock];
    if (cached != nil) {
        return [cached pointerValue];
    }

    void *fn = DFFindSwiftSymbol(prefix, suffix);
    if (fn == NULL) {
        DFLog(@"warning: SimulatorKit symbol missing — role='%s' prefix='%s' suffix='%s'. Likely renamed in this Xcode; private bridge will skip this hook.",
              role ?: "?", prefix ?: "", suffix ?: "");
    }

    [lock lock];
    cache[key] = [NSValue valueWithPointer:fn];
    [lock unlock];
    return fn;
}

static id DFDigitizerDelegateGetter(id self, SEL _cmd) {
    (void)_cmd;
    return objc_getAssociatedObject(self, DFDigitizerDelegateAssociationKey);
}

static id DFDigitizerWakeDelegateGetter(id self, SEL _cmd) {
    (void)_cmd;
    return objc_getAssociatedObject(self, DFDigitizerWakeDelegateAssociationKey);
}

static Class __attribute__((unused)) DFEnsureDigitizerProxyClass(Class baseClass) {
    if (baseClass == Nil) {
        return Nil;
    }

    NSString *subclassName = [NSString stringWithFormat:@"%@_XCWProxy", NSStringFromClass(baseClass)];
    Class subclass = NSClassFromString(subclassName);
    if (subclass != Nil) {
        return subclass;
    }

    subclass = objc_allocateClassPair(baseClass, subclassName.UTF8String, 0);
    if (subclass == Nil) {
        return baseClass;
    }

    class_addMethod(subclass, sel_registerName("delegate"), (IMP)DFDigitizerDelegateGetter, "@@:");
    class_addMethod(subclass, sel_registerName("wakeOnTouchDelegate"), (IMP)DFDigitizerWakeDelegateGetter, "@@:");
    objc_registerClassPair(subclass);
    return subclass;
}

static id DFSendObject(id target, const char *selectorName) {
    return ((id(*)(id, SEL))objc_msgSend)(target, sel_registerName(selectorName));
}

static NSString *DFOptionalStringFromObjectSelector(id target, const char *selectorName) {
    if (target == nil || selectorName == NULL) {
        return nil;
    }

    id value = DFSendObject(target, selectorName);
    return [value isKindOfClass:[NSString class]] ? value : nil;
}

static NSString *DFTrimmedString(NSString *value) {
    NSString *trimmed = [value stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
    return trimmed.length > 0 ? trimmed : nil;
}

static id DFAllocInitRect(Class cls, NSRect rect) {
    id instance = ((id(*)(id, SEL))objc_msgSend)(cls, sel_registerName("alloc"));
    return ((id(*)(id, SEL, NSRect))objc_msgSend)(instance, sel_registerName("initWithFrame:"), rect);
}

// Logs missing dlsym lookups exactly once per (symbol) so we can spot Apple's
// renames without spamming.
static void DFLogMissingSymbolOnce(const char *symbolName) {
    if (symbolName == NULL || symbolName[0] == '\0') return;
    static NSLock *lock;
    static NSMutableSet<NSString *> *seen;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        lock = [NSLock new];
        seen = [NSMutableSet new];
    });
    NSString *key = [NSString stringWithUTF8String:symbolName];
    [lock lock];
    BOOL isNew = ![seen containsObject:key];
    if (isNew) [seen addObject:key];
    [lock unlock];
    if (isNew) {
        DFLog(@"warning: dlsym('%s') returned NULL — symbol likely renamed in this Xcode/macOS.", symbolName);
    }
}

static id DFCallSwiftSelfGetterByFunction(id selfObject, void *function) {
    if (selfObject == nil || function == NULL) {
        return nil;
    }

    id result = nil;
    __asm__ volatile(
        "mov x20, %1\n"
        "blr %2\n"
        "mov %0, x0\n"
        : "=r" (result)
        : "r" (selfObject), "r" (function)
        : "x0", "x20", "x30", "memory"
    );
    return result;
}

static id __attribute__((unused)) DFCallSwiftSelfGetter(id selfObject, const char *symbolName) {
    if (selfObject == nil) return nil;
    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        DFLogMissingSymbolOnce(symbolName);
        return nil;
    }
    return DFCallSwiftSelfGetterByFunction(selfObject, function);
}

static id DFCallSwiftSelfGetterByPattern(id selfObject, const char *prefix, const char *suffix, const char *role) {
    if (selfObject == nil) return nil;
    void *function = DFResolveSwiftSymbol(prefix, suffix, role);
    return DFCallSwiftSelfGetterByFunction(selfObject, function);
}

static NSDictionary<NSNumber *, id> * DFReadAdapterScreens(id adapter) {
    // The full mangled tail of `SimDeviceScreenAdapter.screens.getter` drifts
    // across Xcode releases (Xcode 26.4 retyped it from
    // `[UInt32: SimScreen]` (ObjC) to `[UInt32: SimDeviceScreen]` (Swift)).
    // Resolve by stable prefix instead. If the values are now SimDeviceScreen
    // wrappers, unwrap each via `.screen` so callers keep talking to a
    // SimScreen-shaped object.
    id screens = DFCallSwiftSelfGetterByPattern(
        adapter,
        "$s12SimulatorKit22SimDeviceScreenAdapterC7screens",
        "vg",
        "SimDeviceScreenAdapter.screens.getter"
    );
    if (![screens isKindOfClass:[NSDictionary class]]) {
        return @{};
    }

    NSMutableDictionary<NSNumber *, id> *unwrapped = [NSMutableDictionary dictionaryWithCapacity:[(NSDictionary *)screens count]];
    SEL screenSelector = sel_registerName("screen");
    [(NSDictionary *)screens enumerateKeysAndObjectsUsingBlock:^(id key, id value, BOOL *stop) {
        (void)stop;
        if (![key isKindOfClass:[NSNumber class]] || value == nil) {
            return;
        }
        id rawScreen = value;
        if ([value respondsToSelector:screenSelector]) {
            id underlying = ((id(*)(id, SEL))objc_msgSend)(value, screenSelector);
            if (underlying != nil) {
                rawScreen = underlying;
            }
        }
        unwrapped[key] = rawScreen;
    }];
    return unwrapped;
}

static uint32_t __attribute__((unused)) DFCallSwiftSelfGetterU32(id selfObject, const char *symbolName) {
    if (selfObject == nil) {
        return 0;
    }

    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        return 0;
    }

    uint32_t result = 0;
    __asm__ volatile(
        "mov x20, %1\n"
        "blr %2\n"
        "mov %w0, w0\n"
        : "=r" (result)
        : "r" (selfObject), "r" (function)
        : "x0", "x20", "x30", "memory"
    );
    return result;
}

static uintptr_t __attribute__((unused)) DFCallSwiftUWordGetter(const char *symbolName) {
    if (symbolName == NULL) {
        return 0;
    }

    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        return 0;
    }

    uintptr_t result = 0;
    __asm__ volatile(
        "blr %1\n"
        "mov %0, x0\n"
        : "=r" (result)
        : "r" (function)
        : "x0", "x30", "memory"
    );
    return result;
}

static BOOL __attribute__((unused)) DFCallSwiftVoidMethodWithSelfAndTwoArgs(id selfObject, id firstArgument, const void *secondArgument, const char *symbolName) {
    if (selfObject == nil || firstArgument == nil || secondArgument == nil) {
        return NO;
    }

    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        return NO;
    }

    __asm__ volatile(
        "mov x20, %0\n"
        "mov x0, %1\n"
        "mov x1, %2\n"
        "blr %3\n"
        :
        : "r" (selfObject), "r" (firstArgument), "r" (secondArgument), "r" (function)
        : "x0", "x1", "x20", "x30", "memory"
    );
    return YES;
}

static BOOL DFCallSwiftVoidMethodWithSelfAndObjectByFunction(id selfObject, id firstArgument, void *function) {
    if (selfObject == nil || firstArgument == nil || function == NULL) {
        return NO;
    }

    __asm__ volatile(
        "mov x20, %0\n"
        "mov x0, %1\n"
        "blr %2\n"
        :
        : "r" (selfObject), "r" (firstArgument), "r" (function)
        : "x0", "x20", "x30", "memory"
    );
    return YES;
}

static BOOL __attribute__((unused)) DFCallSwiftVoidMethodWithSelfAndObject(id selfObject, id firstArgument, const char *symbolName) {
    if (selfObject == nil || firstArgument == nil) return NO;
    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        DFLogMissingSymbolOnce(symbolName);
        return NO;
    }
    return DFCallSwiftVoidMethodWithSelfAndObjectByFunction(selfObject, firstArgument, function);
}

static BOOL DFCallSwiftVoidMethodWithSelfAndObjectByPattern(id selfObject, id firstArgument, const char *prefix, const char *suffix, const char *role) {
    if (selfObject == nil || firstArgument == nil) return NO;
    void *function = DFResolveSwiftSymbol(prefix, suffix, role);
    return DFCallSwiftVoidMethodWithSelfAndObjectByFunction(selfObject, firstArgument, function);
}

static BOOL __attribute__((unused)) DFCallSwiftVoidMethodWithSelfAndUWordAndBool(id selfObject, uintptr_t firstArgument, BOOL secondArgument, const char *symbolName) {
    if (selfObject == nil) {
        return NO;
    }

    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        return NO;
    }

    uintptr_t enabledValue = secondArgument ? 1 : 0;
    __asm__ volatile(
        "mov x20, %0\n"
        "mov x0, %1\n"
        "mov x1, %2\n"
        "blr %3\n"
        :
        : "r" (selfObject), "r" (firstArgument), "r" (enabledValue), "r" (function)
        : "x0", "x1", "x20", "x30", "memory"
    );
    return YES;
}

static BOOL __attribute__((unused)) DFCallSwiftThrowingMethodWithSelfAndObjectAndUWord(id selfObject, id firstArgument, uintptr_t secondArgument, const char *symbolName, NSError **error) {
    if (selfObject == nil || firstArgument == nil) {
        return NO;
    }

    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        return NO;
    }

    uintptr_t thrownBits = 0;
    __asm__ volatile(
        "mov x20, %1\n"
        "mov x0, %2\n"
        "mov x1, %3\n"
        "mov x21, xzr\n"
        "blr %4\n"
        "mov %0, x21\n"
        : "=r" (thrownBits)
        : "r" (selfObject), "r" (firstArgument), "r" (secondArgument), "r" (function)
        : "x0", "x1", "x20", "x21", "x30", "memory"
    );

    if (thrownBits >= 0x1000) {
        DFLog(@"SimulatorKit connect(screen:inputs:) returned x21 = 0x%llx", (unsigned long long)thrownBits);
    }

    if (error != NULL) {
        *error = nil;
    }

    return YES;
}

static void DFSpinRunLoop(NSTimeInterval duration) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:duration];
    while ([deadline timeIntervalSinceNow] > 0) {
        @autoreleasepool {
            NSDate *slice = [NSDate dateWithTimeIntervalSinceNow:0.05];
            if ([NSThread isMainThread]) {
                [[NSRunLoop currentRunLoop] runUntilDate:slice];
            } else {
                [NSThread sleepForTimeInterval:0.05];
            }
        }
    }
}

static void DFRunOnMainSync(dispatch_block_t block) {
    if (block == nil) {
        return;
    }

    if ([NSThread isMainThread]) {
        block();
        return;
    }

    dispatch_sync(dispatch_get_main_queue(), block);
}

static void DFRunOnMainAsync(dispatch_block_t block) {
    if (block == nil) {
        return;
    }

    if ([NSThread isMainThread]) {
        block();
        return;
    }

    dispatch_async(dispatch_get_main_queue(), block);
}

static CVPixelBufferRef DFCreatePixelBufferFromSurface(IOSurfaceRef surface) {
    if (surface == nil) {
        return nil;
    }

    CVPixelBufferRef pixelBuffer = nil;
    NSDictionary *attributes = @{
        (id)kCVPixelBufferIOSurfacePropertiesKey: @{},
        (id)kCVPixelBufferMetalCompatibilityKey: @YES,
        (id)kCVPixelBufferCGImageCompatibilityKey: @YES,
        (id)kCVPixelBufferCGBitmapContextCompatibilityKey: @YES,
    };
    CVReturn status = CVPixelBufferCreateWithIOSurface(kCFAllocatorDefault, surface, (__bridge CFDictionaryRef)attributes, &pixelBuffer);
    if (status != kCVReturnSuccess) {
        DFLog(@"CVPixelBufferCreateWithIOSurface failed: %d", status);
    }
    return status == kCVReturnSuccess ? pixelBuffer : nil;
}

static Ivar DFGetIvar(id object, const char *name) {
    if (object == nil || name == NULL) {
        return NULL;
    }
    return class_getInstanceVariable([object class], name);
}

static id DFGetObjectIvar(id object, const char *name) {
    Ivar ivar = DFGetIvar(object, name);
    return ivar != NULL ? object_getIvar(object, ivar) : nil;
}

static void DFSetBoolIvar(id object, const char *name, BOOL value) {
    Ivar ivar = DFGetIvar(object, name);
    if (ivar == NULL) {
        return;
    }

    uint8_t *bytes = (uint8_t *)(__bridge void *)object;
    bytes[ivar_getOffset(ivar)] = value ? 1 : 0;
}

static void DFSetCGFloatIvar(id object, const char *name, CGFloat value) {
    Ivar ivar = DFGetIvar(object, name);
    if (ivar == NULL) {
        return;
    }

    uint8_t *bytes = (uint8_t *)(__bridge void *)object;
    *((CGFloat *)(bytes + ivar_getOffset(ivar))) = value;
}

static void DFSetNSEdgeInsetsIvar(id object, const char *name, NSEdgeInsets value) {
    Ivar ivar = DFGetIvar(object, name);
    if (ivar == NULL) {
        return;
    }

    uint8_t *bytes = (uint8_t *)(__bridge void *)object;
    *((NSEdgeInsets *)(bytes + ivar_getOffset(ivar))) = value;
}

static void DFSetCGSizeIvar(id object, const char *name, CGSize value) {
    Ivar ivar = DFGetIvar(object, name);
    if (ivar == NULL) {
        return;
    }

    uint8_t *bytes = (uint8_t *)(__bridge void *)object;
    *((CGSize *)(bytes + ivar_getOffset(ivar))) = value;
}

static void DFStoreWeakObjectIvar(id object, const char *name, id value) {
    Ivar ivar = DFGetIvar(object, name);
    if (ivar == NULL) {
        return;
    }

    object_setIvar(object, ivar, value);
}

static void DFSetStrongObjectIvar(id object, const char *name, id value) {
    Ivar ivar = DFGetIvar(object, name);
    if (ivar == NULL) {
        return;
    }

    object_setIvar(object, ivar, value);
}

static BOOL DFSendHIDMessage(id hidClient, IndigoHIDMessage *message, BOOL freeWhenDone, NSError **error) {
    if (hidClient == nil || message == nil) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"Private SimulatorKit HID transport was unavailable."
            );
        }
        return NO;
    }

    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSError *sendError = nil;

    ((void(*)(id, SEL, IndigoHIDMessage *, BOOL, dispatch_queue_t, id))objc_msgSend)(
        hidClient,
        sel_registerName("sendWithMessage:freeWhenDone:completionQueue:completion:"),
        message,
        freeWhenDone,
        dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0),
        ^(NSError *completionError) {
            sendError = completionError;
            dispatch_semaphore_signal(semaphore);
        }
    );

    dispatch_time_t timeout = dispatch_time(DISPATCH_TIME_NOW, (int64_t)(3 * NSEC_PER_SEC));
    if (dispatch_semaphore_wait(semaphore, timeout) != 0) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"Timed out waiting for SimulatorKit HID delivery."
            );
        }
        return NO;
    }

    if (sendError != nil) {
        if (error != NULL) {
            *error = sendError;
        }
        return NO;
    }

    return YES;
}

static DFIndigoMessage *DFCreateIndigoTouchMessage(CGPoint normalizedPoint, NSSize displaySize, BOOL touchDown, NSError **error) {
    DFIndigoHIDMessageForMouseNSEventFn mouseMessage = (DFIndigoHIDMessageForMouseNSEventFn)dlsym(RTLD_DEFAULT, "IndigoHIDMessageForMouseNSEvent");
    if (mouseMessage == NULL) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not expose IndigoHIDMessageForMouseNSEvent."
            );
        }
        return NULL;
    }

    NSEventType eventType = touchDown ? NSEventTypeLeftMouseDown : NSEventTypeLeftMouseUp;
    CGPoint ratioPoint = CGPointMake(
        fmax(0.0, fmin(1.0, normalizedPoint.x)),
        fmax(0.0, fmin(1.0, normalizedPoint.y))
    );

    DFIndigoMessage *baseMessage = (DFIndigoMessage *)mouseMessage(&ratioPoint, NULL, DFIndigoTouchTarget, eventType, displaySize, 0);
    if (baseMessage == NULL) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit failed to create a base Indigo HID touch packet."
            );
        }
        return NULL;
    }

    size_t messageSize = sizeof(DFIndigoMessage) + sizeof(DFIndigoPayload);
    DFIndigoMessage *message = calloc(1, messageSize);
    if (message == NULL) {
        free(baseMessage);
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"Unable to allocate the Indigo HID touch packet."
            );
        }
        return NULL;
    }

    message->innerSize = (uint32_t)sizeof(DFIndigoPayload);
    message->eventType = DFIndigoEventTypeTouch;
    message->payload.field1 = DFIndigoTouchEventKind;
    message->payload.timestamp = mach_absolute_time();
    message->payload.event.touch = baseMessage->payload.event.touch;
    message->payload.event.touch.xRatio = ratioPoint.x;
    message->payload.event.touch.yRatio = ratioPoint.y;

    DFIndigoPayload *secondPayload = (DFIndigoPayload *)((uint8_t *)&message->payload + sizeof(DFIndigoPayload));
    memcpy(secondPayload, &message->payload, sizeof(DFIndigoPayload));
    secondPayload->event.touch.field1 = 0x1;
    secondPayload->event.touch.field2 = 0x2;

    free(baseMessage);
    return message;
}

static DFIndigoMessage *DFCreateIndigoMultiTouchMessage(CGPoint normalizedPoint1, CGPoint normalizedPoint2, NSSize displaySize, BOOL touchDown, NSError **error) {
    DFIndigoHIDMessageForMouseNSEventFn mouseMessage = (DFIndigoHIDMessageForMouseNSEventFn)dlsym(RTLD_DEFAULT, "IndigoHIDMessageForMouseNSEvent");
    if (mouseMessage == NULL) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not expose IndigoHIDMessageForMouseNSEvent."
            );
        }
        return NULL;
    }

    NSEventType eventType = touchDown ? NSEventTypeLeftMouseDown : NSEventTypeLeftMouseUp;
    CGPoint ratioPoint = CGPointMake(
        fmax(0.0, fmin(1.0, normalizedPoint1.x)),
        fmax(0.0, fmin(1.0, normalizedPoint1.y))
    );
    CGPoint secondRatioPoint = CGPointMake(
        fmax(0.0, fmin(1.0, normalizedPoint2.x)),
        fmax(0.0, fmin(1.0, normalizedPoint2.y))
    );

    DFIndigoMessage *baseMessage = (DFIndigoMessage *)mouseMessage(&ratioPoint, NULL, DFIndigoTouchTarget, eventType, displaySize, 0);
    if (baseMessage == NULL) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit failed to create a base Indigo HID touch packet."
            );
        }
        return NULL;
    }

    size_t messageSize = sizeof(DFIndigoMessage) + sizeof(DFIndigoPayload);
    DFIndigoMessage *message = calloc(1, messageSize);
    if (message == NULL) {
        free(baseMessage);
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"Unable to allocate the Indigo HID touch packet."
            );
        }
        return NULL;
    }

    message->innerSize = (uint32_t)sizeof(DFIndigoPayload);
    message->eventType = DFIndigoEventTypeTouch;
    message->payload.field1 = DFIndigoTouchEventKind;
    message->payload.timestamp = mach_absolute_time();
    message->payload.event.touch = baseMessage->payload.event.touch;
    message->payload.event.touch.xRatio = ratioPoint.x;
    message->payload.event.touch.yRatio = ratioPoint.y;

    DFIndigoPayload *secondPayload = (DFIndigoPayload *)((uint8_t *)&message->payload + sizeof(DFIndigoPayload));
    memcpy(secondPayload, &message->payload, sizeof(DFIndigoPayload));
    secondPayload->event.touch.field1 = 0x1;
    secondPayload->event.touch.field2 = 0x2;
    secondPayload->event.touch.xRatio = secondRatioPoint.x;
    secondPayload->event.touch.yRatio = secondRatioPoint.y;

    free(baseMessage);
    return message;
}

static IndigoHIDMessage *DFCreateKeyboardMessage(uint16_t keyCode, BOOL keyDown, NSError **error) {
    DFIndigoHIDMessageForKeyboardArbitraryFn keyboardMessage = (DFIndigoHIDMessageForKeyboardArbitraryFn)dlsym(RTLD_DEFAULT, "IndigoHIDMessageForKeyboardArbitrary");
    if (keyboardMessage == NULL) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not expose IndigoHIDMessageForKeyboardArbitrary."
            );
        }
        return NULL;
    }

    IndigoHIDMessage *message = keyboardMessage((int)keyCode, keyDown ? DFKeyboardDirectionDown : DFKeyboardDirectionUp);
    if (message == NULL && error != NULL) {
        *error = DFMakeError(
            DFPrivateSimulatorErrorCodeTouchDispatchFailed,
            [NSString stringWithFormat:@"SimulatorKit failed to encode keyboard HID for keyCode %u.", keyCode]
        );
    }
    return message;
}

static IndigoHIDMessage *DFCreateKeyboardMessageFromEvent(NSEvent *event, NSError **error) {
    DFIndigoHIDMessageForKeyboardNSEventFn keyboardMessage = (DFIndigoHIDMessageForKeyboardNSEventFn)dlsym(RTLD_DEFAULT, "IndigoHIDMessageForKeyboardNSEvent");
    if (keyboardMessage == NULL) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not expose IndigoHIDMessageForKeyboardNSEvent."
            );
        }
        return nil;
    }

    IndigoHIDMessage *message = keyboardMessage(event);
    if (message == NULL && error != NULL) {
        *error = DFMakeError(
            DFPrivateSimulatorErrorCodeTouchDispatchFailed,
            @"SimulatorKit could not construct an NSEvent keyboard HID packet."
        );
    }

    return message;
}

static IndigoHIDMessage *DFCreateButtonMessage(uint32_t buttonCode, uint32_t operation, uint32_t target, NSError **error) {
    DFIndigoHIDMessageForButtonFn buttonMessage = (DFIndigoHIDMessageForButtonFn)dlsym(RTLD_DEFAULT, "IndigoHIDMessageForButton");
    if (buttonMessage == NULL) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not expose IndigoHIDMessageForButton."
            );
        }
        return NULL;
    }

    IndigoHIDMessage *message = buttonMessage(buttonCode, operation, target);
    if (message == NULL && error != NULL) {
        *error = DFMakeError(
            DFPrivateSimulatorErrorCodeTouchDispatchFailed,
            [NSString stringWithFormat:@"SimulatorKit could not construct hardware button HID for code %u.", buttonCode]
        );
    }

    return message;
}

static IndigoHIDMessage *DFCreateArbitraryHIDMessage(uint32_t target, uint32_t page, uint32_t usage, uint32_t operation, NSError **error) {
    DFIndigoHIDMessageForHIDArbitraryFn arbitraryMessage = (DFIndigoHIDMessageForHIDArbitraryFn)dlsym(RTLD_DEFAULT, "IndigoHIDMessageForHIDArbitrary");
    if (arbitraryMessage == NULL) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not expose IndigoHIDMessageForHIDArbitrary."
            );
        }
        return NULL;
    }

    IndigoHIDMessage *message = arbitraryMessage(target, page, usage, operation);
    if (message == NULL && error != NULL) {
        *error = DFMakeError(
            DFPrivateSimulatorErrorCodeTouchDispatchFailed,
            [NSString stringWithFormat:@"SimulatorKit could not construct arbitrary HID for page 0x%x usage 0x%x.", page, usage]
        );
    }

    return message;
}

static BOOL DFCallSwiftUnitAngleMeasurementGetterByFunction(id selfObject, void *function, DFUnitAngleMeasurement *measurement) {
    if (selfObject == nil || function == NULL || measurement == NULL) {
        return NO;
    }

    DFUnitAngleMeasurement result = { nil, 0 };
    __asm__ volatile(
        "mov x20, %0\n"
        "mov x8, %1\n"
        "blr %2\n"
        :
        : "r" (selfObject), "r" (&result), "r" (function)
        : "x8", "x20", "x30", "memory"
    );

    *measurement = result;
    return YES;
}

static BOOL __attribute__((unused)) DFCallSwiftUnitAngleMeasurementGetter(id selfObject, const char *symbolName, DFUnitAngleMeasurement *measurement) {
    if (selfObject == nil || measurement == NULL) return NO;
    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        DFLogMissingSymbolOnce(symbolName);
        return NO;
    }
    return DFCallSwiftUnitAngleMeasurementGetterByFunction(selfObject, function, measurement);
}

static BOOL DFCallSwiftUnitAngleMeasurementGetterByPattern(id selfObject, const char *prefix, const char *suffix, const char *role, DFUnitAngleMeasurement *measurement) {
    if (selfObject == nil || measurement == NULL) return NO;
    void *function = DFResolveSwiftSymbol(prefix, suffix, role);
    return DFCallSwiftUnitAngleMeasurementGetterByFunction(selfObject, function, measurement);
}

static BOOL DFCallSwiftUnitAngleMeasurementSetterByFunction(id selfObject, DFUnitAngleMeasurement measurement, void *function) {
    if (selfObject == nil || function == NULL) {
        return NO;
    }

    DFUnitAngleMeasurement value = measurement;
    __asm__ volatile(
        "mov x20, %0\n"
        "mov x0, %1\n"
        "blr %2\n"
        :
        : "r" (selfObject), "r" (&value), "r" (function)
        : "x0", "x20", "x30", "memory"
    );

    return YES;
}

static BOOL __attribute__((unused)) DFCallSwiftUnitAngleMeasurementSetter(id selfObject, DFUnitAngleMeasurement measurement, const char *symbolName) {
    if (selfObject == nil) return NO;
    void *function = dlsym(RTLD_DEFAULT, symbolName);
    if (function == NULL) {
        DFLogMissingSymbolOnce(symbolName);
        return NO;
    }
    return DFCallSwiftUnitAngleMeasurementSetterByFunction(selfObject, measurement, function);
}

static BOOL DFCallSwiftUnitAngleMeasurementSetterByPattern(id selfObject, DFUnitAngleMeasurement measurement, const char *prefix, const char *suffix, const char *role) {
    if (selfObject == nil) return NO;
    void *function = DFResolveSwiftSymbol(prefix, suffix, role);
    return DFCallSwiftUnitAngleMeasurementSetterByFunction(selfObject, measurement, function);
}

static double DFNormalizedDegrees(double value) {
    double normalized = fmod(value, 360.0);
    if (normalized < 0) {
        normalized += 360.0;
    }
    return normalized;
}

static NSArray<NSString *> * DFInterestingSelectorsForObject(id object) {
    if (object == nil) {
        return @[];
    }

    NSMutableArray<NSString *> *selectors = [NSMutableArray array];
    unsigned int methodCount = 0;
    Method *methods = class_copyMethodList([object class], &methodCount);
    if (methods == NULL) {
        return @[];
    }

    NSArray<NSString *> *terms = @[@"orient", @"rotate", @"button", @"home", @"display", @"screen", @"face"];
    for (unsigned int index = 0; index < methodCount; index += 1) {
        NSString *name = NSStringFromSelector(method_getName(methods[index]));
        NSString *lowercased = name.lowercaseString;
        for (NSString *term in terms) {
            if ([lowercased containsString:term]) {
                [selectors addObject:name];
                break;
            }
        }
    }
    free(methods);

    return [selectors sortedArrayUsingSelector:@selector(compare:)];
}

static void DFLogRuntimeShape(id object, NSString *label) {
    if (object == nil) {
        DFLog(@"%@ is nil", label);
        return;
    }

    DFLog(@"%@ class=%@", label, NSStringFromClass([object class]));
    NSArray<NSString *> *selectors = DFInterestingSelectorsForObject(object);
    if (selectors.count > 0) {
        DFLog(@"%@ interesting selectors=%@", label, selectors);
    }
}

static NSInteger DFOrientationEquivalentValueForMeasurement(DFUnitAngleMeasurement measurement) {
    double degrees = DFNormalizedDegrees(measurement.value);
    if (fabs(degrees - 0.0) < 0.5 || fabs(degrees - 360.0) < 0.5) {
        return 1;
    }
    if (fabs(degrees - 180.0) < 0.5) {
        return 2;
    }
    if (fabs(degrees - 90.0) < 0.5) {
        return 4;
    }
    if (fabs(degrees - 270.0) < 0.5) {
        return 3;
    }
    return 1;
}

static BOOL DFSendIntegerSelectorIfAvailable(id target, const char *selectorName, NSInteger value) {
    if (target == nil || selectorName == NULL) {
        return NO;
    }

    SEL selector = sel_registerName(selectorName);
    if (![target respondsToSelector:selector]) {
        return NO;
    }

    ((void(*)(id, SEL, NSInteger))objc_msgSend)(target, selector, value);
    return YES;
}

// Port lookup on SimDevice — still exposed on macOS 26 even though sendPurpleEvent:
// has been pruned. Returns 0 if unavailable. Matches idb's
// `[simulator.device lookup:@"PurpleWorkspacePort" error:&err]` call.
static mach_port_t DFLookupSimDeviceMachPort(id device, NSString *portName) {
    if (device == nil || portName.length == 0) {
        return MACH_PORT_NULL;
    }

    SEL selector = sel_registerName("lookup:error:");
    if (![device respondsToSelector:selector]) {
        return MACH_PORT_NULL;
    }

    NSError *lookupError = nil;
    mach_port_t port = ((mach_port_t(*)(id, SEL, NSString *, NSError **))objc_msgSend)(
        device, selector, portName, &lookupError);
    if (port == MACH_PORT_NULL && lookupError != nil) {
        DFLog(@"SimDevice lookup:%@ error: %@", portName, lookupError.localizedDescription);
    }
    return port;
}

// Send a GSEvent mach message over PurpleWorkspacePort. Reimplements fbsimctl's
// FBSimulatorHID.sendPurpleEvent: / FBSimulatorPurpleHID.orientationEvent: in a
// self-contained form because modern SimDevice no longer exposes sendPurpleEvent:.
//
// Wire format (see idb PrivateHeaders/SimulatorApp/GSEvent.h):
//   0x00  msgh_bits          = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0)
//   0x04  msgh_size          = 108 (0x6C) — align4(4 + 0x6B)
//   0x08  msgh_remote_port   = PurpleWorkspacePort
//   0x0C  msgh_local_port    = MACH_PORT_NULL
//   0x14  msgh_id            = 0x7B
//   0x18  GSEvent type       = GSEventTypeDeviceOrientationChanged | GSEventHostFlag
//   0x48  record_info_size   = 4
//   0x4C  orientation value  = UIDeviceOrientation
static BOOL DFSendPurpleOrientationEvent(id device, NSInteger orientationValue) {
    mach_port_t purplePort = DFLookupSimDeviceMachPort(device, @"PurpleWorkspacePort");
    if (purplePort == MACH_PORT_NULL) {
        DFLog(@"PurpleWorkspacePort unavailable — cannot send GSEvent orientation");
        return NO;
    }

    uint8_t buf[112];
    memset(buf, 0, sizeof(buf));

    mach_msg_header_t *header = (mach_msg_header_t *)buf;
    header->msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0);
    header->msgh_size = 108;
    header->msgh_remote_port = purplePort;
    header->msgh_local_port = MACH_PORT_NULL;
    header->msgh_id = DFGSEventMachMessageID;

    *(uint32_t *)(buf + 0x18) = DFGSEventTypeDeviceOrientationChanged | DFGSEventHostFlag;
    *(uint32_t *)(buf + 0x48) = 4;
    *(uint32_t *)(buf + 0x4C) = (uint32_t)orientationValue;

    kern_return_t kr = mach_msg_send(header);
    if (kr != KERN_SUCCESS) {
        DFLog(@"mach_msg_send to PurpleWorkspacePort failed: %d (%s)", kr, mach_error_string(kr));
        return NO;
    }

    DFLog(@"Sent GSEvent orientation %ld via PurpleWorkspacePort", (long)orientationValue);
    return YES;
}

// Post a Darwin notification inside the guest iOS via SimDevice. Same selector
// idb uses in FBSimulatorHID.postDarwinNotification:error:. This is the fallback
// channel we use to signal orientation changes because on iOS 26 the GSEvent
// orientation pipe no longer drives UIKit autorotation. Apps can observe
// `org.nativescript.simdeck.rotate.<name>` and force a geometry update.
static BOOL DFPostSimDeviceDarwinNotification(id device, NSString *notificationName) {
    if (device == nil || notificationName.length == 0) {
        return NO;
    }

    SEL selector = sel_registerName("postDarwinNotification:error:");
    if (![device respondsToSelector:selector]) {
        return NO;
    }

    NSError *postError = nil;
    BOOL ok = ((BOOL(*)(id, SEL, NSString *, NSError **))objc_msgSend)(
        device, selector, notificationName, &postError);
    if (!ok) {
        DFLog(@"postDarwinNotification:%@ failed: %@", notificationName, postError.localizedDescription ?: @"unknown");
        return NO;
    }
    DFLog(@"Posted Darwin notification: %@", notificationName);
    return YES;
}

static NSString *DFRotationNotificationNameForOrientation(NSInteger orientationValue) {
    // Matches UIDeviceOrientation enum.
    switch (orientationValue) {
    case 1: return @"org.nativescript.simdeck.rotate.portrait";
    case 2: return @"org.nativescript.simdeck.rotate.portrait-upside-down";
    case 3: return @"org.nativescript.simdeck.rotate.landscape-left";
    case 4: return @"org.nativescript.simdeck.rotate.landscape-right";
    default: return nil;
    }
}

static BOOL DFTrySendIntegerSelectors(id target, NSString *label, NSInteger value) {
    if (target == nil) {
        return NO;
    }

    static const char *const candidateSelectors[] = {
        "gsEventsSendOrientation:",
        "setDeviceOrientation:",
        "setOrientation:",
        "_setDeviceOrientation:",
        "_setOrientation:",
        "setInterfaceOrientation:",
        "_setInterfaceOrientation:",
        "sendOrientation:",
        "simulateOrientation:",
        "setSimulatedDeviceOrientation:",
        "applyOrientation:",
        "requestOrientationChange:",
        "setCurrentOrientation:",
        "rotateToOrientation:",
    };

    size_t count = sizeof(candidateSelectors) / sizeof(candidateSelectors[0]);
    for (size_t index = 0; index < count; index += 1) {
        if (DFSendIntegerSelectorIfAvailable(target, candidateSelectors[index], value)) {
            DFLog(@"Sent orientation %ld via %@ -%s", (long)value, label, candidateSelectors[index]);
            return YES;
        }
    }

    return NO;
}

static BOOL DFSendDeviceOrientationEvent(id device, NSInteger orientationValue) {
    if (DFTrySendIntegerSelectors(device, @"device", orientationValue)) {
        return YES;
    }

    if (DFSendPurpleOrientationEvent(device, orientationValue)) {
        return YES;
    }

    return NO;
}

static BOOL DFSetDisplayRotationMeasurement(id object, DFUnitAngleMeasurement measurement, const char *prefix, const char *role) {
    if (object == nil || prefix == NULL) {
        return NO;
    }

    return DFCallSwiftUnitAngleMeasurementSetterByPattern(object, measurement, prefix, "vsTj", role);
}

static void DFConfigureDisplayGeometry(id displayView, CGSize displaySize) {
    if (displayView == nil || displaySize.width <= 0 || displaySize.height <= 0) {
        return;
    }

    NSRect frame = NSMakeRect(0, 0, displaySize.width, displaySize.height);
    if ([displayView respondsToSelector:@selector(setFrame:)]) {
        ((void(*)(id, SEL, NSRect))objc_msgSend)(displayView, @selector(setFrame:), frame);
    }

    id chromeView = DFGetObjectIvar(displayView, "chromeView");
    if (chromeView != nil) {
        if ([chromeView respondsToSelector:@selector(setFrame:)]) {
            ((void(*)(id, SEL, NSRect))objc_msgSend)(chromeView, @selector(setFrame:), frame);
        }
        DFSetCGSizeIvar(chromeView, "displaySize", displaySize);

        id chromeRenderView = DFGetObjectIvar(chromeView, "_renderView");
        if (chromeRenderView != nil) {
            if ([chromeRenderView respondsToSelector:@selector(setFrame:)]) {
                ((void(*)(id, SEL, NSRect))objc_msgSend)(chromeRenderView, @selector(setFrame:), frame);
            }
            DFSetCGSizeIvar(chromeRenderView, "displaySize", displaySize);
        }
    }
}

static NSArray *DFChromeInputsForDisplayView(id displayView) {
    if (displayView == nil) {
        return nil;
    }

    id chromeView = DFGetObjectIvar(displayView, "chromeView");
    if (chromeView == nil) {
        return nil;
    }

    id inputs = DFGetObjectIvar(chromeView, "_inputs");
    return [inputs isKindOfClass:[NSArray class]] ? inputs : nil;
}

static id DFChromeInputButton(id chromeInput) {
    return DFGetObjectIvar(chromeInput, "_button");
}

static id DFChromeInputDescriptorObject(id chromeInput) {
    return DFGetObjectIvar(chromeInput, "input");
}

static NSString *DFChromeInputIdentifier(id chromeInput) {
    NSString *uuid = DFTrimmedString(DFOptionalStringFromObjectSelector(chromeInput, "uuid"));
    if (uuid.length > 0) {
        return uuid;
    }

    id button = DFChromeInputButton(chromeInput);
    id identifier = button != nil ? DFSendObject(button, "identifier") : nil;
    if ([identifier respondsToSelector:@selector(description)]) {
        NSString *description = DFTrimmedString([identifier description]);
        if (description.length > 0) {
            return description;
        }
    }

    NSString *title = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "title"));
    if (title.length > 0) {
        return [@"title:" stringByAppendingString:title];
    }

    NSString *toolTip = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "toolTip"));
    if (toolTip.length > 0) {
        return [@"tooltip:" stringByAppendingString:toolTip];
    }

    NSString *accessibilityLabel = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "accessibilityLabel"));
    if (accessibilityLabel.length > 0) {
        return [@"ax:" stringByAppendingString:accessibilityLabel];
    }

    id input = DFChromeInputDescriptorObject(chromeInput);
    if (input != nil) {
        NSString *inputDescription = DFTrimmedString([input respondsToSelector:@selector(description)] ? [input description] : nil);
        if (inputDescription.length > 0) {
            return [@"input:" stringByAppendingString:inputDescription];
        }
    }

    return [NSString stringWithFormat:@"ptr:%p", chromeInput];
}

static NSString *DFChromeInputSummary(id chromeInput) {
    if (chromeInput == nil) {
        return @"<nil>";
    }

    NSMutableArray<NSString *> *parts = [NSMutableArray array];

    NSString *uuid = DFOptionalStringFromObjectSelector(chromeInput, "uuid");
    if (uuid.length > 0) {
        [parts addObject:[NSString stringWithFormat:@"uuid=%@", uuid]];
    }

    id button = DFChromeInputButton(chromeInput);
    if (button != nil) {
        NSString *title = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "title"));
        if (title.length > 0) {
            [parts addObject:[NSString stringWithFormat:@"title=%@", title]];
        }

        NSString *toolTip = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "toolTip"));
        if (toolTip.length > 0) {
            [parts addObject:[NSString stringWithFormat:@"toolTip=%@", toolTip]];
        }

        NSString *accessibilityLabel = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "accessibilityLabel"));
        if (accessibilityLabel.length > 0) {
            [parts addObject:[NSString stringWithFormat:@"ax=%@", accessibilityLabel]];
        }

        id identifier = DFSendObject(button, "identifier");
        if ([identifier respondsToSelector:@selector(description)]) {
            NSString *identifierDescription = DFTrimmedString([identifier description]);
            if (identifierDescription.length > 0) {
                [parts addObject:[NSString stringWithFormat:@"identifier=%@", identifierDescription]];
            }
        }
    }

    id input = DFChromeInputDescriptorObject(chromeInput);
    if (input != nil) {
        NSString *inputClass = NSStringFromClass([input class]);
        if (inputClass.length > 0) {
            [parts addObject:[NSString stringWithFormat:@"inputClass=%@", inputClass]];
        }

        if ([input respondsToSelector:@selector(description)]) {
            NSString *inputDescription = DFTrimmedString([input description]);
            if (inputDescription.length > 0 && ![inputDescription isEqualToString:inputClass]) {
                [parts addObject:[NSString stringWithFormat:@"input=%@", inputDescription]];
            }
        }
    }

    if (parts.count == 0) {
        return NSStringFromClass([chromeInput class]);
    }

    return [parts componentsJoinedByString:@", "];
}

static BOOL DFChromeInputMatchesHome(id chromeInput) {
    NSString *summary = DFChromeInputSummary(chromeInput).lowercaseString;
    return [summary containsString:@"home"];
}

static BOOL DFTriggerChromeInput(id chromeInput, NSError **error) {
    if (chromeInput == nil) {
        return NO;
    }

    SEL selector = sel_registerName("buttonClicked:");
    if (![chromeInput respondsToSelector:selector]) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit chrome input did not expose buttonClicked:."
            );
        }
        return NO;
    }

    id button = DFChromeInputButton(chromeInput);
    ((void(*)(id, SEL, id))objc_msgSend)(chromeInput, selector, button);
    return YES;
}

static id DFFindChromeInputForIdentifier(id displayView, NSString *identifier) {
    if (identifier.length == 0) {
        return nil;
    }

    for (id chromeInput in DFChromeInputsForDisplayView(displayView)) {
        if ([DFChromeInputIdentifier(chromeInput) isEqualToString:identifier]) {
            return chromeInput;
        }
    }

    return nil;
}

static NSArray *DFSubviewsForObject(id object) {
    if (object == nil || ![object respondsToSelector:@selector(subviews)]) {
        return @[];
    }

    id subviews = ((id(*)(id, SEL))objc_msgSend)(object, @selector(subviews));
    return [subviews isKindOfClass:[NSArray class]] ? subviews : @[];
}

static NSArray<id> *DFFlattenViewTree(id root) {
    if (root == nil) {
        return @[];
    }

    NSMutableArray<id> *orderedViews = [NSMutableArray array];
    NSMutableArray<id> *pending = [NSMutableArray arrayWithObject:root];
    NSMutableSet<NSValue *> *visited = [NSMutableSet set];

    while (pending.count > 0) {
        id candidate = pending.lastObject;
        [pending removeLastObject];

        NSValue *pointerValue = [NSValue valueWithPointer:(__bridge const void *)candidate];
        if ([visited containsObject:pointerValue]) {
            continue;
        }

        [visited addObject:pointerValue];
        [orderedViews addObject:candidate];

        NSArray *subviews = DFSubviewsForObject(candidate);
        for (id subview in [subviews reverseObjectEnumerator]) {
            [pending addObject:subview];
        }
    }

    return orderedViews;
}

static NSString *DFButtonLikeControlIdentifier(id control) {
    if (control == nil) {
        return nil;
    }

    return [NSString stringWithFormat:@"button:%p", control];
}

static NSString *DFButtonLikeControlTitle(id control) {
    return DFTrimmedString(DFOptionalStringFromObjectSelector(control, "title"));
}

static NSString *DFButtonLikeControlToolTip(id control) {
    return DFTrimmedString(DFOptionalStringFromObjectSelector(control, "toolTip"));
}

static NSString *DFButtonLikeControlAccessibilityLabel(id control) {
    return DFTrimmedString(DFOptionalStringFromObjectSelector(control, "accessibilityLabel"));
}

static NSString *DFButtonLikeControlSummary(id control) {
    if (control == nil) {
        return @"<nil>";
    }

    NSMutableArray<NSString *> *parts = [NSMutableArray array];
    [parts addObject:[NSString stringWithFormat:@"class=%@", NSStringFromClass([control class])]];

    NSString *title = DFButtonLikeControlTitle(control);
    if (title.length > 0) {
        [parts addObject:[NSString stringWithFormat:@"title=%@", title]];
    }

    NSString *toolTip = DFButtonLikeControlToolTip(control);
    if (toolTip.length > 0) {
        [parts addObject:[NSString stringWithFormat:@"toolTip=%@", toolTip]];
    }

    NSString *accessibilityLabel = DFButtonLikeControlAccessibilityLabel(control);
    if (accessibilityLabel.length > 0) {
        [parts addObject:[NSString stringWithFormat:@"ax=%@", accessibilityLabel]];
    }

    if ([control respondsToSelector:@selector(identifier)]) {
        id identifier = ((id(*)(id, SEL))objc_msgSend)(control, @selector(identifier));
        if ([identifier respondsToSelector:@selector(description)]) {
            NSString *identifierDescription = DFTrimmedString([identifier description]);
            if (identifierDescription.length > 0) {
                [parts addObject:[NSString stringWithFormat:@"identifier=%@", identifierDescription]];
            }
        }
    }

    return [parts componentsJoinedByString:@", "];
}

static BOOL DFButtonLikeControlLooksInteractive(id control) {
    if (control == nil) {
        return NO;
    }

    NSString *className = NSStringFromClass([control class]).lowercaseString;
    if ([className containsString:@"button"]) {
        return YES;
    }

    if ([control respondsToSelector:sel_registerName("performClick:")]) {
        return YES;
    }

    if ([control respondsToSelector:@selector(action)] && [control respondsToSelector:@selector(target)]) {
        return YES;
    }

    return NO;
}

static NSArray<id> *DFButtonLikeControlsForDisplayView(id displayView) {
    if (displayView == nil) {
        return @[];
    }

    id chromeView = DFGetObjectIvar(displayView, "chromeView");
    NSArray<id> *viewTree = DFFlattenViewTree(chromeView ?: displayView);
    NSMutableArray<id> *controls = [NSMutableArray array];

    for (id view in viewTree) {
        if (!DFButtonLikeControlLooksInteractive(view)) {
            continue;
        }

        NSString *title = DFButtonLikeControlTitle(view);
        NSString *toolTip = DFButtonLikeControlToolTip(view);
        NSString *accessibilityLabel = DFButtonLikeControlAccessibilityLabel(view);
        if (title.length == 0 && toolTip.length == 0 && accessibilityLabel.length == 0) {
            continue;
        }

        [controls addObject:view];
    }

    return controls;
}

static id DFFindButtonLikeControlForIdentifier(id displayView, NSString *identifier) {
    if (identifier.length == 0) {
        return nil;
    }

    for (id control in DFButtonLikeControlsForDisplayView(displayView)) {
        if ([DFButtonLikeControlIdentifier(control) isEqualToString:identifier]) {
            return control;
        }
    }

    return nil;
}

static BOOL DFTriggerButtonLikeControl(id control, NSError **error) {
    if (control == nil) {
        return NO;
    }

    SEL performClickSelector = sel_registerName("performClick:");
    if ([control respondsToSelector:performClickSelector]) {
        ((void(*)(id, SEL, id))objc_msgSend)(control, performClickSelector, nil);
        return YES;
    }

    SEL accessibilityPressSelector = sel_registerName("accessibilityPerformPress");
    if ([control respondsToSelector:accessibilityPressSelector]) {
        BOOL didPress = ((BOOL(*)(id, SEL))objc_msgSend)(control, accessibilityPressSelector);
        if (didPress) {
            return YES;
        }
    }

    if ([control respondsToSelector:@selector(action)] && [control respondsToSelector:@selector(target)]) {
        SEL action = ((SEL(*)(id, SEL))objc_msgSend)(control, @selector(action));
        id target = ((id(*)(id, SEL))objc_msgSend)(control, @selector(target));
        if (action != NULL && target != nil && [NSApp sendAction:action to:target from:control]) {
            return YES;
        }
    }

    if (error != NULL) {
        *error = DFMakeError(
            DFPrivateSimulatorErrorCodeTouchDispatchFailed,
            @"SimulatorKit exposed a control view, but it did not respond to performClick:, accessibilityPerformPress, or action dispatch."
        );
    }
    return NO;
}

static void DFLogChromeRuntimeState(id displayView) {
    if (displayView == nil) {
        DFLog(@"displayView is nil");
        return;
    }

    id chromeView = DFGetObjectIvar(displayView, "chromeView");
    DFLogRuntimeShape(chromeView, @"chromeView");

    id chromeRenderView = DFGetObjectIvar(chromeView, "_renderView");
    if (chromeRenderView != nil) {
        DFLogRuntimeShape(chromeRenderView, @"chromeRenderView");
    }

    NSArray *chromeInputs = DFChromeInputsForDisplayView(displayView) ?: @[];
    NSMutableArray<NSString *> *inputSummaries = [NSMutableArray arrayWithCapacity:chromeInputs.count];
    for (id chromeInput in chromeInputs) {
        [inputSummaries addObject:DFChromeInputSummary(chromeInput)];
    }
    DFLog(@"SimulatorKit chrome inputs: %@", inputSummaries);

    NSArray *buttonLikeControls = DFButtonLikeControlsForDisplayView(displayView);
    NSMutableArray<NSString *> *buttonSummaries = [NSMutableArray arrayWithCapacity:buttonLikeControls.count];
    for (id control in buttonLikeControls) {
        [buttonSummaries addObject:DFButtonLikeControlSummary(control)];
    }
    DFLog(@"SimulatorKit button-like controls: %@", buttonSummaries);
}

static BOOL DFSendSingleKeyboardEvent(id hidClient, uint16_t keyCode, BOOL keyDown, NSError **error) {
    IndigoHIDMessage *message = DFCreateKeyboardMessage(keyCode, keyDown, error);
    if (message == NULL) {
        return NO;
    }

    return DFSendHIDMessage(hidClient, message, YES, error);
}

typedef struct {
    const char *label;
    BOOL useButton;
    uint32_t buttonCode;
    uint32_t consumerPage;
    uint32_t consumerUsage;
    uint32_t target;
} DFHomeButtonHIDStrategy;

static BOOL DFSendHomeStrategyEdge(id hidClient, const DFHomeButtonHIDStrategy *strategy, uint32_t operation, NSError **error) {
    IndigoHIDMessage *message = strategy->useButton
        ? DFCreateButtonMessage(strategy->buttonCode, operation, strategy->target, error)
        : DFCreateArbitraryHIDMessage(strategy->target, strategy->consumerPage, strategy->consumerUsage, operation, error);
    if (message == NULL) {
        return NO;
    }
    return DFSendHIDMessage(hidClient, message, YES, error);
}

static BOOL DFPressHomeViaHIDClient(id hidClient, NSError **error) {
    if (hidClient == nil) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not provide a headless HID client for Home."
            );
        }
        return NO;
    }

    // Ordered from most likely correct to last-resort. Each entry gets a press+release;
    // the first strategy that SimulatorKit accepts wins. Labels are logged so you can
    // cross-reference the private bridge log against whatever iOS actually did.
    //
    // The consumer-control paths are tried first because the IndigoButton path with
    // code=0x191 is accepted by some runtimes but routed to a different hardware
    // button (observed on macOS 26 simulators as a reboot trigger). The usage-page
    // 0x0c / usage 0x40 (Menu) is the documented fbsimctl mapping for iOS Home.
    static const DFHomeButtonHIDStrategy strategies[] = {
        // USB consumer-control Menu (0x0c/0x40) — the documented iOS Home mapping
        // used by fbsimctl / idb; works on both Face ID and Touch ID simulators.
        { "IndigoHIDMessageForHIDArbitrary page=0x0c usage=0x40 (Menu) target=0x32", NO, 0, DFConsumerControlUsagePage, 0x40, DFIndigoTouchTarget },
        // Older iOS builds exposed Home directly at usage 0x65 on the consumer page.
        { "IndigoHIDMessageForHIDArbitrary page=0x0c usage=0x65 (Home) target=0x32", NO, 0, DFConsumerControlUsagePage, DFHomeConsumerUsage, DFIndigoTouchTarget },
        // Legacy IndigoButton paths — may be rejected, accepted-but-misrouted, or
        // correctly map to Home depending on the SimulatorKit build. Kept as a last
        // resort after the documented consumer-control paths.
        { "IndigoHIDMessageForButton code=0x191 target=0x2",  YES, DFHomeButtonCode, 0, 0, 0x2 },
        { "IndigoHIDMessageForButton code=0x191 target=0x32", YES, DFHomeButtonCode, 0, 0, DFIndigoTouchTarget },
    };

    NSError *lastError = nil;
    for (size_t index = 0; index < sizeof(strategies) / sizeof(strategies[0]); index++) {
        const DFHomeButtonHIDStrategy *strategy = &strategies[index];

        NSError *downError = nil;
        if (!DFSendHomeStrategyEdge(hidClient, strategy, DFButtonDirectionDown, &downError)) {
            DFLog(@"Home strategy rejected (down): %s — %@", strategy->label, downError.localizedDescription ?: @"no error");
            lastError = downError;
            continue;
        }

        [NSThread sleepForTimeInterval:0.08];

        NSError *upError = nil;
        if (!DFSendHomeStrategyEdge(hidClient, strategy, DFButtonDirectionUp, &upError)) {
            DFLog(@"Home strategy rejected (up): %s — %@", strategy->label, upError.localizedDescription ?: @"no error");
            lastError = upError;
            continue;
        }

        DFLog(@"Home dispatched via %s", strategy->label);
        return YES;
    }

    if (error != NULL) {
        *error = lastError ?: DFMakeError(
            DFPrivateSimulatorErrorCodeTouchDispatchFailed,
            @"SimulatorKit rejected every Home HID strategy."
        );
    }
    return NO;
}

@interface DFPrivateSimulatorDisplayBridge ()

@property (nonatomic, strong) NSView *displayView;

@end

@interface DFPrivateSimulatorChromeButton : NSObject

@property (nonatomic, copy, readonly) NSString *identifier;
@property (nonatomic, copy, readonly) NSString *title;
@property (nonatomic, copy, readonly) NSString *toolTip;
@property (nonatomic, copy, readonly) NSString *accessibilityLabel;
@property (nonatomic, copy, readonly) NSString *summary;

- (instancetype)initWithIdentifier:(NSString *)identifier
                             title:(NSString *)title
                           toolTip:(NSString *)toolTip
                accessibilityLabel:(NSString *)accessibilityLabel
                           summary:(NSString *)summary;

@end

@implementation DFPrivateSimulatorChromeButton

- (instancetype)initWithIdentifier:(NSString *)identifier
                             title:(NSString *)title
                           toolTip:(NSString *)toolTip
                accessibilityLabel:(NSString *)accessibilityLabel
                           summary:(NSString *)summary {
    self = [super init];
    if (self == nil) {
        return nil;
    }

    _identifier = [identifier copy];
    _title = [title copy];
    _toolTip = [toolTip copy];
    _accessibilityLabel = [accessibilityLabel copy];
    _summary = [summary copy];
    return self;
}

@end

@implementation DFPrivateSimulatorDisplayBridge {
    id _serviceContext;
    id _device;
    id _screenAdapterHost;
    id _screenAdapter;
    id _bootstrapScreen;
    id _activeScreen;
    id _rawScreen;
    id _hidClient;
    id _digitizerInputView;
    dispatch_queue_t _callbackQueue;
    NSWindow *_headlessHostWindow;
    NSView *_headlessHostView;
    NSUUID *_screenAdapterCallbackUUID;
    NSUUID *_screenCallbackUUID;
    CVPixelBufferRef _latestPixelBuffer;
    NSString *_displayStatusValue;
    uint32_t _activeScreenID;
    CGSize _displayPixelSize;
    CGPoint _lastTouchPoint;
    BOOL _hasLastTouchPoint;
    BOOL _hasLoggedFirstFrame;
    BOOL _isActivatingDisplay;
    BOOL _hasActivatedDisplay;
    BOOL _digitizerInputReady;
    double _deviceRotationDegrees;
}

+ (BOOL)loadPrivateFrameworks:(NSError **)error {
    static dispatch_once_t onceToken;
    static NSError *loadError = nil;

    dispatch_once(&onceToken, ^{
        if (!dlopen(DFCoreSimulatorPath.fileSystemRepresentation, RTLD_NOW | RTLD_GLOBAL)) {
            loadError = DFMakeError(
                DFPrivateSimulatorErrorCodeFrameworkLoadFailed,
                [NSString stringWithFormat:@"Unable to load CoreSimulator from %@.", DFCoreSimulatorPath]
            );
            return;
        }

        if (!dlopen(DFSimulatorKitPath.fileSystemRepresentation, RTLD_NOW | RTLD_GLOBAL)) {
            loadError = DFMakeError(
                DFPrivateSimulatorErrorCodeFrameworkLoadFailed,
                [NSString stringWithFormat:@"Unable to load SimulatorKit from %@.", DFSimulatorKitPath]
            );
        }
    });

    if (error != NULL) {
        *error = loadError;
    }

    return loadError == nil;
}

- (void)updateStatus:(NSString *)status {
    if ([_displayStatusValue isEqualToString:status]) {
        return;
    }
    _displayStatusValue = [status copy];
    DFLog(@"%@", _displayStatusValue);
    [self notifyDelegateOfStatus:_displayStatusValue isReady:_latestPixelBuffer != nil];
}

- (void)notifyDelegateOfStatus:(NSString *)status isReady:(BOOL)isReady {
    id<DFPrivateSimulatorDisplayBridgeDelegate> delegate = self.delegate;
    if (delegate == nil) {
        return;
    }

    NSString *statusCopy = [status copy];
    __weak typeof(self) weakSelf = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        __strong typeof(weakSelf) strongSelf = weakSelf;
        if (strongSelf == nil) {
            return;
        }
        [delegate privateSimulatorDisplayBridge:strongSelf didChangeDisplayStatus:statusCopy isReady:isReady];
    });
}

- (void)notifyDelegateOfFrame:(CVPixelBufferRef)pixelBuffer {
    id<DFPrivateSimulatorDisplayBridgeDelegate> delegate = self.delegate;
    if (delegate == nil || pixelBuffer == nil) {
        return;
    }

    CVPixelBufferRetain(pixelBuffer);
    __weak typeof(self) weakSelf = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        __strong typeof(weakSelf) strongSelf = weakSelf;
        if (strongSelf != nil) {
            [delegate privateSimulatorDisplayBridge:strongSelf didUpdateFrame:pixelBuffer];
        }
        CVPixelBufferRelease(pixelBuffer);
    });
}

- (nullable instancetype)initWithUDID:(NSString *)udid error:(NSError * _Nullable __autoreleasing *)error {
    return [self initWithUDID:udid attachDisplay:YES error:error];
}

- (nullable instancetype)initWithUDID:(NSString *)udid
                         attachDisplay:(BOOL)attachDisplay
                                 error:(NSError * _Nullable __autoreleasing *)error {
    if (![DFPrivateSimulatorDisplayBridge loadPrivateFrameworks:error]) {
        return nil;
    }

    self = [super init];
    if (self == nil) {
        return nil;
    }

    _callbackQueue = dispatch_queue_create("org.nativescript.simdeck.private-screen", DISPATCH_QUEUE_SERIAL);
    dispatch_queue_set_specific(_callbackQueue, DFPrivateSimulatorCallbackQueueKey, (void *)DFPrivateSimulatorCallbackQueueKey, NULL);
    [self updateStatus:[NSString stringWithFormat:@"Starting private CoreSimulator attach for %@", udid]];

    Class serviceContextClass = NSClassFromString(@"SimServiceContext");
    if (serviceContextClass == Nil) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeServiceContextFailed,
                @"CoreSimulator did not expose SimServiceContext in this Xcode runtime."
            );
        }
        return nil;
    }

    NSError *serviceError = nil;
    id contextAlloc = ((id(*)(id, SEL))objc_msgSend)(serviceContextClass, sel_registerName("alloc"));
    _serviceContext = ((id(*)(id, SEL, id, long long, NSError **))objc_msgSend)(
        contextAlloc,
        sel_registerName("initWithDeveloperDir:connectionType:error:"),
        nil,
        0LL,
        &serviceError
    );
    if (_serviceContext == nil) {
        if (error != NULL) {
            *error = serviceError ?: DFMakeError(
                DFPrivateSimulatorErrorCodeServiceContextFailed,
                @"Unable to create a CoreSimulator service context."
            );
        }
        return nil;
    }

    NSError *deviceSetError = nil;
    id deviceSet = ((id(*)(id, SEL, NSError **))objc_msgSend)(_serviceContext, sel_registerName("defaultDeviceSetWithError:"), &deviceSetError);
    if (deviceSet == nil) {
        if (error != NULL) {
            *error = deviceSetError ?: DFMakeError(
                DFPrivateSimulatorErrorCodeServiceContextFailed,
                @"Unable to access the default CoreSimulator device set."
            );
        }
        return nil;
    }

    NSArray *devices = DFSendObject(deviceSet, "devices");
    for (id candidate in devices) {
        id deviceUDID = DFSendObject(candidate, "UDID");
        NSString *candidateUDID = [deviceUDID respondsToSelector:sel_registerName("UUIDString")]
            ? DFSendObject(deviceUDID, "UUIDString")
            : [deviceUDID description];
        if ([candidateUDID isEqualToString:udid]) {
            _device = candidate;
            break;
        }
    }

    if (_device == nil) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeDeviceLookupFailed,
                [NSString stringWithFormat:@"Unable to locate simulator %@ inside the CoreSimulator device set.", udid]
            );
        }
        return nil;
    }

    Class legacyHIDClientClass = NSClassFromString(@"SimulatorKit.SimDeviceLegacyHIDClient");
    if (legacyHIDClientClass != Nil) {
        NSError *hidClientError = nil;
        id hidClientAlloc = ((id(*)(id, SEL))objc_msgSend)(legacyHIDClientClass, sel_registerName("alloc"));
        _hidClient = ((id(*)(id, SEL, id, NSError **))objc_msgSend)(
            hidClientAlloc,
            sel_registerName("initWithDevice:error:"),
            _device,
            &hidClientError
        );

        if (_hidClient != nil) {
            DFLog(@"Created private SimulatorKit HID client for %@", udid);
        } else {
            DFLog(@"Failed to create private SimulatorKit HID client for %@: %@", udid, hidClientError.localizedDescription ?: @"unknown error");
        }
    } else {
        DFLog(@"SimulatorKit legacy HID client class was unavailable.");
    }

    if (!attachDisplay) {
        _displayPixelSize = CGSizeMake(1.0, 1.0);
        [self updateStatus:@"Private HID ready"];
        return self;
    }

    _screenAdapterHost = DFCallSwiftSelfGetterByPattern(
        _device,
        "$sSo9SimDeviceC12SimulatorKitE13screenAdapter",
        "vg",
        "SimDevice.screenAdapter.getter (SimulatorKit extension)"
    );
    if (_screenAdapterHost == nil) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeDisplayAttachFailed,
                @"CoreSimulator did not expose a SimulatorKit screen adapter."
            );
        }
        return nil;
    }

    Class screenClass = NSClassFromString(@"SimulatorKit.SimDeviceScreen");
    if (screenClass == Nil) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeDisplayAttachFailed,
                @"SimulatorKit did not expose SimDeviceScreen."
            );
        }
        return nil;
    }

    _bootstrapScreen = ((id(*)(id, SEL))objc_msgSend)(screenClass, sel_registerName("alloc"));
    _bootstrapScreen = ((id(*)(id, SEL, id, uint32_t))objc_msgSend)(_bootstrapScreen, sel_registerName("initWithDevice:screenID:"), _device, 0);
    [self updateStatus:@"Waiting for CoreSimulator screen adapter"];
    DFSpinRunLoop(0.5);

    _screenAdapter = object_getIvar(_screenAdapterHost, class_getInstanceVariable([_screenAdapterHost class], "_screenAdapter"));
    if (_screenAdapter == nil) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeDisplayAttachFailed,
                @"CoreSimulator did not provide a headless screen adapter proxy."
            );
        }
        return nil;
    }

    _screenAdapterCallbackUUID = [NSUUID UUID];
    __weak typeof(self) weakSelf = self;
    ((void(*)(id, SEL, id, id, id, id))objc_msgSend)(
        _screenAdapter,
        sel_registerName("registerScreenAdapterCallbacksWithUUID:callbackQueue:screenConnectedCallback:screenWillDisconnectCallback:"),
        _screenAdapterCallbackUUID,
        _callbackQueue,
        ^(id simScreen) {
            (void)simScreen;
            __strong typeof(weakSelf) strongSelf = weakSelf;
            if (strongSelf == nil) {
                return;
            }
            [strongSelf updateStatus:@"CoreSimulator screen proxy connected"];
        },
        ^(id simScreen) {
            (void)simScreen;
            __strong typeof(weakSelf) strongSelf = weakSelf;
            if (strongSelf == nil) {
                return;
            }
            [strongSelf updateStatus:@"CoreSimulator screen proxy disconnected"];
        }
    );

    [self updateStatus:@"Waiting for headless simulator screens"];

    // Xcode 26.4's SimulatorKit doesn't expose adapter screens immediately after
    // the bootstrap SimDeviceScreen is allocated — they trickle in over a few
    // seconds (or only after Simulator.app primes the device). Poll instead of
    // relying on a fixed 0.5s sleep.
    NSDictionary<NSNumber *, id> *screens = DFReadAdapterScreens(_screenAdapterHost);
    NSDate *screenDeadline = [NSDate dateWithTimeIntervalSinceNow:10.0];
    while (screens.count == 0 && [screenDeadline timeIntervalSinceNow] > 0) {
        DFSpinRunLoop(0.1);
        screens = DFReadAdapterScreens(_screenAdapterHost);
    }
    if (screens.count == 0) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeDisplayAttachFailed,
                @"The CoreSimulator screen adapter did not expose any live screens."
            );
        }
        return nil;
    }

    NSArray<NSNumber *> *sortedScreenIDs = [[screens allKeys] sortedArrayUsingSelector:@selector(compare:)];
    NSNumber *selectedScreenID = nil;
    for (NSNumber *candidate in sortedScreenIDs) {
        if (candidate.unsignedIntValue > 0) {
            selectedScreenID = candidate;
            break;
        }
    }
    if (selectedScreenID == nil) {
        selectedScreenID = sortedScreenIDs.firstObject;
    }
    _activeScreenID = selectedScreenID.unsignedIntValue;
    DFLog(@"Discovered headless screens %@; selecting %@", sortedScreenIDs, selectedScreenID);

    _activeScreen = ((id(*)(id, SEL))objc_msgSend)(screenClass, sel_registerName("alloc"));
    _activeScreen = ((id(*)(id, SEL, id, uint32_t))objc_msgSend)(_activeScreen, sel_registerName("initWithDevice:screenID:"), _device, _activeScreenID);
    _rawScreen = screens[selectedScreenID];
    DFLogRuntimeShape(_activeScreen, @"activeScreen");
    DFLogRuntimeShape(_rawScreen, @"rawScreen");
    DFLogRuntimeShape(_device, @"device");
    DFSpinRunLoop(0.1);

    Class simDisplayViewClass = NSClassFromString(@"SimulatorKit.SimDisplayView");
    if (simDisplayViewClass != Nil) {
        DFRunOnMainSync(^{
            self->_displayView = DFAllocInitRect(simDisplayViewClass, NSMakeRect(0, 0, 1, 1));
            self->_digitizerInputView = self->_displayView != nil ? object_getIvar(self->_displayView, DFGetIvar(self->_displayView, "digitizerView")) : nil;
            if (self->_displayView != nil) {
                DFSetStrongObjectIvar(self->_displayView, "device", self->_device);
                ((void(*)(id, SEL, id))objc_msgSend)(self->_displayView, sel_registerName("setDevice:"), self->_device);
            }
            if (self->_digitizerInputView != nil && self->_hidClient != nil) {
                DFSetStrongObjectIvar(self->_displayView, "_hidClient", self->_hidClient);
                objc_setAssociatedObject(self->_digitizerInputView, DFDigitizerDelegateAssociationKey, self->_hidClient, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
                objc_setAssociatedObject(self->_digitizerInputView, DFDigitizerWakeDelegateAssociationKey, self->_displayView, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
                DFStoreWeakObjectIvar(self->_digitizerInputView, "delegate", self->_hidClient);
                DFStoreWeakObjectIvar(self->_digitizerInputView, "wakeOnTouchDelegate", self->_displayView);
                DFSetBoolIvar(self->_digitizerInputView, "isEnabled", YES);
                DFSetBoolIvar(self->_digitizerInputView, "isPaused", NO);
                DFSetBoolIvar(self->_digitizerInputView, "isAsleep", NO);
                DFSetCGFloatIvar(self->_digitizerInputView, "scale", 1.0);
                DFSetNSEdgeInsetsIvar(self->_digitizerInputView, "screenInset", NSEdgeInsetsMake(0, 0, 0, 0));
                self->_digitizerInputReady = YES;
                DFLog(@"Prepared SimDisplayView digitizer bridge for %@", udid);
            }
        });
    } else {
        DFLog(@"SimulatorKit display view class was unavailable.");
    }

    id rawScreen = _rawScreen;
    if (rawScreen == nil || ![rawScreen respondsToSelector:sel_registerName("registerScreenCallbacksWithUUID:callbackQueue:frameCallback:surfacesChangedCallback:propertiesChangedCallback:")]) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeDisplayAttachFailed,
                @"The selected CoreSimulator screen did not expose display callbacks."
            );
        }
        return nil;
    }

    _screenCallbackUUID = [NSUUID UUID];
    ((void(*)(id, SEL, id, id, id, id, id))objc_msgSend)(
        rawScreen,
        sel_registerName("registerScreenCallbacksWithUUID:callbackQueue:frameCallback:surfacesChangedCallback:propertiesChangedCallback:"),
        _screenCallbackUUID,
        _callbackQueue,
        ^{
            __strong typeof(weakSelf) strongSelf = weakSelf;
            if (strongSelf == nil || strongSelf->_latestPixelBuffer == nil) {
                return;
            }
            if (!strongSelf->_hasLoggedFirstFrame) {
                strongSelf->_hasLoggedFirstFrame = YES;
                [strongSelf updateStatus:@"Receiving headless screen frames"];
            }
            [strongSelf notifyDelegateOfFrame:strongSelf->_latestPixelBuffer];
        },
        ^(id surface, id maskedSurface) {
            (void)maskedSurface;
            __strong typeof(weakSelf) strongSelf = weakSelf;
            if (strongSelf == nil) {
                return;
            }

            CVPixelBufferRef pixelBuffer = DFCreatePixelBufferFromSurface((__bridge IOSurfaceRef)surface);
            if (pixelBuffer == nil) {
                [strongSelf updateStatus:@"Headless screen surfaced an unsupported IOSurface"];
                return;
            }

            if (strongSelf->_latestPixelBuffer != nil) {
                CVPixelBufferRelease(strongSelf->_latestPixelBuffer);
            }
            strongSelf->_latestPixelBuffer = pixelBuffer;
            size_t width = CVPixelBufferGetWidth(pixelBuffer);
            size_t height = CVPixelBufferGetHeight(pixelBuffer);
            strongSelf->_displayPixelSize = CGSizeMake((CGFloat)width, (CGFloat)height);
            [strongSelf notifyDelegateOfFrame:pixelBuffer];
            DFRunOnMainAsync(^{
                if (strongSelf->_headlessHostWindow != nil) {
                    NSSize windowSize = NSMakeSize((CGFloat)width, (CGFloat)height);
                    [strongSelf->_headlessHostWindow setContentSize:windowSize];
                    strongSelf->_headlessHostView.frame = NSMakeRect(0, 0, windowSize.width, windowSize.height);
                }
                DFConfigureDisplayGeometry(strongSelf->_displayView, strongSelf->_displayPixelSize);
                if (strongSelf->_digitizerInputView != nil) {
                    [strongSelf->_digitizerInputView setFrame:NSMakeRect(0, 0, (CGFloat)width, (CGFloat)height)];
                }
            });
            [strongSelf updateStatus:[NSString stringWithFormat:@"Private display ready (%zux%zu)", width, height]];
        },
        ^(id properties) {
            __strong typeof(weakSelf) strongSelf = weakSelf;
            if (strongSelf == nil) {
                return;
            }
            DFLog(@"Headless screen properties updated: class=%@", properties != nil ? NSStringFromClass([properties class]) : @"(nil)");
            if (properties != nil && [properties respondsToSelector:sel_registerName("uiOrientation")]) {
                NSInteger uiOrientation = ((NSInteger(*)(id, SEL))objc_msgSend)(properties, sel_registerName("uiOrientation"));
                DFLog(@"Headless screen uiOrientation=%ld", (long)uiOrientation);
            }
            [strongSelf updateStatus:@"Headless screen properties updated"];
        }
    );

    if (_displayView == nil) {
        _displayView = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 430, 932)];
        _displayView.wantsLayer = YES;
    }
    [self updateStatus:@"Waiting for IOSurface callback"];
    [self activateDisplayIfNeeded];

    DFSpinRunLoop(0.25);
    return self;
}

- (void)dealloc {
    if (_latestPixelBuffer != nil) {
        CVPixelBufferRelease(_latestPixelBuffer);
        _latestPixelBuffer = nil;
    }
}

- (void)activateDisplayIfNeeded {
    if (_hasActivatedDisplay || _isActivatingDisplay || _displayView == nil || _activeScreen == nil) {
        return;
    }

    DFRunOnMainSync(^{
        if (self->_hasActivatedDisplay || self->_isActivatingDisplay || self->_displayView == nil || self->_activeScreen == nil) {
            return;
        }

        if (self->_displayView.window == nil) {
            CGSize hostSize = self->_displayPixelSize;
            if (hostSize.width < 1.0 || hostSize.height < 1.0) {
                hostSize = CGSizeMake(430.0, 932.0);
            }

            if (self->_headlessHostWindow == nil) {
                NSRect frame = NSMakeRect(-10000.0, -10000.0, hostSize.width, hostSize.height);
                self->_headlessHostWindow = [[NSWindow alloc] initWithContentRect:frame
                                                                         styleMask:NSWindowStyleMaskBorderless
                                                                           backing:NSBackingStoreBuffered
                                                                             defer:NO];
                self->_headlessHostWindow.releasedWhenClosed = NO;
                self->_headlessHostWindow.opaque = NO;
                self->_headlessHostWindow.backgroundColor = [NSColor clearColor];
                self->_headlessHostWindow.hasShadow = NO;
                self->_headlessHostWindow.level = NSNormalWindowLevel;
                self->_headlessHostWindow.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorIgnoresCycle;
                self->_headlessHostWindow.ignoresMouseEvents = YES;
                self->_headlessHostView = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, hostSize.width, hostSize.height)];
                self->_headlessHostView.wantsLayer = YES;
                self->_headlessHostView.layer.backgroundColor = NSColor.clearColor.CGColor;
                self->_headlessHostWindow.contentView = self->_headlessHostView;
            } else {
                [self->_headlessHostWindow setContentSize:NSMakeSize(hostSize.width, hostSize.height)];
                self->_headlessHostView.frame = NSMakeRect(0, 0, hostSize.width, hostSize.height);
            }

            if (self->_displayView.superview != self->_headlessHostView) {
                [self->_displayView removeFromSuperview];
                self->_displayView.frame = self->_headlessHostView.bounds;
                self->_displayView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
                [self->_headlessHostView addSubview:self->_displayView];
            }

            [self->_headlessHostWindow orderFront:nil];
            DFLog(@"Mounted private SimulatorKit display into headless host window.");
        }

        if (self->_displayView.window == nil) {
            [self updateStatus:@"Waiting for private display host window"];
            return;
        }

        self->_isActivatingDisplay = YES;
        [self updateStatus:@"Attaching private SimulatorKit display"];

        NSError *activationError = nil;
        id renderableView = [self->_displayView respondsToSelector:sel_registerName("renderableView")]
            ? DFSendObject(self->_displayView, "renderableView")
            : nil;
        if (renderableView == nil) {
            activationError = DFMakeError(
                DFPrivateSimulatorErrorCodeDisplayAttachFailed,
                @"SimulatorKit did not expose a renderable view for the private display."
            );
        } else {
            // SimulatorKit.SimDisplayRenderableView.connect(screen:) — the ObjC
            // dispatch thunk (`Tj`) suffix is stable; the middle of the mangling
            // (parameter type) is what drifts.
            BOOL connected = DFCallSwiftVoidMethodWithSelfAndObjectByPattern(
                renderableView,
                self->_activeScreen,
                "$s12SimulatorKit24SimDisplayRenderableViewC7connect",
                "FTj",
                "SimDisplayRenderableView.connect(screen:)"
            );
            if (!connected) {
                activationError = DFMakeError(
                    DFPrivateSimulatorErrorCodeDisplayAttachFailed,
                    @"Failed to locate SimulatorKit renderableView.connect(screen:)."
                );
            } else {
                DFLog(@"Activated private renderableView.connect(screen:) without SimDisplayView.connect(screen:inputs:).");
            }
        }

        if (activationError != nil) {
            self->_isActivatingDisplay = NO;
            [self updateStatus:[NSString stringWithFormat:@"Private SimulatorKit attach failed: %@", activationError.localizedDescription ?: @"unknown error"]];
            DFLog(@"Private SimulatorKit display activation failed: %@", activationError);
            return;
        }

        self->_hasActivatedDisplay = YES;
        self->_isActivatingDisplay = NO;
        [self updateStatus:@"Private SimulatorKit display attached"];
        DFLog(@"Activated SimulatorKit private display attach for screen %u", self->_activeScreenID);

        __weak typeof(self) weakSelf = self;
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.15 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            __strong typeof(weakSelf) strongSelf = weakSelf;
            if (strongSelf == nil) {
                return;
            }

            id chromeView = strongSelf->_displayView != nil
                ? DFGetObjectIvar(strongSelf->_displayView, "chromeView")
                : nil;
            if (chromeView != nil) {
                DFSetBoolIvar(chromeView, "isEnabled", YES);
            }

            DFLogChromeRuntimeState(strongSelf->_displayView);
            NSArray<DFPrivateSimulatorChromeButton *> *buttons = [strongSelf availableChromeButtons];
            NSMutableArray<NSString *> *summaries = [NSMutableArray arrayWithCapacity:buttons.count];
            for (DFPrivateSimulatorChromeButton *button in buttons) {
                [summaries addObject:button.summary];
            }
            DFLog(@"Post-attach SimulatorKit chrome buttons: %@", summaries);
        });
    });
}

- (nullable CVPixelBufferRef)copyPixelBuffer {
    __block CVPixelBufferRef pixelBuffer = nil;
    dispatch_block_t work = ^{
        if (self->_latestPixelBuffer != nil) {
            pixelBuffer = CVPixelBufferRetain(self->_latestPixelBuffer);
        }
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    return pixelBuffer;
}

- (NSArray<DFPrivateSimulatorChromeButton *> *)availableChromeButtons {
    __block NSArray<DFPrivateSimulatorChromeButton *> *buttons = @[];

    dispatch_block_t work = ^{
        NSMutableArray<DFPrivateSimulatorChromeButton *> *mappedButtons = [NSMutableArray array];
        NSMutableSet<NSString *> *seenIdentifiers = [NSMutableSet set];
        for (id chromeInput in DFChromeInputsForDisplayView(self->_displayView)) {
            NSString *identifier = DFChromeInputIdentifier(chromeInput);
            if (identifier.length == 0 || [seenIdentifiers containsObject:identifier]) {
                continue;
            }
            [seenIdentifiers addObject:identifier];
            id button = DFChromeInputButton(chromeInput);
            NSString *title = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "title")) ?: @"";
            NSString *toolTip = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "toolTip")) ?: @"";
            NSString *accessibilityLabel = DFTrimmedString(DFOptionalStringFromObjectSelector(button, "accessibilityLabel")) ?: @"";
            NSString *summary = DFChromeInputSummary(chromeInput);

            [mappedButtons addObject:[[DFPrivateSimulatorChromeButton alloc] initWithIdentifier:identifier
                                                                                          title:title
                                                                                        toolTip:toolTip
                                                                             accessibilityLabel:accessibilityLabel
                                                                                        summary:summary]];
        }

        for (id control in DFButtonLikeControlsForDisplayView(self->_displayView)) {
            NSString *identifier = DFButtonLikeControlIdentifier(control);
            if (identifier.length == 0 || [seenIdentifiers containsObject:identifier]) {
                continue;
            }
            [seenIdentifiers addObject:identifier];

            NSString *title = DFButtonLikeControlTitle(control) ?: @"";
            NSString *toolTip = DFButtonLikeControlToolTip(control) ?: @"";
            NSString *accessibilityLabel = DFButtonLikeControlAccessibilityLabel(control) ?: @"";
            NSString *summary = DFButtonLikeControlSummary(control);

            [mappedButtons addObject:[[DFPrivateSimulatorChromeButton alloc] initWithIdentifier:identifier
                                                                                          title:title
                                                                                        toolTip:toolTip
                                                                             accessibilityLabel:accessibilityLabel
                                                                                        summary:summary]];
        }

        buttons = [mappedButtons copy];
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    return buttons;
}

- (BOOL)pressChromeButtonWithIdentifier:(NSString *)identifier error:(NSError * _Nullable __autoreleasing *)error {
    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        id chromeInput = DFFindChromeInputForIdentifier(self->_displayView, identifier);
        if (chromeInput != nil) {
            NSError *chromeError = nil;
            if (!DFTriggerChromeInput(chromeInput, &chromeError)) {
                dispatchError = chromeError ?: DFMakeError(
                    DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                    [NSString stringWithFormat:@"SimulatorKit rejected chrome button %@.", identifier]
                );
                return;
            }

            DFLog(@"Triggered private SimulatorKit chrome input: %@", DFChromeInputSummary(chromeInput));
            success = YES;
            return;
        }

        id control = DFFindButtonLikeControlForIdentifier(self->_displayView, identifier);
        if (control == nil) {
            dispatchError = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                [NSString stringWithFormat:@"SimulatorKit did not expose a chrome control for identifier %@.", identifier]
            );
            return;
        }

        NSError *controlError = nil;
        if (!DFTriggerButtonLikeControl(control, &controlError)) {
            dispatchError = controlError ?: DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                [NSString stringWithFormat:@"SimulatorKit rejected chrome control %@.", identifier]
            );
            return;
        }

        DFLog(@"Triggered private SimulatorKit button-like control: %@", DFButtonLikeControlSummary(control));
        success = YES;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError;
    }

    return success;
}

- (BOOL)pressHomeButton:(NSError * _Nullable __autoreleasing *)error {
    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        // Prefer Simulator.app's own chrome affordance when available (iPhone 8 / SE
        // with the Touch ID bezel). This goes through the same code path as clicking
        // the modeled Home button in the Simulator window — guaranteed not to reboot.
        NSArray *chromeInputs = DFChromeInputsForDisplayView(self->_displayView);
        for (id chromeInput in chromeInputs) {
            if (!DFChromeInputMatchesHome(chromeInput)) {
                continue;
            }

            NSError *chromeError = nil;
            if (DFTriggerChromeInput(chromeInput, &chromeError)) {
                DFLog(@"Triggered private SimulatorKit Home chrome input: %@", DFChromeInputSummary(chromeInput));
                success = YES;
                return;
            }

            DFLog(@"Failed to trigger Home chrome input %@: %@", DFChromeInputSummary(chromeInput), chromeError.localizedDescription);
        }

        NSArray *buttonLikeControls = DFButtonLikeControlsForDisplayView(self->_displayView);
        for (id control in buttonLikeControls) {
            NSString *summary = DFButtonLikeControlSummary(control);
            if (![summary.lowercaseString containsString:@"home"]) {
                continue;
            }

            NSError *controlError = nil;
            if (DFTriggerButtonLikeControl(control, &controlError)) {
                DFLog(@"Triggered private SimulatorKit Home button-like control: %@", summary);
                success = YES;
                return;
            }

            DFLog(@"Failed to trigger Home button-like control %@: %@", summary, controlError.localizedDescription);
        }

        // Fallback: drive the private Indigo HID client directly. Required for
        // Face ID devices where Simulator.app doesn't render a chrome Home button.
        NSError *hidError = nil;
        if (DFPressHomeViaHIDClient(self->_hidClient, &hidError)) {
            success = YES;
            return;
        }
        DFLog(@"HID Home path failed after chrome fallback: %@", hidError.localizedDescription ?: @"unknown error");

        if (chromeInputs.count > 0) {
            NSMutableArray<NSString *> *summaries = [NSMutableArray arrayWithCapacity:chromeInputs.count];
            for (id chromeInput in chromeInputs) {
                [summaries addObject:DFChromeInputSummary(chromeInput)];
            }
            DFLog(@"Available SimulatorKit chrome inputs: %@", summaries);
        }

        if (buttonLikeControls.count > 0) {
            NSMutableArray<NSString *> *summaries = [NSMutableArray arrayWithCapacity:buttonLikeControls.count];
            for (id control in buttonLikeControls) {
                [summaries addObject:DFButtonLikeControlSummary(control)];
            }
            DFLog(@"Available SimulatorKit button-like controls: %@", summaries);
        }

        dispatchError = hidError ?: DFMakeError(
            DFPrivateSimulatorErrorCodeTouchDispatchFailed,
            @"No Home path succeeded (no chrome Home control, HID strategies rejected)."
        );
        return;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError;
    }

    return success;
}

- (BOOL)rotateByDegrees:(double)deltaDegrees error:(NSError * _Nullable __autoreleasing *)error {
    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        // Resolve `<View>.deviceRotation` getter/setter ObjC-thunks by stable
        // mangled prefix; the Foundation.Measurement type tail in the middle is
        // what shifts across Xcodes.
        static const char *displayViewPrefix    = "$s12SimulatorKit14SimDisplayViewC14deviceRotation";
        static const char *chromePrefix         = "$s12SimulatorKit20SimDisplayChromeViewC14deviceRotation";
        static const char *chromeRenderPrefix   = "$s12SimulatorKit26SimDisplayChromeRenderViewC14deviceRotation";
        static const char *digitizerPrefix      = "$s12SimulatorKit21SimDigitizerInputViewC14deviceRotation";
        static const char *getterSuffix         = "vgTj";

        // Seed the next target rotation from whatever SimulatorKit exposes (if anything).
        // If every getter is missing on this Xcode/macOS build, fall back to our locally
        // tracked ivar so the device still rotates even when we can't read SimulatorKit's
        // internal rotation state.
        __block DFUnitAngleMeasurement measurement = { [NSUnitAngle degrees], self->_deviceRotationDegrees };
        __block BOOL readFromSimulatorKit = NO;
        __block BOOL viewsUpdated = NO;

        DFRunOnMainSync(^{
            DFConfigureDisplayGeometry(self->_displayView, self->_displayPixelSize);

            id chromeView = self->_displayView != nil
                ? object_getIvar(self->_displayView, DFGetIvar(self->_displayView, "chromeView"))
                : nil;
            id chromeRenderView = chromeView != nil
                ? object_getIvar(chromeView, DFGetIvar(chromeView, "_renderView"))
                : nil;

            DFUnitAngleMeasurement readMeasurement = { [NSUnitAngle degrees], 0 };
            if (DFCallSwiftUnitAngleMeasurementGetterByPattern(self->_displayView,        displayViewPrefix, getterSuffix, "SimDisplayView.deviceRotation.getter",        &readMeasurement) ||
                DFCallSwiftUnitAngleMeasurementGetterByPattern(chromeView,                chromePrefix,      getterSuffix, "SimDisplayChromeView.deviceRotation.getter",  &readMeasurement) ||
                DFCallSwiftUnitAngleMeasurementGetterByPattern(self->_digitizerInputView, digitizerPrefix,   getterSuffix, "SimDigitizerInputView.deviceRotation.getter", &readMeasurement)) {
                readFromSimulatorKit = YES;
                measurement = readMeasurement;
                if (measurement.unit == nil) {
                    measurement.unit = [NSUnitAngle degrees];
                }
            }

            measurement.value = DFNormalizedDegrees(measurement.value + deltaDegrees);
            self->_deviceRotationDegrees = measurement.value;

            if (readFromSimulatorKit) {
                viewsUpdated |= DFSetDisplayRotationMeasurement(self->_displayView,        measurement, displayViewPrefix,  "SimDisplayView.deviceRotation.setter");
                viewsUpdated |= DFSetDisplayRotationMeasurement(chromeView,                measurement, chromePrefix,       "SimDisplayChromeView.deviceRotation.setter");
                viewsUpdated |= DFSetDisplayRotationMeasurement(chromeRenderView,          measurement, chromeRenderPrefix, "SimDisplayChromeRenderView.deviceRotation.setter");
                viewsUpdated |= DFSetDisplayRotationMeasurement(self->_digitizerInputView, measurement, digitizerPrefix,    "SimDigitizerInputView.deviceRotation.setter");
            }
        });

        NSInteger orientationValue = DFOrientationEquivalentValueForMeasurement(measurement);
        BOOL propagatedOrientation = DFSendDeviceOrientationEvent(self->_device, orientationValue);

        if (!propagatedOrientation) {
            // Try the screen and adapter targets too — on newer SimulatorKit builds
            // the orientation selector may live there rather than on SimDevice.
            propagatedOrientation = DFTrySendIntegerSelectors(self->_activeScreen,      @"activeScreen",      orientationValue) ||
                                    DFTrySendIntegerSelectors(self->_rawScreen,         @"rawScreen",         orientationValue) ||
                                    DFTrySendIntegerSelectors(self->_screenAdapter,     @"screenAdapter",     orientationValue) ||
                                    DFTrySendIntegerSelectors(self->_screenAdapterHost, @"screenAdapterHost", orientationValue) ||
                                    DFTrySendIntegerSelectors(self->_serviceContext,    @"serviceContext",    orientationValue);
        }

        // On iOS 26 the GSEvent is delivered to backboardd but no longer updates
        // UIDevice.current.orientation, so UIKit autorotation never fires. Post a
        // Darwin notification as a side-channel that apps can observe to force a
        // geometry update. Harmless on older iOS where autorotation still works.
        NSString *notificationName = DFRotationNotificationNameForOrientation(orientationValue);
        if (notificationName != nil) {
            DFPostSimDeviceDarwinNotification(self->_device, notificationName);
        }

        if (!propagatedOrientation) {
            DFLogRuntimeShape(self->_device,              @"device");
            DFLogRuntimeShape(self->_activeScreen,        @"activeScreen");
            DFLogRuntimeShape(self->_rawScreen,           @"rawScreen");
            DFLogRuntimeShape(self->_screenAdapter,       @"screenAdapter");
            DFLogRuntimeShape(self->_screenAdapterHost,   @"screenAdapterHost");
            DFLogRuntimeShape(self->_serviceContext,      @"serviceContext");
        }

        if (!propagatedOrientation && !viewsUpdated) {
            dispatchError = DFMakeError(
                DFPrivateSimulatorErrorCodeDisplayAttachFailed,
                @"Failed to rotate: SimulatorKit view rotation unavailable and every orientation selector rejected."
            );
            return;
        }

        DFLog(@"Rotated to %.0f° (orientation=%ld) viewsUpdated=%d orientationSent=%d",
              measurement.value, (long)orientationValue, viewsUpdated, propagatedOrientation);
        success = YES;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError;
    }

    return success;
}

- (BOOL)pressHardwareButtonNamed:(NSString *)buttonName
                       durationMs:(NSUInteger)durationMs
                            error:(NSError * _Nullable __autoreleasing *)error {
    NSString *normalizedName = buttonName.lowercaseString ?: @"";
    if ([normalizedName isEqualToString:@"home"]) {
        if (durationMs > 0) {
            [NSThread sleepForTimeInterval:(NSTimeInterval)durationMs / 1000.0];
        }
        return [self pressHomeButton:error];
    }

    static NSDictionary<NSString *, NSNumber *> *buttonCodes;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        buttonCodes = @{
            // FBSimulatorHIDButton raw values used by fbsimctl/idb/AXe.
            @"apple-pay": @1,
            @"lock": @3,
            @"power": @3,
            @"side-button": @4,
            @"side": @4,
            @"siri": @5,
        };
    });

    NSNumber *buttonCode = buttonCodes[normalizedName];
    if (buttonCode == nil) {
        if (error != NULL) {
            *error = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                [NSString stringWithFormat:@"Unsupported hardware button `%@`.", buttonName ?: @""]
            );
        }
        return NO;
    }

    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        if (self->_hidClient == nil) {
            dispatchError = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not provide a headless HID client for hardware buttons."
            );
            return;
        }

        uint32_t code = buttonCode.unsignedIntValue;
        uint32_t targets[] = { DFIndigoTouchTarget, 0x2 };
        for (NSUInteger targetIndex = 0; targetIndex < sizeof(targets) / sizeof(targets[0]); targetIndex++) {
            NSError *downError = nil;
            IndigoHIDMessage *down = DFCreateButtonMessage(code, DFButtonDirectionDown, targets[targetIndex], &downError);
            if (down == NULL) {
                dispatchError = downError;
                continue;
            }
            if (!DFSendHIDMessage(self->_hidClient, down, YES, &downError)) {
                dispatchError = downError;
                continue;
            }

            if (durationMs > 0) {
                [NSThread sleepForTimeInterval:(NSTimeInterval)durationMs / 1000.0];
            }

            NSError *upError = nil;
            IndigoHIDMessage *up = DFCreateButtonMessage(code, DFButtonDirectionUp, targets[targetIndex], &upError);
            if (up == NULL) {
                dispatchError = upError;
                continue;
            }
            if (!DFSendHIDMessage(self->_hidClient, up, YES, &upError)) {
                dispatchError = upError;
                continue;
            }

            DFLog(@"Sending hardware button `%@` code=%u target=0x%x durationMs=%lu", buttonName, code, targets[targetIndex], (unsigned long)durationMs);
            success = YES;
            return;
        }
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError ?: DFMakeError(
            DFPrivateSimulatorErrorCodeTouchDispatchFailed,
            [NSString stringWithFormat:@"SimulatorKit rejected hardware button `%@`.", buttonName ?: @""]
        );
    }

    return success;
}

- (BOOL)rotateRight:(NSError * _Nullable __autoreleasing *)error {
    return [self rotateByDegrees:90.0 error:error];
}

- (BOOL)rotateLeft:(NSError * _Nullable __autoreleasing *)error {
    return [self rotateByDegrees:-90.0 error:error];
}

- (void)disconnect {
    dispatch_block_t work = ^{
        if (self->_screenAdapter != nil && self->_screenAdapterCallbackUUID != nil) {
            ((void(*)(id, SEL, id))objc_msgSend)(self->_screenAdapter, sel_registerName("unregisterScreenAdapterCallbacksWithUUID:"), self->_screenAdapterCallbackUUID);
        }

        NSDictionary<NSNumber *, id> *screens = DFReadAdapterScreens(self->_screenAdapterHost);
        id rawScreen = screens[@(self->_activeScreenID)];
        if (rawScreen != nil && self->_screenCallbackUUID != nil && [rawScreen respondsToSelector:sel_registerName("unregisterScreenCallbacksWithUUID:")]) {
            ((void(*)(id, SEL, id))objc_msgSend)(rawScreen, sel_registerName("unregisterScreenCallbacksWithUUID:"), self->_screenCallbackUUID);
        }

        if (self->_latestPixelBuffer != nil) {
            CVPixelBufferRelease(self->_latestPixelBuffer);
            self->_latestPixelBuffer = nil;
        }

        [self->_headlessHostWindow orderOut:nil];
        [self->_headlessHostWindow close];
        self->_headlessHostWindow = nil;
        self->_headlessHostView = nil;

        [self updateStatus:@"Disconnected"];
        [self.displayView removeFromSuperview];
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }
}

- (BOOL)isDisplayReady {
    __block BOOL ready = NO;
    dispatch_block_t work = ^{
        ready = self->_latestPixelBuffer != nil;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    return ready;
}

- (NSString *)displayStatus {
    __block NSString *status = @"Starting private CoreSimulator attach";
    dispatch_block_t work = ^{
        status = self->_displayStatusValue ?: @"Starting private CoreSimulator attach";
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    return status;
}

- (BOOL)sendTouchAtNormalizedX:(double)normalizedX
                   normalizedY:(double)normalizedY
                         phase:(DFPrivateSimulatorTouchPhase)phase
                         error:(NSError * _Nullable __autoreleasing *)error {
    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        if (self->_hidClient == nil) {
            dispatchError = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not provide a headless HID client for this simulator."
            );
            return;
        }

        CGFloat clampedX = (CGFloat)fmax(0.0, fmin(1.0, normalizedX));
        CGFloat clampedY = (CGFloat)fmax(0.0, fmin(1.0, normalizedY));
        CGSize displaySize = self->_displayPixelSize;
        if (displaySize.width < 1.0 || displaySize.height < 1.0) {
            displaySize = CGSizeMake(1.0, 1.0);
        }
        CGPoint point = CGPointMake(
            clampedX * fmax(displaySize.width - 1.0, 1.0),
            clampedY * fmax(displaySize.height - 1.0, 1.0)
        );

        NSString *phaseLabel = @"moved";
        switch (phase) {
        case DFPrivateSimulatorTouchPhaseBegan:
            phaseLabel = @"began";
            break;
        case DFPrivateSimulatorTouchPhaseMoved:
            phaseLabel = @"moved";
            break;
        case DFPrivateSimulatorTouchPhaseEnded:
        case DFPrivateSimulatorTouchPhaseCancelled:
            phaseLabel = phase == DFPrivateSimulatorTouchPhaseEnded ? @"ended" : @"cancelled";
            break;
        }
        BOOL touchDown = phase == DFPrivateSimulatorTouchPhaseBegan || phase == DFPrivateSimulatorTouchPhaseMoved;
        DFIndigoMessage *message = DFCreateIndigoTouchMessage(CGPointMake(clampedX, clampedY), displaySize, touchDown, &dispatchError);
        if (message == NULL) {
            return;
        }

        NSError *messageError = nil;
        if (!DFSendHIDMessage(self->_hidClient, (IndigoHIDMessage *)message, YES, &messageError)) {
            dispatchError = messageError ?: DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit rejected the Indigo HID touch packet."
            );
            return;
        }

        if (phase != DFPrivateSimulatorTouchPhaseMoved) {
            DFLog(@"Sending %@ Indigo HID touch at pixel (%.1f, %.1f) ratio (%.4f, %.4f) within %.0fx%.0f", phaseLabel, point.x, point.y, clampedX, clampedY, displaySize.width, displaySize.height);
        }

        self->_lastTouchPoint = point;
        self->_hasLastTouchPoint = YES;
        if (phase == DFPrivateSimulatorTouchPhaseEnded || phase == DFPrivateSimulatorTouchPhaseCancelled) {
            self->_lastTouchPoint = CGPointZero;
            self->_hasLastTouchPoint = NO;
        }

        success = YES;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError;
    }

    return success;
}

- (BOOL)sendMultiTouchAtNormalizedX1:(double)normalizedX1
                         normalizedY1:(double)normalizedY1
                         normalizedX2:(double)normalizedX2
                         normalizedY2:(double)normalizedY2
                               phase:(DFPrivateSimulatorTouchPhase)phase
                               error:(NSError * _Nullable __autoreleasing *)error {
    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        if (self->_hidClient == nil) {
            dispatchError = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not provide a headless HID client for this simulator."
            );
            return;
        }

        CGFloat x1 = (CGFloat)fmax(0.0, fmin(1.0, normalizedX1));
        CGFloat y1 = (CGFloat)fmax(0.0, fmin(1.0, normalizedY1));
        CGFloat x2 = (CGFloat)fmax(0.0, fmin(1.0, normalizedX2));
        CGFloat y2 = (CGFloat)fmax(0.0, fmin(1.0, normalizedY2));
        CGSize displaySize = self->_displayPixelSize;
        if (displaySize.width < 1.0 || displaySize.height < 1.0) {
            displaySize = CGSizeMake(1.0, 1.0);
        }

        NSString *phaseLabel = @"moved";
        switch (phase) {
        case DFPrivateSimulatorTouchPhaseBegan:
            phaseLabel = @"began";
            break;
        case DFPrivateSimulatorTouchPhaseMoved:
            phaseLabel = @"moved";
            break;
        case DFPrivateSimulatorTouchPhaseEnded:
        case DFPrivateSimulatorTouchPhaseCancelled:
            phaseLabel = phase == DFPrivateSimulatorTouchPhaseEnded ? @"ended" : @"cancelled";
            break;
        }

        BOOL touchDown = phase == DFPrivateSimulatorTouchPhaseBegan || phase == DFPrivateSimulatorTouchPhaseMoved;
        DFIndigoMessage *message = DFCreateIndigoMultiTouchMessage(CGPointMake(x1, y1), CGPointMake(x2, y2), displaySize, touchDown, &dispatchError);
        if (message == NULL) {
            return;
        }

        NSError *messageError = nil;
        if (!DFSendHIDMessage(self->_hidClient, (IndigoHIDMessage *)message, YES, &messageError)) {
            dispatchError = messageError ?: DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit rejected the Indigo HID multi-touch packet."
            );
            return;
        }

        if (phase != DFPrivateSimulatorTouchPhaseMoved) {
            DFLog(@"Sending %@ Indigo HID multi-touch p1=(%.4f, %.4f) p2=(%.4f, %.4f) within %.0fx%.0f", phaseLabel, x1, y1, x2, y2, displaySize.width, displaySize.height);
        }
        success = YES;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError;
    }

    return success;
}

- (BOOL)sendKeyCode:(uint16_t)keyCode
          modifiers:(NSUInteger)modifiers
              error:(NSError * _Nullable __autoreleasing *)error {
    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        if (self->_hidClient == nil) {
            dispatchError = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not provide a headless HID client for keyboard input."
            );
            return;
        }

        static const struct {
            NSUInteger mask;
            uint16_t keyCode;
        } modifierMap[] = {
            // IndigoHIDMessageForKeyboardArbitrary expects USB HID keyboard
            // usages. These are left-side modifier usages from the Keyboard page.
            { DFKeyboardModifierCapsLock, 57 },
            { DFKeyboardModifierControl, 224 },
            { DFKeyboardModifierShift, 225 },
            { DFKeyboardModifierOption, 226 },
            { DFKeyboardModifierCommand, 227 },
        };

        NSMutableArray<NSNumber *> *modifierKeyCodes = [NSMutableArray array];
        for (NSUInteger index = 0; index < sizeof(modifierMap) / sizeof(modifierMap[0]); index++) {
            if ((modifiers & modifierMap[index].mask) != 0) {
                [modifierKeyCodes addObject:@(modifierMap[index].keyCode)];
            }
        }

        NSError *messageError = nil;
        for (NSNumber *modifierKeyCode in modifierKeyCodes) {
            if (!DFSendSingleKeyboardEvent(self->_hidClient, modifierKeyCode.unsignedShortValue, YES, &messageError)) {
                dispatchError = messageError;
                return;
            }
        }

        if (!DFSendSingleKeyboardEvent(self->_hidClient, keyCode, YES, &messageError)) {
            dispatchError = messageError;
            return;
        }

        if (!DFSendSingleKeyboardEvent(self->_hidClient, keyCode, NO, &messageError)) {
            dispatchError = messageError;
            return;
        }

        for (NSNumber *modifierKeyCode in [modifierKeyCodes reverseObjectEnumerator]) {
            if (!DFSendSingleKeyboardEvent(self->_hidClient, modifierKeyCode.unsignedShortValue, NO, &messageError)) {
                dispatchError = messageError;
                return;
            }
        }

        DFLog(@"Sending keyboard HID keyCode %u with modifiers 0x%lx", keyCode, (unsigned long)modifiers);
        success = YES;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError;
    }

    return success;
}

- (BOOL)sendKeyCode:(uint16_t)keyCode
                down:(BOOL)down
               error:(NSError * _Nullable __autoreleasing *)error {
    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        if (self->_hidClient == nil) {
            dispatchError = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not provide a headless HID client for keyboard input."
            );
            return;
        }

        NSError *messageError = nil;
        if (!DFSendSingleKeyboardEvent(self->_hidClient, keyCode, down, &messageError)) {
            dispatchError = messageError;
            return;
        }

        DFLog(@"Sending keyboard HID keyCode %u %@", keyCode, down ? @"down" : @"up");
        success = YES;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError;
    }

    return success;
}

- (BOOL)sendKeyEvent:(NSEvent *)event
               error:(NSError * _Nullable __autoreleasing *)error {
    __block BOOL success = NO;
    __block NSError *dispatchError = nil;

    dispatch_block_t work = ^{
        if (self->_hidClient == nil) {
            dispatchError = DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit did not provide a headless HID client for keyboard input."
            );
            return;
        }

        NSError *messageError = nil;
        IndigoHIDMessage *message = DFCreateKeyboardMessageFromEvent(event, &messageError);
        if (message == NULL) {
            dispatchError = messageError;
            return;
        }

        if (!DFSendHIDMessage(self->_hidClient, message, NO, &messageError)) {
            dispatchError = messageError ?: DFMakeError(
                DFPrivateSimulatorErrorCodeTouchDispatchFailed,
                @"SimulatorKit rejected the NSEvent keyboard HID packet."
            );
            return;
        }

        DFLog(@"Sending NSEvent keyboard HID type=%ld keyCode=%hu modifiers=0x%llx", (long)event.type, event.keyCode, event.modifierFlags);
        success = YES;
    };

    if (dispatch_get_specific(DFPrivateSimulatorCallbackQueueKey) != NULL) {
        work();
    } else {
        dispatch_sync(_callbackQueue, work);
    }

    if (!success && error != NULL) {
        *error = dispatchError;
    }

    return success;
}

@end
