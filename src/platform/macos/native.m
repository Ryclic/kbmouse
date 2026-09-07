#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <IOKit/hid/IOHIDManager.h>
#import <IOKit/hidsystem/IOHIDLib.h>
#import <IOKit/hidsystem/IOHIDParameter.h>
#import <IOKit/hidsystem/IOHIDShared.h>

// Quartz uses top-left screen points; AppKit uses bottom-left screen points.
// All windows are accessed on the main thread. The tap stays on the Rust runtime thread.
typedef struct { int32_t x, y; uint32_t width, height; } KBRect;
typedef struct { uint16_t code; bool down; } KBKey;
typedef struct {
    CFMachPortRef tap;
    CFRunLoopSourceRef source;
    uint16_t leader;
    IOHIDManagerRef capsManager;
    CFMutableSetRef capsDevicesDown;
    io_connect_t hidSystem;
    bool originalCapsLock;
    bool active;
    bool swallowed[128];
    bool buttons[3];
    KBKey queue[256];
    unsigned read, write;
    bool overflow;
} KBState;

static NSPanel *panel;
static NSImage *drawing;
static NSColor *background, *grid, *textColor, *accent;
static CGFloat fontSize, opacity;
static bool contrast, glow, crisp;
static KBRect sceneBounds;

static void onMain(dispatch_block_t block) {
    if ([NSThread isMainThread]) block(); else dispatch_async(dispatch_get_main_queue(), block);
}
void kb_init(bool oneShot) {
    [NSApplication sharedApplication];
    if (oneShot) {
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        [NSApp finishLaunching];
    }
}
static NSColor *color(const char *hex) {
    unsigned value = 0xffffff;
    if (hex && hex[0] == '#') sscanf(hex + 1, "%x", &value);
    return [NSColor colorWithSRGBRed:((value >> 16) & 255) / 255.0
        green:((value >> 8) & 255) / 255.0 blue:(value & 255) / 255.0 alpha:1];
}
static const uint16_t capsCode = 57;
static const char *lastError = "macOS keyboard capture failed";
const char *kb_error(void) { return lastError; }

static void enqueue(KBState *s, uint16_t code, bool down) {
    unsigned next = (s->write + 1) % 256;
    if (next == s->read) s->overflow = true;
    else { s->queue[s->write] = (KBKey){code, down}; s->write = next; }
}
// Raw HID values describe physical press/release, unlike Quartz Caps Lock flags.
// Track each keyboard so repeats and unplugging a held keyboard cannot stick the leader.
static void capsTransition(KBState *s, const void *device, bool down) {
    bool wasDown = CFSetGetCount(s->capsDevicesDown) != 0;
    if (down) CFSetAddValue(s->capsDevicesDown, device);
    else CFSetRemoveValue(s->capsDevicesDown, device);
    bool isDown = CFSetGetCount(s->capsDevicesDown) != 0;
    if (wasDown == isDown) return;
    if (isDown) s->active = true;
    enqueue(s, capsCode, isDown);
}
static void capsInput(void *context, IOReturn result, void *sender, IOHIDValueRef value) {
    (void)sender;
    KBState *s = context;
    if (result != kIOReturnSuccess) { s->overflow = true; return; }
    IOHIDElementRef element = IOHIDValueGetElement(value);
    if (IOHIDElementGetUsagePage(element) != kHIDPage_KeyboardOrKeypad ||
        IOHIDElementGetUsage(element) != kHIDUsage_KeyboardCapsLock) return;
    capsTransition(s, IOHIDElementGetDevice(element), IOHIDValueGetIntegerValue(value) != 0);
}
static void capsRemoved(void *context, IOReturn result, void *sender, IOHIDDeviceRef device) {
    (void)result; (void)sender;
    capsTransition(context, device, false);
}
static void stopCaps(KBState *s) {
    if (s->capsManager) {
        IOHIDManagerUnscheduleFromRunLoop(s->capsManager, CFRunLoopGetCurrent(), kCFRunLoopCommonModes);
        IOHIDManagerClose(s->capsManager, kIOHIDOptionsTypeNone);
        CFRelease(s->capsManager); s->capsManager = NULL;
    }
    if (s->capsDevicesDown) { CFRelease(s->capsDevicesDown); s->capsDevicesDown = NULL; }
    if (s->hidSystem) {
        IOHIDSetModifierLockState(s->hidSystem, kIOHIDCapsLockState, s->originalCapsLock);
        IOServiceClose(s->hidSystem); s->hidSystem = 0;
    }
}
static bool startCaps(KBState *s) {
    @autoreleasepool {
        if (IOHIDCheckAccess(kIOHIDRequestTypeListenEvent) != kIOHIDAccessTypeGranted) {
            lastError = "Caps Lock requires Input Monitoring. Enable kbmouse (or its launching terminal) in System Settings > Privacy & Security > Input Monitoring, then restart kbmouse";
            return false;
        }
        lastError = "could not open physical keyboards for Caps Lock; check Input Monitoring permission and restart kbmouse";
        s->capsManager = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
        if (!s->capsManager) return false;
        NSDictionary *keyboard = @{@kIOHIDDeviceUsagePageKey:@(kHIDPage_GenericDesktop),
            @kIOHIDDeviceUsageKey:@(kHIDUsage_GD_Keyboard)};
        IOHIDManagerSetDeviceMatching(s->capsManager, (__bridge CFDictionaryRef)keyboard);
        NSDictionary *caps = @{@kIOHIDElementUsagePageKey:@(kHIDPage_KeyboardOrKeypad),
            @kIOHIDElementUsageKey:@(kHIDUsage_KeyboardCapsLock)};
        IOHIDManagerSetInputValueMatching(s->capsManager, (__bridge CFDictionaryRef)caps);
        // Observe only: do not seize keyboards or install a system-wide remapping.
        if (IOHIDManagerOpen(s->capsManager, kIOHIDOptionsTypeNone) != kIOReturnSuccess) {
            stopCaps(s); return false;
        }
        io_service_t service = IOServiceGetMatchingService(MACH_PORT_NULL, IOServiceMatching("IOHIDSystem"));
        IOReturn result = service ? IOServiceOpen(service, mach_task_self(), kIOHIDParamConnectType, &s->hidSystem) : kIOReturnNotFound;
        if (service) IOObjectRelease(service);
        if (result != kIOReturnSuccess) {
            lastError = "could not access the macOS Caps Lock state";
            stopCaps(s); return false;
        }
        if (IOHIDGetModifierLockState(s->hidSystem, kIOHIDCapsLockState, &s->originalCapsLock) != kIOReturnSuccess) {
            IOServiceClose(s->hidSystem); s->hidSystem = 0;
            lastError = "could not read the macOS Caps Lock state";
            stopCaps(s); return false;
        }
        s->capsDevicesDown = CFSetCreateMutable(kCFAllocatorDefault, 0, &kCFTypeSetCallBacks);
        IOHIDManagerRegisterInputValueCallback(s->capsManager, capsInput, s);
        IOHIDManagerRegisterDeviceRemovalCallback(s->capsManager, capsRemoved, s);
        IOHIDManagerScheduleWithRunLoop(s->capsManager, CFRunLoopGetCurrent(), kCFRunLoopCommonModes);
        return true;
    }
}
static CGEventFlags capsFlags(KBState *s, CGEventFlags flags) {
    return s->originalCapsLock ? flags | kCGEventFlagMaskAlphaShift : flags & ~kCGEventFlagMaskAlphaShift;
}
static CGEventRef tapCallback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *info) {
    (void)proxy;
    KBState *s = info;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        // Fail closed at the runtime level: stale held keys must not keep moving/dragging.
        s->overflow = true;
        return event;
    }
    unsigned code = (unsigned)CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
    if (s->leader == capsCode) {
        // Suppress the logical toggle separately from the physical HID events.
        // Restoring the kernel lock state also restores the LED; retain its initial value.
        CGEventFlags flags = CGEventGetFlags(event);
        if ((flags & kCGEventFlagMaskAlphaShift) != (capsFlags(s, flags) & kCGEventFlagMaskAlphaShift)) {
            if (IOHIDSetModifierLockState(s->hidSystem, kIOHIDCapsLockState, s->originalCapsLock) != kIOReturnSuccess)
                s->overflow = true;
            CGEventSetFlags(event, capsFlags(s, flags));
        }
        if (code == capsCode) return NULL;
    }
    if (type == kCGEventFlagsChanged) return event;
    bool down = type == kCGEventKeyDown;
    bool consume = code == s->leader || s->active || (code < 128 && s->swallowed[code]);
    if (!consume || (!down && code != s->leader && (code >= 128 || !s->swallowed[code]))) return event;
    if (code < 128) s->swallowed[code] = down;
    if (code == s->leader && down) s->active = true;
    enqueue(s, code, down);
    return NULL;
}
void *kb_create(uint16_t leader) {
    lastError = "macOS keyboard capture requires Accessibility. Enable kbmouse (or its launching terminal) in System Settings > Privacy & Security > Accessibility, then restart kbmouse";
    if (!AXIsProcessTrusted()) return NULL;
    KBState *s = calloc(1, sizeof(KBState));
    if (!s) return NULL;
    s->leader = leader;
    lastError = "could not create macOS keyboard capture; check Accessibility and Input Monitoring permissions, then restart kbmouse";
    s->tap = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionDefault,
        (1ULL << kCGEventKeyDown) | (1ULL << kCGEventKeyUp) | (1ULL << kCGEventFlagsChanged), tapCallback, s);
    if (!s->tap) { free(s); return NULL; }
    if (leader == capsCode && !startCaps(s)) { CFRelease(s->tap); free(s); return NULL; }
    s->source = CFMachPortCreateRunLoopSource(NULL, s->tap, 0);
    CFRunLoopAddSource(CFRunLoopGetCurrent(), s->source, kCFRunLoopCommonModes);
    CGEventTapEnable(s->tap, true);
    return s;
}
int kb_poll(void *state, double timeout, KBKey *key) {
    KBState *s = state;
    @autoreleasepool {
        if ([NSThread isMainThread] && s->read == s->write) {
            NSEvent *event = [NSApp nextEventMatchingMask:NSEventMaskAny
                untilDate:[NSDate dateWithTimeIntervalSinceNow:timeout]
                inMode:NSDefaultRunLoopMode dequeue:YES];
            if (event) [NSApp sendEvent:event];
        } else if (s->read == s->write) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, timeout, true);
        }
    }
    if (s->overflow) return -1;
    if (s->read == s->write) return 0;
    *key = s->queue[s->read]; s->read = (s->read + 1) % 256;
    return 1;
}
void kb_active(void *state, bool active) { ((KBState *)state)->active = active; }
bool kb_leader(void *state, uint16_t leader) {
    KBState *s = state;
    if (s->leader == leader) return true;
    if (leader == capsCode && !startCaps(s)) return false;
    if (s->leader == capsCode) stopCaps(s);
    s->leader = leader;
    return true;
}
KBRect kb_bounds(bool span) {
    @autoreleasepool {
        CGDirectDisplayID displays[32]; uint32_t count = 0;
        CGGetActiveDisplayList(32, displays, &count);
        CGEventRef cursorEvent = CGEventCreate(NULL);
        CGPoint focus = CGEventGetLocation(cursorEvent);
        CFRelease(cursorEvent);
        // Prefer the focused window; use the pointer's monitor when AX is unavailable.
        NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
        if (app) {
            AXUIElementRef application = AXUIElementCreateApplication(app.processIdentifier);
            AXUIElementSetMessagingTimeout(application, 0.05);
            CFTypeRef window = NULL, position = NULL, size = NULL;
            if (AXUIElementCopyAttributeValue(application, kAXFocusedWindowAttribute, &window) == kAXErrorSuccess) {
                if (AXUIElementCopyAttributeValue((AXUIElementRef)window, kAXPositionAttribute, &position) == kAXErrorSuccess &&
                    AXUIElementCopyAttributeValue((AXUIElementRef)window, kAXSizeAttribute, &size) == kAXErrorSuccess &&
                    CFGetTypeID(position) == AXValueGetTypeID() && CFGetTypeID(size) == AXValueGetTypeID()) {
                    CGPoint p; CGSize z;
                    if (AXValueGetValue(position, kAXValueCGPointType, &p) && AXValueGetValue(size, kAXValueCGSizeType, &z))
                        focus = CGPointMake(p.x + z.width / 2, p.y + z.height / 2);
                }
            }
            if (size) CFRelease(size); if (position) CFRelease(position);
            if (window) CFRelease(window); CFRelease(application);
        }
        CGRect bounds = CGDisplayBounds(CGMainDisplayID());
        for (unsigned i = 0; i < count; i++) {
            CGRect r = CGDisplayBounds(displays[i]);
            if (span) bounds = CGRectUnion(bounds, r);
            else if (CGRectContainsPoint(r, focus)) { bounds = r; break; }
        }
        return (KBRect){bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height};
    }
}
void kb_config(const char *bg, const char *gr, const char *tx, const char *ac,
    uint32_t font, uint8_t alpha, bool hc, bool lg, bool cr) {
    @autoreleasepool {
        background = color(bg); grid = color(gr); textColor = color(tx); accent = color(ac);
        fontSize = font; opacity = MIN(alpha, 100) / 100.0; contrast = hc; glow = lg; crisp = cr;
    }
}
void kb_begin(KBRect bounds) {
    @autoreleasepool {
        sceneBounds = bounds;
        drawing = [[NSImage alloc] initWithSize:NSMakeSize(bounds.width, bounds.height)];
        [drawing lockFocusFlipped:YES];
        CGContextSetShouldAntialias(NSGraphicsContext.currentContext.CGContext, !crisp);
        [[background colorWithAlphaComponent:opacity] setFill];
        NSRectFill(NSMakeRect(0, 0, bounds.width, bounds.height));
    }
}
void kb_cell(KBRect bounds, const char *label, bool matched, bool typed) {
    @autoreleasepool {
        NSRect r = NSMakeRect(bounds.x - sceneBounds.x, bounds.y - sceneBounds.y, bounds.width, bounds.height);
        [[grid colorWithAlphaComponent:matched ? 0.85 : 0.3] setStroke];
        [[NSBezierPath bezierPathWithRect:NSInsetRect(r, 0.5, 0.5)] stroke];
        NSString *string = [NSString stringWithUTF8String:label];
        NSMutableDictionary *attrs = [@{NSFontAttributeName:[NSFont monospacedSystemFontOfSize:fontSize weight:NSFontWeightSemibold],
            NSForegroundColorAttributeName:matched ? (typed ? accent : textColor) : grid} mutableCopy];
        NSSize size = [string sizeWithAttributes:attrs];
        NSRect labelRect = NSMakeRect(NSMidX(r) - size.width / 2, NSMidY(r) - size.height / 2, size.width, size.height);
        if (contrast && matched) {
            [background setFill]; [[NSBezierPath bezierPathWithRoundedRect:NSInsetRect(labelRect, -5, -2) xRadius:3 yRadius:3] fill];
        }
        if (glow && matched) {
            NSShadow *shadow = [NSShadow new]; shadow.shadowColor = accent; shadow.shadowBlurRadius = 4;
            attrs[NSShadowAttributeName] = shadow;
        }
        [string drawInRect:labelRect withAttributes:attrs];
    }
}
void kb_end(void) {
    @autoreleasepool {
        [drawing unlockFocus];
        NSImage *image = drawing;
        KBRect bounds = sceneBounds;
        drawing = nil;
        onMain(^{
            if (!panel) {
                panel = [[NSPanel alloc] initWithContentRect:NSZeroRect
                    styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
                    backing:NSBackingStoreBuffered defer:NO];
                panel.opaque = NO; panel.backgroundColor = NSColor.clearColor;
                panel.ignoresMouseEvents = YES; panel.hasShadow = NO;
                panel.hidesOnDeactivate = NO; panel.level = NSScreenSaverWindowLevel;
                panel.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
            }
            CGFloat top = CGDisplayBounds(CGMainDisplayID()).size.height;
            [panel setFrame:NSMakeRect(bounds.x, top - bounds.y - bounds.height, bounds.width, bounds.height) display:NO];
            NSImageView *view = [[NSImageView alloc] initWithFrame:NSMakeRect(0, 0, bounds.width, bounds.height)];
            view.image = image; view.imageScaling = NSImageScaleAxesIndependently;
            panel.contentView = view;
            [panel orderFrontRegardless];
        });
    }
}
void kb_hide(void) { onMain(^{ [panel orderOut:nil]; }); }
static CGPoint pointer(void) {
    CGEventRef event = CGEventCreate(NULL);
    CGPoint p = CGEventGetLocation(event); CFRelease(event); return p;
}
void kb_move(void *state, int32_t x, int32_t y, bool relative) {
    KBState *s = state;
    CGPoint p = relative ? pointer() : CGPointZero; p.x += x; p.y += y;
    CGEventType type = s->buttons[0] ? kCGEventLeftMouseDragged :
        s->buttons[1] ? kCGEventRightMouseDragged : s->buttons[2] ? kCGEventOtherMouseDragged : kCGEventMouseMoved;
    CGMouseButton button = s->buttons[1] ? kCGMouseButtonRight : s->buttons[2] ? kCGMouseButtonCenter : kCGMouseButtonLeft;
    CGEventRef event = CGEventCreateMouseEvent(NULL, type, p, button);
    CGEventPost(kCGHIDEventTap, event); CFRelease(event);
}
void kb_button(void *state, uint32_t button, bool down) {
    KBState *s = state; s->buttons[button] = down;
    CGEventType types[3][2] = {{kCGEventLeftMouseUp,kCGEventLeftMouseDown},
        {kCGEventRightMouseUp,kCGEventRightMouseDown},{kCGEventOtherMouseUp,kCGEventOtherMouseDown}};
    CGEventRef event = CGEventCreateMouseEvent(NULL, types[button][down], pointer(), button);
    CGEventSetIntegerValueField(event, kCGMouseEventClickState, 1);
    CGEventPost(kCGHIDEventTap, event); CFRelease(event);
}
void kb_scroll(int32_t amount) {
    CGEventRef event = CGEventCreateScrollWheelEvent(NULL, kCGScrollEventUnitPixel, 1, amount);
    CGEventPost(kCGHIDEventTap, event); CFRelease(event);
}
void kb_destroy(void *state) {
    KBState *s = state;
    for (unsigned i = 0; i < 3; i++) if (s->buttons[i]) kb_button(s, i, false);
    CGEventTapEnable(s->tap, false);
    CFRunLoopRemoveSource(CFRunLoopGetCurrent(), s->source, kCFRunLoopCommonModes);
    CFRelease(s->source); CFRelease(s->tap); stopCaps(s); free(s); kb_hide();
}
